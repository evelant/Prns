#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use prns_lxmf::{mailbox::*, wire::*, LxmfVerification};
const PEER: [u8; 16] = [7; 16];
struct Signer;
impl BasicLxmfSigner for Signer {
    fn sign_lxmf(&self, _: &[u8]) -> [u8; 64] {
        [9; 64]
    }
}
pub(in crate::named_identities) fn incoming(
    peer: [u8; 16],
    destination: [u8; 16],
    timestamp: u64,
) -> NewInboundMessage {
    let mut bytes = [0; MAX_BASIC_LXMF_WIRE_BYTES];
    let wire = compose_basic_direct_lxmf(
        destination,
        peer,
        timestamp,
        b"",
        b"local activity",
        None,
        &Signer,
        &mut bytes,
    )
    .unwrap();
    NewInboundMessage {
        message_id: wire.message_id(),
        source: peer,
        destination,
        timestamp_unix_ms: timestamp,
        title: vec![],
        content: b"local activity".to_vec(),
        exact_wire: bytes[..usize::from(wire.wire_len())].to_vec(),
        verification: LxmfVerification::Verified,
    }
}
fn database(path: &Path) -> Database {
    let db = Database::create(path).unwrap();
    let write = db.begin_write().unwrap();
    initialize_mailbox_tables(&write).unwrap();
    write.commit().unwrap();
    db
}
fn insert(db: &Database, message: NewInboundMessage) {
    execute_mailbox_request(db, MailboxRequest::InsertInbound(message)).unwrap();
}
fn summary(db: &Database) -> IdentityActivitySummary {
    let mut manager = prns_groups_runner_native::GroupManager::open(
        db,
        prns_host::IdentityHash::new([1; 16]),
        None,
    )
    .unwrap();
    match execute(db, Some(&mut manager), Operation::Read).unwrap() {
        Reply::Read(summary) => summary,
        _ => panic!("reply"),
    }
}
fn mark_seen(db: &Database, peer: [u8; 16], position: u64) -> IdentityActivitySeenOutcome {
    match execute(
        db,
        None,
        Operation::Mark(MarkIdentityActivitySeenInput {
            scope: IdentityActivityScope::Direct { destination: peer },
            through_sequence: position,
        }),
    )
    .unwrap()
    {
        Reply::Mark(result) => result,
        _ => panic!("reply"),
    }
}
#[test]
fn durable_seen_scope_ignores_duplicates_receipts_and_later_arrivals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.redb");
    let db = database(&path);
    let message = incoming(PEER, [1; 16], 1000);
    insert(&db, message.clone());
    insert(&db, message); // Durable ingress duplicate creates no new position.
    let initial = summary(&db);
    assert_eq!(initial.unread_direct, 1);
    assert!(!initial.partial);
    let captured = initial.conversations[0].through_sequence;
    insert(&db, incoming(PEER, [1; 16], 2000));
    insert(&db, incoming([8; 16], [1; 16], 3000));
    assert_eq!(
        mark_seen(&db, PEER, captured),
        IdentityActivitySeenOutcome::Saved
    );
    assert_eq!(
        summary(&db).unread_direct,
        2,
        "marking old visible page must preserve later and other-peer activity"
    );
    assert!(
        matches!(
            mark_seen(&db, PEER, 3),
            IdentityActivitySeenOutcome::InvalidInput { .. }
        ),
        "other peer cannot supply a watermark"
    );
    assert!(matches!(
        mark_seen(&db, PEER, u64::MAX),
        IdentityActivitySeenOutcome::InvalidInput { .. }
    ));
    assert_eq!(mark_seen(&db, PEER, 0), IdentityActivitySeenOutcome::Saved);
    drop(db);
    let db = Database::open(&path).unwrap();
    assert_eq!(summary(&db).unread_direct, 2);
    let other = database(&dir.path().join("b.redb"));
    insert(&other, incoming(PEER, [1; 16], 1000));
    assert_eq!(
        summary(&other).unread_direct,
        1,
        "another owner must have an independent seen cursor"
    );
    let mut manager = prns_groups_runner_native::GroupManager::open(
        &db,
        prns_host::IdentityHash::new([1; 16]),
        None,
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(
        matches!(execute(&db, Some(&mut manager), Operation::Read).unwrap(), Reply::Read(summary) if summary.unread_direct == 2)
    );
    assert_eq!(
        mark_seen(&db, PEER, captured),
        IdentityActivitySeenOutcome::Saved
    );
    assert!(
        std::fs::read(&path).unwrap() == before,
        "polling and duplicate acknowledgments need no flash write"
    );
}
#[test]
fn bounded_direct_projection_reports_omission_and_cursor_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(&dir.path().join("bounded.redb"));
    for peer in 0..65_u8 {
        insert(
            &db,
            incoming([peer; 16], [99; 16], (u64::from(peer) + 1) * 1000),
        );
    }
    let result = summary(&db);
    assert_eq!(result.unread_direct, 65);
    assert_eq!(result.conversations.len(), MAX_DIRECT_CONVERSATIONS);
    assert!(result.partial);
    let (rows, partial) = read_mailbox_activity(&db, 2).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(partial);
    let write = db.begin_write().unwrap();
    {
        let mut table = write.open_table(SEEN).unwrap();
        for index in 0..MAX_SEEN_SCOPES {
            let mut scope = [0; 16];
            scope[..8].copy_from_slice(&(index as u64).to_be_bytes());
            table
                .insert(
                    key(&IdentityActivityScope::Direct { destination: scope }).as_slice(),
                    1,
                )
                .unwrap();
        }
    }
    write.commit().unwrap();
    assert_eq!(
        mark_seen(&db, [64; 16], 65),
        IdentityActivitySeenOutcome::Capacity
    );
}

#[test]
fn process_child_activity_commit() {
    let Some(path) = std::env::var_os("PRNS_ACTIVITY_CRASH_DATABASE") else {
        return;
    };
    let db = Database::open(path).unwrap();
    assert_eq!(mark_seen(&db, PEER, 1), IdentityActivitySeenOutcome::Saved);
    std::process::exit(73);
}
#[test]
fn seen_commit_survives_abrupt_process_exit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash.redb");
    let db = database(&path);
    insert(&db, incoming(PEER, [1; 16], 1000));
    drop(db);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "named_identities::activity::tests::process_child_activity_commit",
            "--nocapture",
        ])
        .env("PRNS_ACTIVITY_CRASH_DATABASE", &path)
        .status()
        .unwrap();
    assert_eq!(child.code(), Some(73));
    let db = Database::open(path).unwrap();
    assert_eq!(summary(&db).unread_direct, 0);
}
