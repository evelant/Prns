#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    development_store::DevelopmentStoreOwner, snapshot::SnapshotStore,
    test_support::foreign_block_on,
};
use prns_groups_runner_native::{EnrollmentRequest, EnrollmentResult};
use prns_host::IdentityHash;
use std::{
    future::Future,
    path::Path,
    sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
    task::{Context, Waker},
};

fn open(root: &Path, identity: IdentityHash) -> Arc<Supervisor> {
    std::fs::create_dir_all(root).unwrap();
    let store = DevelopmentStoreOwner::open(root, &root.join("application.redb")).unwrap();
    store.initialize_groups_blocking(identity).unwrap();
    Arc::new(Supervisor {
        snapshots: Arc::new(SnapshotStore::new()),
        operation_admitted: Arc::new(AtomicBool::new(false)),
        state: Mutex::new(SupervisorState {
            application_owner: Some(store),
            ..SupervisorState::default()
        }),
    })
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

fn request_token(nonce: u8) -> [u8; 32] {
    static ISSUED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    prns_groups_runner_native::RequestToken::new(
        *ISSUED.get_or_init(|| now_secs().unwrap()),
        [nonce; 20],
    )
    .unwrap()
    .into_bytes()
}

fn created(supervisor: &Supervisor, request: u8, title: &str) -> [u8; 32] {
    match foreign_block_on(create_with_supervisor(
        supervisor,
        CreateGroupInput {
            client_request_id: request_token(request),
            title: title.into(),
        },
    )) {
        CreateGroupOutcome::Accepted { group_id } => group_id,
        outcome => panic!("creation failed: {outcome:?}"),
    }
}

#[test]
fn an_empty_app_catalog_can_receive_enrollment_without_a_group_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("prns/development");
    let identity = IdentityHash::new([101; 16]);
    let supervisor = open(&root, identity);
    let client = client(&supervisor).unwrap();
    assert_eq!(foreign_block_on(client.identity()).unwrap(), identity);
    assert!(foreign_block_on(client.network_configs())
        .unwrap()
        .is_empty());
    assert_eq!(
        foreign_block_on(conversations_with_supervisor(&supervisor)),
        GroupConversationListOutcome::Ready {
            conversations: Vec::new()
        }
    );
    assert!(foreign_block_on(client.invitations(now_secs().unwrap()))
        .unwrap()
        .is_empty());
    let prepared = foreign_block_on(crate::groups::prepare_network(client))
        .unwrap()
        .unwrap();
    let destinations = prepared.destination_configs().unwrap();
    assert_eq!(destinations.len(), 2);
    assert_eq!(destinations[0].name.aspects(), &["groups".to_owned()]);
    assert_eq!(
        destinations[1].name.aspects(),
        &["groups".to_owned(), "enrollment".to_owned()]
    );
    assert!(!root.join("group-fixture.json").exists());
    assert_eq!(
        supervisor.snapshots.read().runtime,
        DevelopmentNodeRuntime::Stopped
    );
    close(&supervisor);
}

#[test]
fn offline_creation_survives_caller_drop_stop_drain_and_actual_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("prns/development");
    let identity = IdentityHash::new([102; 16]);
    let supervisor = open(&root, identity);
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .admit_test_barrier(entered, released)
        .unwrap();
    ready.recv().unwrap();
    let input = CreateGroupInput {
        client_request_id: request_token(103),
        title: "Offline crew".into(),
    };
    let mut caller = Box::pin(create_with_supervisor(&supervisor, input.clone()));
    assert!(caller
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    drop(caller);

    let stopping = Arc::clone(&supervisor);
    let (stop_entered, stop_ready) = mpsc::sync_channel(1);
    let (stop_reply, stopped) = mpsc::sync_channel(1);
    let stopping_thread = std::thread::spawn(move || {
        let mut state = stopping.lock_state();
        stop_entered.send(()).unwrap();
        stop_reply
            .send(super::super::stop_locked(&stopping, &mut state))
            .unwrap();
    });
    stop_ready.recv().unwrap();
    assert!(matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).unwrap();
    assert_eq!(
        stopped.recv().unwrap(),
        DevelopmentNodeStopOutcome::AlreadyStopped
    );
    stopping_thread.join().unwrap();

    let group_id = created(&supervisor, 103, "Offline crew");
    let GroupConversationListOutcome::Ready { conversations } =
        foreign_block_on(conversations_with_supervisor(&supervisor))
    else {
        panic!("history unavailable");
    };
    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].group_id, group_id);
    assert_eq!(conversations[0].owner_identity, Some(*identity.as_bytes()));
    assert_eq!(conversations[0].member_count, 1);
    assert_eq!(conversations[0].members.len(), 1);
    assert_eq!(conversations[0].members[0].identity, *identity.as_bytes());
    assert!(conversations[0].members[0].is_local);
    assert!(conversations[0].members[0].is_owner);
    assert_eq!(
        conversations[0].control_head,
        foreign_block_on(
            client(&supervisor)
                .unwrap()
                .for_group(GroupId(group_id))
                .network_config()
        )
        .unwrap()
        .control
        .head
        .0
    );
    assert!(conversations[0].can_invite);
    let former_client = client(&supervisor).unwrap();
    close(&supervisor);
    assert!(matches!(
        foreign_block_on(former_client.conversations()),
        Err(Error::Unavailable)
    ));

    let reopened = open(&root, identity);
    assert_eq!(created(&reopened, 103, "Offline crew"), group_id);
    let mut conflict = input;
    conflict.title = "Another group".into();
    assert!(matches!(
        foreign_block_on(create_with_supervisor(&reopened, conflict)),
        CreateGroupOutcome::InvalidInput { .. }
    ));
    assert_eq!(
        foreign_block_on(client(&reopened).unwrap().conversations())
            .unwrap()
            .len(),
        1
    );
    close(&reopened);
}

#[test]
fn separate_groups_and_an_offline_invitation_reopen_under_the_same_app_store_owner() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("prns/development");
    let identity = IdentityHash::new([104; 16]);
    let recipient = IdentityHash::new([105; 16]);
    let supervisor = open(&root, identity);
    let first = created(&supervisor, 106, "First group");
    let second = created(&supervisor, 107, "Second group");
    assert_ne!(first, second);
    let groups = client(&supervisor).unwrap();
    assert_eq!(foreign_block_on(groups.network_configs()).unwrap().len(), 2);
    let before = foreign_block_on(groups.conversations()).unwrap();
    assert_eq!(before.len(), 2);
    assert_eq!(
        before
            .iter()
            .map(|entry| entry.title.as_str())
            .collect::<Vec<_>>(),
        vec!["First group", "Second group"]
    );
    for group in [first, second] {
        let config = foreign_block_on(groups.for_group(GroupId(group)).network_config()).unwrap();
        assert_eq!(config.group, GroupId(group));
        assert!(
            foreign_block_on(groups.messages(GroupId(group), None, 50, SystemClock.now()))
                .unwrap()
                .messages
                .is_empty()
        );
    }
    let request = EnrollmentRequest::Invite {
        request_id: request_token(110),
        group: GroupId(first),
        recipient,
        now: now_secs().unwrap(),
    };
    let EnrollmentResult::Invited(invitation_id) =
        foreign_block_on(enroll_with_supervisor(&supervisor, request.clone())).unwrap()
    else {
        panic!("not an invitation");
    };
    let invitations = foreign_block_on(groups.invitations(now_secs().unwrap())).unwrap();
    assert_eq!(invitations.len(), 1);
    assert_eq!(invitations[0].id, invitation_id);
    assert_eq!(invitations[0].recipient, recipient);
    assert_eq!(invitations[0].inviter, identity);
    assert_eq!(invitations[0].title, "First group");
    assert!(invitations[0].outgoing);
    close(&supervisor);

    let reopened = open(&root, identity);
    let EnrollmentResult::Invited(retried) =
        foreign_block_on(enroll_with_supervisor(&reopened, request)).unwrap()
    else {
        panic!("not an invitation");
    };
    assert_eq!(retried, invitation_id);
    let client = client(&reopened).unwrap();
    assert_eq!(foreign_block_on(client.conversations()).unwrap().len(), 2);
    assert_eq!(
        foreign_block_on(client.invitations(now_secs().unwrap()))
            .unwrap()
            .len(),
        1
    );
    let after = foreign_block_on(client.conversations()).unwrap();
    assert_eq!(
        after
            .iter()
            .map(|entry| (entry.group, entry.title.as_str()))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|entry| (entry.group, entry.title.as_str()))
            .collect::<Vec<_>>()
    );
    for group in [first, second] {
        assert!(
            foreign_block_on(client.messages(GroupId(group), None, 50, SystemClock.now()))
                .unwrap()
                .messages
                .is_empty()
        );
    }
    close(&reopened);
}
