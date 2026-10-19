//! One immutable local-identity scope per native service database.
//!
//! Local record IDs, request IDs and cursors are meaningful only together with
//! [`StorageScope`]. Keep clients pinned to this owner; never retarget a queued
//! operation using a UI's currently selected identity. Lifecycle authorization
//! remains the embedding runtime's responsibility.

use prns_identities_core::{LocalIdentityId, PublicIdentityHash};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

const BINDING: TableDefinition<u8, &[u8]> = TableDefinition::new("prns_identity_scope_v1");
const MAGIC: &[u8; 8] = b"PRNSSCP1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageScope {
    pub identity: LocalIdentityId,
    pub public_hash: PublicIdentityHash,
}

impl StorageScope {
    fn validate(self) -> Result<(), Error> {
        if self.identity.incarnation == [0; 16] || self.identity.sequence == 0 {
            return Err(Error::InvalidScope);
        }
        Ok(())
    }

    /// Deterministic file name. Public keys and user-supplied labels are not path
    /// components; importing another key under an existing local ID cannot select
    /// a second database and bypass its binding check.
    pub fn path_under(self, root: &Path) -> Result<PathBuf, Error> {
        self.validate()?;
        let mut name = String::with_capacity(64);
        use std::fmt::Write;
        for byte in self.identity.incarnation {
            write!(&mut name, "{byte:02x}").map_err(|_| Error::InvalidScope)?;
        }
        write!(&mut name, "-{:016x}.redb", self.identity.sequence)
            .map_err(|_| Error::InvalidScope)?;
        Ok(root.join(name))
    }

    fn encoded(self) -> [u8; 48] {
        let mut bytes = [0; 48];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..24].copy_from_slice(&self.identity.incarnation);
        bytes[24..32].copy_from_slice(&self.identity.sequence.to_be_bytes());
        bytes[32..].copy_from_slice(&self.public_hash.0);
        bytes
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Database(redb::Error),
    InvalidScope,
    MissingBinding,
    WrongBinding,
    UnsafePath,
    UnsupportedPlatform,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "identity-scoped service storage: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
fn db(error: impl Into<redb::Error>) -> Error {
    Error::Database(error.into())
}

/// A service database permanently bound to one local identity. Opening another
/// owner at the same path uses redb's exclusive lock. Moving/copying a file cannot
/// make it belong to another scope; open verifies its durable binding first.
pub struct ScopedDatabase {
    database: Database,
    scope: StorageScope,
}

impl ScopedDatabase {
    /// Explicitly create a new file. Existing/incomplete files are never replaced.
    /// Service schemas must subsequently initialize atomically and validate on
    /// open; a marker-only file is not evidence of service-store readiness.
    pub fn create_new(path: &Path, scope: StorageScope) -> Result<Self, Error> {
        scope.validate()?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        return Err(Error::UnsupportedPlatform);
        let file = options.open(path)?;
        let database = Database::builder().create_file(file).map_err(db)?;
        let mut tx = database.begin_write().map_err(db)?;
        tx.set_durability(redb::Durability::Immediate).map_err(db)?;
        tx.open_table(BINDING)
            .map_err(db)?
            .insert(0, scope.encoded().as_slice())
            .map_err(db)?;
        tx.commit().map_err(db)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()?;
        Ok(Self { database, scope })
    }

    /// Open a complete existing binding. Missing markers and wrong owners fail
    /// without initializing service tables or rewriting the file's identity.
    pub fn open(path: &Path, scope: StorageScope) -> Result<Self, Error> {
        scope.validate()?;
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Error::UnsafePath);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
                return Err(Error::UnsafePath);
            }
        }
        #[cfg(not(unix))]
        return Err(Error::UnsupportedPlatform);
        let database = Database::open(path).map_err(db)?;
        {
            let read = database.begin_read().map_err(db)?;
            let binding = match read.open_table(BINDING) {
                Ok(table) => table,
                Err(redb::TableError::TableDoesNotExist(_)) => return Err(Error::MissingBinding),
                Err(error) => return Err(db(error)),
            };
            let value = binding.get(0).map_err(db)?.ok_or(Error::MissingBinding)?;
            if value.value() != scope.encoded() {
                return Err(Error::WrongBinding);
            }
            let mut entries = binding.iter().map_err(db)?;
            let (key, _) = entries.next().ok_or(Error::MissingBinding)?.map_err(db)?;
            if key.value() != 0 || entries.next().transpose().map_err(db)?.is_some() {
                return Err(Error::WrongBinding);
            }
        }
        Ok(Self { database, scope })
    }

    pub const fn scope(&self) -> StorageScope {
        self.scope
    }
    pub fn database(&self) -> &Database {
        &self.database
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn binding_survives_reopen_and_rejects_other_local_owner_or_key() {
        let root = std::env::temp_dir().join(format!("prns-identity-scope-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let scope = StorageScope {
            identity: LocalIdentityId {
                incarnation: [7; 16],
                sequence: 1,
            },
            public_hash: PublicIdentityHash([8; 16]),
        };
        let path = scope.path_under(&root).unwrap();
        let store = ScopedDatabase::create_new(&path, scope).unwrap();
        assert_eq!(store.scope(), scope);
        assert!(ScopedDatabase::create_new(&path, scope).is_err());
        drop(store);
        for wrong in [
            StorageScope {
                public_hash: PublicIdentityHash([9; 16]),
                ..scope
            },
            StorageScope {
                identity: LocalIdentityId {
                    sequence: 2,
                    ..scope.identity
                },
                ..scope
            },
            StorageScope {
                identity: LocalIdentityId {
                    incarnation: [9; 16],
                    ..scope.identity
                },
                ..scope
            },
        ] {
            assert!(matches!(
                ScopedDatabase::open(&path, wrong),
                Err(Error::WrongBinding)
            ));
        }
        let store = ScopedDatabase::open(&path, scope).unwrap();
        assert_eq!(store.scope(), scope);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn absent_binding_and_invalid_scope_never_initialize_an_existing_database() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "prns-identity-scope-missing-{}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let scope = StorageScope {
            identity: LocalIdentityId {
                incarnation: [7; 16],
                sequence: 1,
            },
            public_hash: PublicIdentityHash([8; 16]),
        };
        let path = scope.path_under(&root).unwrap();
        drop(Database::create(&path).unwrap());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            ScopedDatabase::open(&path, scope),
            Err(Error::MissingBinding)
        ));
        assert!(ScopedDatabase::create_new(&path, scope).is_err());
        let raw = Database::open(&path).unwrap();
        assert!(matches!(
            raw.begin_read().unwrap().open_table(BINDING),
            Err(redb::TableError::TableDoesNotExist(_))
        ));
        drop(raw);
        for identity in [
            LocalIdentityId {
                incarnation: [0; 16],
                sequence: 1,
            },
            LocalIdentityId {
                sequence: 0,
                ..scope.identity
            },
        ] {
            let invalid = StorageScope { identity, ..scope };
            assert!(matches!(
                invalid.path_under(&root),
                Err(Error::InvalidScope)
            ));
            assert!(matches!(
                ScopedDatabase::create_new(&root.join("invalid.redb"), invalid),
                Err(Error::InvalidScope)
            ));
        }
        assert!(!root.join("invalid.redb").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
