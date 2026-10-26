//! Explicit offline disposal of the pre-management alpha's group data.
use prns_groups_store::StoreIncarnation;
use prns_groups_store_native::{NativeError, NativeLimits, NativeStoreOwner, StoreNamespace};
use redb::{Database, ReadableDatabase, TableDefinition};
use serde::Deserialize;
use std::path::PathBuf;
use zeroize::Zeroizing;

const CATALOG: TableDefinition<u8, &[u8]> = TableDefinition::new("prns_group_enrollment_v1");
const ACTIONS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("group_ui_actions_v1");
const LEGACY_LIMITS: NativeLimits = NativeLimits {
    max_changes: 128,
    max_key_bytes: 256,
    max_value_bytes: 262_144,
    max_staged_bytes: 2_097_152,
    max_live_objects: 8192,
    max_live_bytes: 16_777_216,
    max_receipts: 4096,
};

#[derive(Deserialize)]
struct LegacyCatalog {
    version: u8,
    entries: Vec<LegacyEntry>,
}
#[derive(Deserialize)]
struct LegacyEntry {
    namespace: [u8; 16],
    incarnation: [u8; 24],
}

#[derive(Debug)]
enum ResetError {
    Usage,
    UnsupportedVersion,
    InvalidCatalog,
    Database(redb::Error),
    Store(NativeError),
    Json(serde_json::Error),
}
impl std::fmt::Display for ResetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage => f.write_str("Usage: reset-test-groups --database PATH --confirm-legacy-reset (app must be stopped)"),
            Self::UnsupportedVersion => f.write_str("Only version-one test groups can be reset by this tool."),
            Self::InvalidCatalog => f.write_str("Invalid legacy group catalog; nothing was reset."),
            Self::Database(error) => write!(f, "Database: {error}"),
            Self::Store(error) => write!(f, "Group storage: {error}"),
            Self::Json(error) => write!(f, "Group catalog: {error}"),
        }
    }
}
impl std::error::Error for ResetError {}
fn db_error(error: impl Into<redb::Error>) -> ResetError {
    ResetError::Database(error.into())
}

fn reset(database: &Database) -> Result<usize, ResetError> {
    let catalog: LegacyCatalog = {
        let read = database.begin_read().map_err(db_error)?;
        let table = match read.open_table(CATALOG) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
            Err(error) => return Err(db_error(error)),
        };
        let value = table
            .get(0)
            .map_err(db_error)?
            .ok_or(ResetError::InvalidCatalog)?;
        if value.value().len() > 262_144 {
            return Err(ResetError::InvalidCatalog);
        }
        let bytes = Zeroizing::new(value.value().to_vec());
        serde_json::from_slice(&bytes).map_err(ResetError::Json)?
    };
    if catalog.version != 1 {
        return Err(ResetError::UnsupportedVersion);
    }
    if catalog.entries.len() > 4
        || catalog.entries.iter().enumerate().any(|(i, e)| {
            catalog.entries[..i]
                .iter()
                .any(|prior| prior.namespace == e.namespace)
        })
    {
        return Err(ResetError::InvalidCatalog);
    }
    let mut owner = NativeStoreOwner::new(database);
    // Validate every incarnation before deleting any namespace. Missing namespaces
    // allow a reset interrupted between durable deletions to resume safely.
    for entry in &catalog.entries {
        let incarnation =
            StoreIncarnation::new(entry.incarnation).map_err(|_| ResetError::InvalidCatalog)?;
        owner
            .validate_delete(StoreNamespace(entry.namespace), incarnation, LEGACY_LIMITS)
            .map_err(ResetError::Store)?;
    }
    for entry in &catalog.entries {
        owner
            .delete(
                StoreNamespace(entry.namespace),
                StoreIncarnation::new(entry.incarnation).map_err(|_| ResetError::InvalidCatalog)?,
                LEGACY_LIMITS,
            )
            .map_err(ResetError::Store)?;
    }
    let mut write = database.begin_write().map_err(db_error)?;
    write
        .set_durability(redb::Durability::Immediate)
        .map_err(db_error)?;
    write.delete_table(ACTIONS).map_err(db_error)?;
    write.delete_table(CATALOG).map_err(db_error)?;
    write.commit().map_err(db_error)?;
    Ok(catalog.entries.len())
}

fn run() -> Result<(), ResetError> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--database")) {
        return Err(ResetError::Usage);
    }
    let path = PathBuf::from(args.next().ok_or(ResetError::Usage)?);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--confirm-legacy-reset"))
        || args.next().is_some()
    {
        return Err(ResetError::Usage);
    }
    // Open never creates a missing database. redb's exclusive process lock keeps
    // this offline operation from racing a running application.
    let database = Database::open(path).map_err(db_error)?;
    let count = reset(&database)?;
    println!("Reset {count} legacy test group(s). Other application tables were preserved.");
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use redb::TableHandle;
    const OTHER: TableDefinition<u8, &[u8]> = TableDefinition::new("unrelated_contacts");
    #[test]
    fn reset_preserves_unrelated_data_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let db = Database::create(root.path().join("test.redb")).unwrap();
        let ns = StoreNamespace([1; 16]);
        let inc = StoreIncarnation::new([2; 24]).unwrap();
        NativeStoreOwner::new(&db)
            .initialize(ns, inc, LEGACY_LIMITS)
            .unwrap();
        let write = db.begin_write().unwrap();
        let catalog = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "entries": [{"namespace": ns.0, "incarnation": vec![2; 24]}]
        }))
        .unwrap();
        write
            .open_table(CATALOG)
            .unwrap()
            .insert(0, catalog.as_slice())
            .unwrap();
        write
            .open_table(OTHER)
            .unwrap()
            .insert(0, b"keep".as_slice())
            .unwrap();
        write
            .open_table(ACTIONS)
            .unwrap()
            .insert(b"action".as_slice(), b"pending".as_slice())
            .unwrap();
        write.commit().unwrap();
        assert_eq!(reset(&db).unwrap(), 1);
        assert_eq!(reset(&db).unwrap(), 0);
        assert!(matches!(
            NativeStoreOwner::new(&db).open(ns, inc, LEGACY_LIMITS),
            Err(NativeError::MissingNamespace)
        ));
        assert_eq!(
            db.begin_read()
                .unwrap()
                .open_table(OTHER)
                .unwrap()
                .get(0)
                .unwrap()
                .unwrap()
                .value(),
            b"keep"
        );
        assert!(matches!(
            db.begin_read().unwrap().open_table(ACTIONS),
            Err(redb::TableError::TableDoesNotExist(_))
        ));
    }
    #[test]
    fn current_catalog_is_never_reset() {
        let root = tempfile::tempdir().unwrap();
        let db = Database::create(root.path().join("test.redb")).unwrap();
        let write = db.begin_write().unwrap();
        write
            .open_table(CATALOG)
            .unwrap()
            .insert(0, b"{\"version\":2,\"entries\":[]}".as_slice())
            .unwrap();
        write.commit().unwrap();
        assert!(matches!(reset(&db), Err(ResetError::UnsupportedVersion)));
        assert!(db
            .begin_read()
            .unwrap()
            .open_table(CATALOG)
            .unwrap()
            .get(0)
            .unwrap()
            .is_some());
    }

    enum LaterNamespaceIssue {
        PartialTables,
        WrongIncarnation,
    }

    fn assert_invalid_later_namespace_preserves_reset_scope(issue: LaterNamespaceIssue) {
        let root = tempfile::tempdir().unwrap();
        let db = Database::create(root.path().join("test.redb")).unwrap();
        let first = StoreNamespace([1; 16]);
        let second = StoreNamespace([3; 16]);
        let incarnation = StoreIncarnation::new([2; 24]).unwrap();
        NativeStoreOwner::new(&db)
            .initialize(first, incarnation, LEGACY_LIMITS)
            .unwrap();
        let first_tables = db
            .begin_read()
            .unwrap()
            .list_tables()
            .unwrap()
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        NativeStoreOwner::new(&db)
            .initialize(second, incarnation, LEGACY_LIMITS)
            .unwrap();
        if matches!(issue, LaterNamespaceIssue::PartialTables) {
            let second_table = db
                .begin_read()
                .unwrap()
                .list_tables()
                .unwrap()
                .find(|table| !first_tables.iter().any(|name| name == table.name()))
                .unwrap();
            let write = db.begin_write().unwrap();
            write.delete_table(second_table).unwrap();
            write.commit().unwrap();
        }
        let second_incarnation = match issue {
            LaterNamespaceIssue::PartialTables => [2; 24],
            LaterNamespaceIssue::WrongIncarnation => [4; 24],
        };
        let catalog = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "entries": [
                {"namespace": first.0, "incarnation": vec![2; 24]},
                {"namespace": second.0, "incarnation": second_incarnation},
            ],
        }))
        .unwrap();
        let write = db.begin_write().unwrap();
        write
            .open_table(CATALOG)
            .unwrap()
            .insert(0, catalog.as_slice())
            .unwrap();
        write
            .open_table(ACTIONS)
            .unwrap()
            .insert(b"action".as_slice(), b"pending".as_slice())
            .unwrap();
        write
            .open_table(OTHER)
            .unwrap()
            .insert(0, b"keep".as_slice())
            .unwrap();
        write.commit().unwrap();

        let result = reset(&db);
        match issue {
            LaterNamespaceIssue::PartialTables => assert!(matches!(
                result,
                Err(ResetError::Store(NativeError::MissingNamespace))
            )),
            LaterNamespaceIssue::WrongIncarnation => assert!(matches!(
                result,
                Err(ResetError::Store(NativeError::ForeignIncarnation))
            )),
        }
        assert!(NativeStoreOwner::new(&db)
            .open(first, incarnation, LEGACY_LIMITS)
            .is_ok());
        let read = db.begin_read().unwrap();
        assert_eq!(
            read.open_table(CATALOG)
                .unwrap()
                .get(0)
                .unwrap()
                .unwrap()
                .value(),
            catalog
        );
        assert_eq!(
            read.open_table(ACTIONS)
                .unwrap()
                .get(b"action".as_slice())
                .unwrap()
                .unwrap()
                .value(),
            b"pending"
        );
        assert_eq!(
            read.open_table(OTHER)
                .unwrap()
                .get(0)
                .unwrap()
                .unwrap()
                .value(),
            b"keep"
        );
        let retained_tables = read
            .list_tables()
            .unwrap()
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        assert!(first_tables
            .iter()
            .all(|name| retained_tables.contains(name)));
    }

    #[test]
    fn partial_later_namespace_does_not_delete_earlier_groups_or_actions() {
        assert_invalid_later_namespace_preserves_reset_scope(LaterNamespaceIssue::PartialTables);
    }

    #[test]
    fn wrong_later_incarnation_does_not_delete_earlier_groups_or_actions() {
        assert_invalid_later_namespace_preserves_reset_scope(LaterNamespaceIssue::WrongIncarnation);
    }
}
