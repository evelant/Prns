#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::test_support::foreign_block_on;
use std::{
    future::Future,
    task::{Context, Waker},
};

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
fn read(supervisor: &Supervisor) -> Settings {
    match foreign_block_on(settings_with_supervisor(supervisor, None)) {
        Outcome::Ready { settings } => settings,
        other => panic!("unexpected {other:?}"),
    }
}
fn change(settings: &Settings, name: &str) -> SetBluetoothDiscoveryGroupsInput {
    SetBluetoothDiscoveryGroupsInput {
        owner_id: settings.owner_id,
        generation_id: settings.generation_id,
        groups: vec![name.into()],
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
fn poll_pending(future: std::pin::Pin<&mut impl Future>) {
    assert!(future
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
}

#[test]
fn stopped_settings_are_durable_and_old_owner_tokens_cannot_mutate_reopened_store() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let initial = read(&supervisor);
    assert_eq!(initial.groups, ["reticulum"]);
    assert_eq!(initial.applied_groups, None);
    let input = change(&initial, "trial-a");
    assert!(matches!(
        foreign_block_on(settings_with_supervisor(&supervisor, Some(input.clone()))),
        Outcome::Ready { .. }
    ));
    close(&supervisor);
    supervisor.lock_state().application_owner = Some(
        DevelopmentStoreOwner::open(dir.path(), &dir.path().join("application.redb")).unwrap(),
    );
    assert_eq!(read(&supervisor).groups, ["trial-a"]);
    assert_eq!(
        foreign_block_on(settings_with_supervisor(&supervisor, Some(input))),
        Outcome::GenerationChanged
    );
    let mut stale = change(&read(&supervisor), "wrong");
    stale.generation_id += 1;
    assert_eq!(
        foreign_block_on(settings_with_supervisor(&supervisor, Some(stale))),
        Outcome::GenerationChanged
    );
    assert_eq!(read(&supervisor).groups, ["trial-a"]);
    close(&supervisor);
}

#[test]
fn canceled_foreign_waiter_keeps_admitted_write_until_actual_store_commit() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let initial = read(&supervisor);
    let (entered, entered_rx) = std_mpsc::sync_channel(1);
    let (release, release_rx) = std_mpsc::sync_channel(1);
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .admit_test_barrier(entered, release_rx)
        .unwrap();
    entered_rx.recv().unwrap();
    let mut operation = Box::pin(settings_with_supervisor(
        &supervisor,
        Some(change(&initial, "trial-a")),
    ));
    poll_pending(operation.as_mut());
    drop(operation);
    assert_eq!(
        foreign_block_on(settings_with_supervisor(
            &supervisor,
            Some(change(&initial, "trial-b"))
        )),
        Outcome::Busy
    );
    release.send(()).unwrap();
    assert_eq!(read(&supervisor).groups, ["trial-a"]);
    close(&supervisor);
    let reopened = open(dir.path());
    assert_eq!(read(&reopened).groups, ["trial-a"]);
    close(&reopened);
}

#[test]
fn live_replacement_is_settled_not_optimistic_and_cancellation_does_not_release_gate() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (commands, _commands) = mpsc::channel(1);
    let (shutdown, _shutdown) = watch::channel(false);
    let (_done, done) = std_mpsc::sync_channel(1);
    let worker = super::super::tests::test_worker(
        commands,
        ShutdownSignal {
            sender: shutdown,
            requested: Arc::new(AtomicBool::new(false)),
        },
        done,
        None,
        dir.path().into(),
    );
    worker.runtime.set(runtime.handle().clone()).unwrap();
    let (entered, entered_rx) = std_mpsc::sync_channel(1);
    let (release, release_rx) = oneshot::channel::<()>();
    let release_rx = Arc::new(Mutex::new(Some(release_rx)));
    assert!(worker
        .bluetooth
        .set(BluetoothControl::for_discovery_test(Box::new(move |_| {
            let release = release_rx.lock().unwrap().take().unwrap();
            let entered = entered.clone();
            Box::pin(async move {
                entered.send(()).unwrap();
                release.await.is_ok()
            })
        })))
        .is_ok());
    supervisor.lock_state().worker = Some(worker);
    supervisor
        .snapshots
        .set_runtime(DevelopmentNodeRuntime::Running);
    let initial = read(&supervisor);
    assert_eq!(initial.applied_groups, Some(vec!["reticulum".into()]));
    let mut operation = Box::pin(settings_with_supervisor(
        &supervisor,
        Some(change(&initial, "trial-a")),
    ));
    poll_pending(operation.as_mut());
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(operation);
    let pending = read(&supervisor);
    assert_eq!(pending.groups, ["trial-a"]);
    assert_eq!(pending.applied_groups, None);
    assert_eq!(
        foreign_block_on(settings_with_supervisor(
            &supervisor,
            Some(change(&initial, "trial-b"))
        )),
        Outcome::Busy
    );
    assert_eq!(
        foreign_block_on(super::super::bluetooth_settings_with_supervisor(
            &supervisor,
            Some(false)
        )),
        LocalBluetoothSettingsOutcome::Busy
    );
    release.send(()).unwrap();
    for _ in 0..100 {
        if read(&supervisor).applied_groups == Some(vec!["trial-a".into()]) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        read(&supervisor).applied_groups,
        Some(vec!["trial-a".into()])
    );
    supervisor
        .lock_state()
        .worker
        .as_ref()
        .unwrap()
        .shutdown
        .request();
    assert_eq!(read(&supervisor).applied_groups, None);
    supervisor.snapshots.stopped();
    assert_eq!(read(&supervisor).applied_groups, None);
    close(&supervisor);
}

#[test]
fn stopping_runtime_during_replacement_preserves_saved_names_and_releases_admission() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (commands, _commands) = mpsc::channel(1);
    let (shutdown, _shutdown) = watch::channel(false);
    let (_done, done) = std_mpsc::sync_channel(1);
    let worker = super::super::tests::test_worker(
        commands,
        ShutdownSignal {
            sender: shutdown,
            requested: Arc::new(AtomicBool::new(false)),
        },
        done,
        None,
        dir.path().into(),
    );
    worker.runtime.set(runtime.handle().clone()).unwrap();
    let (entered, entered_rx) = std_mpsc::sync_channel(1);
    assert!(worker
        .bluetooth
        .set(BluetoothControl::for_discovery_test(Box::new(move |_| {
            let entered = entered.clone();
            Box::pin(async move {
                entered.send(()).unwrap();
                std::future::pending().await
            })
        })))
        .is_ok());
    supervisor.lock_state().worker = Some(worker);
    supervisor
        .snapshots
        .set_runtime(DevelopmentNodeRuntime::Running);
    let initial = read(&supervisor);
    let mut operation = Box::pin(settings_with_supervisor(
        &supervisor,
        Some(change(&initial, "trial-a")),
    ));
    poll_pending(operation.as_mut());
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    supervisor
        .lock_state()
        .worker
        .as_ref()
        .unwrap()
        .shutdown
        .request();
    supervisor.snapshots.stopped();
    drop(runtime); // Aborts the generation-owned continuation; the committed setting remains.
    assert!(matches!(
        foreign_block_on(operation),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(read(&supervisor).groups, ["trial-a"]);
    assert_eq!(read(&supervisor).applied_groups, None);
    supervisor.lock_state().worker.take();
    assert!(matches!(
        foreign_block_on(settings_with_supervisor(
            &supervisor,
            Some(change(&read(&supervisor), "trial-b"))
        )),
        Outcome::Ready { .. }
    ));
    close(&supervisor);
    let reopened = open(dir.path());
    assert_eq!(read(&reopened).groups, ["trial-b"]);
    close(&reopened);
}

#[test]
fn requested_but_unsettled_disable_cannot_acknowledge_new_groups_before_old_peers_close() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let (commands, _commands) = mpsc::channel(1);
    let (shutdown, _shutdown) = watch::channel(false);
    let (_done, done) = std_mpsc::sync_channel(1);
    let worker = super::super::tests::test_worker(
        commands,
        ShutdownSignal {
            sender: shutdown,
            requested: Arc::new(AtomicBool::new(false)),
        },
        done,
        None,
        dir.path().into(),
    );
    assert!(worker
        .bluetooth
        .set(BluetoothControl::for_disabling_test())
        .is_ok());
    supervisor.lock_state().worker = Some(worker);
    supervisor
        .snapshots
        .set_runtime(DevelopmentNodeRuntime::Running);
    let initial = read(&supervisor);
    assert_eq!(initial.applied_groups, None);
    assert_eq!(
        foreign_block_on(settings_with_supervisor(
            &supervisor,
            Some(change(&initial, "trial-a"))
        )),
        Outcome::Busy
    );
    assert_eq!(read(&supervisor).groups, ["reticulum"]);
    close(&supervisor);
}

#[test]
fn public_read_returns_fresh_repair_fences_for_corrupt_settings_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let previous = read(&supervisor);
    close(&supervisor);
    {
        let db = redb::Database::open(dir.path().join("application.redb")).unwrap();
        let write = db.begin_write().unwrap();
        {
            write
                .open_table(crate::development_store::APPLICATION_SETTINGS)
                .unwrap()
                .insert(crate::bluetooth_discovery::KEY, "not a valid list")
                .unwrap();
        }
        write.commit().unwrap();
    }
    supervisor.lock_state().application_owner = Some(
        DevelopmentStoreOwner::open(dir.path(), &dir.path().join("application.redb")).unwrap(),
    );
    let Outcome::InvalidSaved {
        owner_id,
        generation_id,
        ..
    } = foreign_block_on(settings_with_supervisor(&supervisor, None))
    else {
        panic!("corruption must be visible and repairable");
    };
    assert_ne!(owner_id, previous.owner_id);
    assert_eq!(
        foreign_block_on(settings_with_supervisor(
            &supervisor,
            Some(change(&previous, "reticulum"))
        )),
        Outcome::GenerationChanged
    );
    let repaired = foreign_block_on(settings_with_supervisor(
        &supervisor,
        Some(SetBluetoothDiscoveryGroupsInput {
            owner_id,
            generation_id,
            groups: vec!["reticulum".into()],
        }),
    ));
    assert!(
        matches!(repaired, Outcome::Ready { settings } if settings.groups == ["reticulum"] && settings.applied_groups.is_none())
    );
    close(&supervisor);
    let reopened = open(dir.path());
    assert_eq!(read(&reopened).groups, ["reticulum"]);
    close(&reopened);
}

#[test]
fn running_node_with_unavailable_bluetooth_does_not_claim_applied_names() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = open(dir.path());
    let (commands, _commands) = mpsc::channel(1);
    let (shutdown, _shutdown) = watch::channel(false);
    let (_done, done) = std_mpsc::sync_channel(1);
    let worker = super::super::tests::test_worker(
        commands,
        ShutdownSignal {
            sender: shutdown,
            requested: Arc::new(AtomicBool::new(false)),
        },
        done,
        None,
        dir.path().into(),
    );
    assert!(worker
        .bluetooth
        .set(BluetoothControl::for_unavailable_test())
        .is_ok());
    supervisor.lock_state().worker = Some(worker);
    supervisor
        .snapshots
        .set_runtime(DevelopmentNodeRuntime::Running);
    assert_eq!(read(&supervisor).applied_groups, None);
    close(&supervisor);
}
