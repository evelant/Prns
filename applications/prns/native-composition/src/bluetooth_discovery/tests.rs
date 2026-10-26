#![allow(clippy::unwrap_used)]
use super::*;
use crate::development_store::DevelopmentStoreOwner;

#[test]
fn validates_byte_bounds_and_canonical_distinct_set() {
    assert_eq!(names(parse(&["b".into(), "a".into()]).unwrap()), ["a", "b"]);
    for invalid in [
        vec![],
        vec!["".into()],
        vec!["x".into(); 5],
        vec!["x".into(); 2],
        vec!["é".repeat(17)],
    ] {
        assert!(parse(&invalid).is_err());
    }
    assert!(parse(&["é".repeat(16)]).is_ok());
}

#[test]
fn default_and_canonical_setting_survive_real_store_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("application.redb");
    let store = DevelopmentStoreOwner::open(dir.path(), &path).unwrap();
    assert_eq!(
        store.read_bluetooth_discovery_blocking().unwrap(),
        DiscoveryGroupSet::reticulum()
    );
    let groups = parse(&["trial-b".into(), "trial-a".into()]).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    store
        .admit_bluetooth_discovery(Some(groups), move |result| send.send(result).unwrap())
        .unwrap();
    assert_eq!(receive.recv().unwrap().unwrap(), groups);
    let first_owner = store.owner_id();
    store.close().unwrap();
    let store = DevelopmentStoreOwner::open(dir.path(), &path).unwrap();
    assert_ne!(store.owner_id(), first_owner);
    assert_eq!(store.read_bluetooth_discovery_blocking().unwrap(), groups);
    store.close().unwrap();
}

#[test]
fn corrupt_saved_value_is_not_silently_replaced_with_default() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::create(dir.path().join("settings.redb")).unwrap();
    let write = db.begin_write().unwrap();
    {
        write
            .open_table(APPLICATION_SETTINGS)
            .unwrap()
            .insert(KEY, "[\"x\",\"x\"]")
            .unwrap();
    }
    write.commit().unwrap();
    assert!(settings(&db, None).is_err());
    assert_eq!(
        settings(&db, Some(DiscoveryGroupSet::reticulum())).unwrap(),
        DiscoveryGroupSet::reticulum()
    );
}
