#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::test_support::foreign_block_on;
use std::future::Future;

fn open(path: &Path) -> Supervisor {
    std::fs::create_dir_all(path).unwrap();
    Supervisor {
        snapshots: Arc::new(SnapshotStore::new()),
        operation_admitted: Arc::new(AtomicBool::new(false)),
        state: Mutex::new(SupervisorState {
            application_owner: Some(
                DevelopmentStoreOwner::open(path, &path.join("application.redb")).unwrap(),
            ),
            ..Default::default()
        }),
    }
}
fn read(supervisor: &Supervisor) -> ConnectionSettingsSnapshot {
    match foreign_block_on(settings_with_supervisor(supervisor, None)) {
        ConnectionSettingsOutcome::Ready { snapshot } => snapshot,
        other => panic!("unexpected settings: {other:?}"),
    }
}
fn change(saved: &ConnectionSettingsSnapshot, port: u16) -> ConnectionSettingsUpdateInput {
    ConnectionSettingsUpdateInput {
        owner_id: saved.owner_id,
        expected_revision: saved.revision,
        settings: ConnectionSettings {
            lan_enabled: false,
            tcp_peers: vec![TcpPeerProfile {
                id: String::new(),
                label: "Retained TCP peer".into(),
                host: "127.0.0.1".into(),
                port,
                enabled: true,
            }],
        },
    }
}
fn close(supervisor: &Supervisor) {
    supervisor
        .lock_state()
        .application_owner
        .take()
        .unwrap()
        .close()
        .unwrap();
}

#[test]
fn stopped_settings_survive_reopen_and_reject_stale_revision_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let initial = read(&supervisor);
    assert_eq!(initial.settings, ConnectionSettings::default());
    let input = change(&initial, 4242);
    assert!(matches!(
        foreign_block_on(settings_with_supervisor(&supervisor, Some(input.clone()))),
        ConnectionSettingsOutcome::Ready { .. }
    ));
    assert!(matches!(
        foreign_block_on(settings_with_supervisor(&supervisor, Some(input.clone()))),
        ConnectionSettingsOutcome::Conflict { .. }
    ));
    assert_eq!(
        supervisor.snapshots.read().runtime,
        DevelopmentNodeRuntime::Stopped
    );
    let saved = read(&supervisor);
    close(&supervisor);
    supervisor.lock_state().application_owner = Some(
        DevelopmentStoreOwner::open(dir.path(), &dir.path().join("application.redb")).unwrap(),
    );
    let reopened = read(&supervisor);
    assert_eq!(reopened.settings, saved.settings);
    assert_eq!(reopened.revision, saved.revision);
    assert_ne!(reopened.owner_id, initial.owner_id);
    assert_eq!(
        foreign_block_on(settings_with_supervisor(&supervisor, Some(input))),
        ConnectionSettingsOutcome::OwnerChanged
    );
    close(&supervisor);
}

#[test]
fn canceled_waiter_does_not_cancel_storage_commit() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let initial = read(&supervisor);
    let (entered, wait_entered) = std_mpsc::sync_channel(1);
    let (release, wait_release) = std_mpsc::sync_channel(1);
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .admit_test_barrier(entered, wait_release)
        .unwrap();
    wait_entered.recv().unwrap();
    let mut pending = Box::pin(settings_with_supervisor(
        &supervisor,
        Some(change(&initial, 4242)),
    ));
    assert!(pending
        .as_mut()
        .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
        .is_pending());
    drop(pending);
    release.send(()).unwrap();
    assert_eq!(read(&supervisor).revision, 1);
    close(&supervisor);
}

#[test]
fn native_start_and_generation_restart_restore_saved_tcp_without_javascript() {
    use std::io::ErrorKind;
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("prns/development");
    let paths = prepare_storage(&storage).unwrap();
    FileVault::new(&paths.identities)
        .store(
            &primary_label().unwrap(),
            &[0xc9; personal_rns::identity::IDENTITY_SECRET_KEY_LEN],
        )
        .unwrap();
    let supervisor = Supervisor {
        snapshots: Arc::new(SnapshotStore::new()),
        operation_admitted: Arc::new(AtomicBool::new(false)),
        state: Mutex::new(SupervisorState::default()),
    };
    assert_eq!(
        admission::prepare_native_storage_with_supervisor(&supervisor, &storage),
        NativeStoragePreparationOutcome::Prepared
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let initial = read(&supervisor);
    assert!(matches!(
        foreign_block_on(settings_with_supervisor(
            &supervisor,
            Some(change(&initial, listener.local_addr().unwrap().port()))
        )),
        ConnectionSettingsOutcome::Ready { .. }
    ));
    let profile_id = read(&supervisor).settings.tcp_peers[0].id.clone();
    let mut previous_generation = 0;
    for _ in 0..2 {
        assert!(matches!(
            start_configured_with_supervisor(
                &supervisor,
                &storage,
                DevelopmentNodeStartInput {
                    development_tcp_target: None
                },
                AppleBluetoothPreparation::WithoutRestoration
            ),
            DevelopmentNodeStartOutcome::Started { .. }
        ));
        let until = std::time::Instant::now() + Duration::from_secs(5);
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock
                        && std::time::Instant::now() < until =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                error => panic!("native TCP did not connect: {error:?}"),
            }
        };
        let snapshot = loop {
            match foreign_block_on(snapshot_with_supervisor(&supervisor)) {
                ConnectionSnapshotOutcome::Ready { snapshot }
                    if snapshot.tcp_peers[0].state == LocalConnectionState::Connected =>
                {
                    break snapshot
                }
                _ if std::time::Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                other => panic!("native TCP state not observed: {other:?}"),
            }
        };
        assert_eq!(snapshot.tcp_peers[0].profile.id, profile_id);
        assert!(snapshot.generation_id > previous_generation);
        previous_generation = snapshot.generation_id;
        assert_eq!(snapshot.applied_revision, Some(1));
        assert_eq!(
            stop_locked(&supervisor, &mut supervisor.lock_state()),
            DevelopmentNodeStopOutcome::Stopped
        );
        drop(socket);
    }
    assert_eq!(read(&supervisor).settings.tcp_peers[0].id, profile_id);
    close(&supervisor);
}
