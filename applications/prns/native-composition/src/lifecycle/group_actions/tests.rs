#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::{
    development_store::DevelopmentStoreOwner, snapshot::SnapshotStore,
    test_support::foreign_block_on,
};
use prns_groups_runner_native::{Clock, SystemClock};
use prns_host::IdentityHash;
use std::{
    future::Future,
    path::Path,
    sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
    task::{Context, Waker},
};

const IDENTITY: IdentityHash = IdentityHash::new([91; 16]);
fn open(root: &Path) -> Arc<Supervisor> {
    open_as(root, IDENTITY)
}
fn open_as(root: &Path, identity: IdentityHash) -> Arc<Supervisor> {
    std::fs::create_dir_all(root).unwrap();
    let store = DevelopmentStoreOwner::open(root, &root.join("application.redb")).unwrap();
    store.initialize_groups_blocking(identity).unwrap();
    Arc::new(Supervisor {
        snapshots: Arc::new(SnapshotStore::new()),
        operation_admitted: Arc::new(AtomicBool::new(false)),
        state: Mutex::new(super::super::SupervisorState {
            application_owner: Some(store),
            ..Default::default()
        }),
    })
}

fn exchange_enrollment(sender: &Client, recipient: &Client) {
    let sender_identity = foreign_block_on(sender.identity()).unwrap();
    let recipient_identity = foreign_block_on(recipient.identity()).unwrap();
    for work in foreign_block_on(sender.enrollment_work(now())).unwrap() {
        if work.recipient != recipient_identity {
            continue;
        }
        let response =
            foreign_block_on(recipient.enrollment_handle(sender_identity, work.frame, now()))
                .unwrap();
        foreign_block_on(sender.enrollment_complete(work.id, response, now())).unwrap();
    }
}

fn enroll_peer(owner: &Client, member: &Client, group_id: [u8; 32], request: u8) {
    let GroupActionResult::Invited { invitation_id } = foreign_block_on(run(
        owner,
        GroupActionInput::Invite {
            input: InviteToGroupInput {
                group_id,
                client_request_id: token(request),
                recipient_identity: foreign_block_on(member.identity()).unwrap().into_bytes(),
            },
        },
    ))
    .unwrap() else {
        panic!("invite");
    };
    exchange_enrollment(owner, member);
    foreign_block_on(run(
        member,
        GroupActionInput::Respond {
            input: RespondGroupInvitationInput {
                invitation_id,
                client_request_id: token(request + 1),
                accept: true,
            },
        },
    ))
    .unwrap();
    for _ in 0..3 {
        exchange_enrollment(member, owner);
        exchange_enrollment(owner, member);
    }
    assert_eq!(
        foreign_block_on(member.conversations()).unwrap()[0].lifecycle,
        prns_groups_runner_native::GroupLifecycle::Active
    );
}

#[test]
fn rejoin_waits_for_the_previous_memberships_saved_send_to_be_reviewed() {
    let root = tempfile::tempdir().unwrap();
    let owner = open(&root.path().join("owner"));
    let member_identity = IdentityHash::new([92; 16]);
    let member = open_as(&root.path().join("member"), member_identity);
    let owner_client = clients(&owner).unwrap().0;
    let (member_client, actions) = clients(&member).unwrap();
    let group_id = create(&owner_client, 121);
    enroll_peer(&owner_client, &member_client, group_id, 122);
    let saved_send = GroupActionInput::Send {
        input: SendGroupTextInput {
            group_id,
            client_request_id: token(124),
            text: "Must not cross a fresh membership".into(),
        },
    };
    let saved_id = crate::group_actions::request_id(&saved_send);
    foreign_block_on(actions.execute(Request::Prepare(saved_send))).unwrap();
    let current = foreign_block_on(owner_client.conversations())
        .unwrap()
        .remove(0);
    let endpoint = current
        .members
        .iter()
        .find(|m| !m.is_local)
        .unwrap()
        .endpoint;
    foreign_block_on(run(
        &owner_client,
        GroupActionInput::RemoveMember {
            input: RemoveGroupMemberInput {
                group_id,
                client_request_id: token(125),
                endpoint_id: endpoint.id.0,
                endpoint_incarnation: endpoint.incarnation.get(),
                expected_control_head: current.control_head.0,
            },
        },
    ))
    .unwrap();
    exchange_enrollment(&owner_client, &member_client);
    let GroupActionResult::Invited { invitation_id } = foreign_block_on(run(
        &owner_client,
        GroupActionInput::Invite {
            input: InviteToGroupInput {
                group_id,
                client_request_id: token(126),
                recipient_identity: member_identity.into_bytes(),
            },
        },
    ))
    .unwrap() else {
        panic!("fresh invite")
    };
    exchange_enrollment(&owner_client, &member_client);
    let response = GroupActionInput::Respond {
        input: RespondGroupInvitationInput {
            invitation_id,
            client_request_id: token(127),
            accept: true,
        },
    };
    assert!(matches!(
        foreign_block_on(actions.execute(Request::Prepare(response.clone()))),
        Err(Error::InvalidInput(_))
    ));
    let reviewed = ready(foreign_block_on(super::execute_with(
        &member,
        GroupActionReference {
            owner_identity: member_identity.into_bytes(),
            client_request_id: saved_id,
        },
    )));
    assert!(matches!(
        reviewed.state,
        GroupActionState::Completed {
            result: GroupActionResult::Rejected { .. }
        }
    ));
    foreign_block_on(actions.execute(Request::Remove {
        request_id: saved_id,
        reviewed_expiry: false,
    }))
    .unwrap();
    foreign_block_on(actions.execute(Request::Prepare(response))).unwrap();
    close(&member);
    close(&owner);
}

#[test]
fn member_removal_rejoin_history_and_leave_after_delete_use_public_actions() {
    use prns_groups_runner_native::GroupLifecycle;
    use prns_groups_service::{ClockReading, GroupId};
    let root = tempfile::tempdir().unwrap();
    let owner = open(&root.path().join("owner"));
    let member = open_as(&root.path().join("member"), IdentityHash::new([92; 16]));
    let owner_client = clients(&owner).unwrap().0;
    let member_client = clients(&member).unwrap().0;
    let group_id = create(&owner_client, 101);
    enroll_peer(&owner_client, &member_client, group_id, 102);
    let prior = foreign_block_on(member_client.conversations())
        .unwrap()
        .remove(0);
    let old_endpoint = prior.members.iter().find(|m| m.is_local).unwrap().endpoint;
    foreign_block_on(run(
        &member_client,
        GroupActionInput::Send {
            input: SendGroupTextInput {
                group_id,
                client_request_id: token(104),
                text: "Before removal".into(),
            },
        },
    ))
    .unwrap();
    let current = foreign_block_on(owner_client.conversations())
        .unwrap()
        .remove(0);
    assert_eq!(
        foreign_block_on(run(
            &owner_client,
            GroupActionInput::RemoveMember {
                input: RemoveGroupMemberInput {
                    group_id,
                    client_request_id: token(105),
                    endpoint_id: old_endpoint.id.0,
                    endpoint_incarnation: old_endpoint.incarnation.get(),
                    expected_control_head: current.control_head.0,
                },
            }
        ))
        .unwrap(),
        GroupActionResult::RemovedMember
    );
    exchange_enrollment(&owner_client, &member_client);
    assert_eq!(
        foreign_block_on(member_client.conversations()).unwrap()[0].lifecycle,
        GroupLifecycle::Removed
    );
    assert!(foreign_block_on(member_client.network_configs())
        .unwrap()
        .is_empty());
    let page = foreign_block_on(member_client.messages(
        GroupId(group_id),
        None,
        100,
        ClockReading::Unknown,
    ))
    .unwrap();
    assert_eq!(page.messages[0].text, "Before removal");
    enroll_peer(&owner_client, &member_client, group_id, 106);
    let current = foreign_block_on(member_client.conversations())
        .unwrap()
        .remove(0);
    assert_ne!(
        current
            .members
            .iter()
            .find(|m| m.is_local)
            .unwrap()
            .endpoint,
        old_endpoint
    );
    foreign_block_on(run(
        &member_client,
        GroupActionInput::Send {
            input: SendGroupTextInput {
                group_id,
                client_request_id: token(108),
                text: "After rejoin".into(),
            },
        },
    ))
    .unwrap();
    let page =
        foreign_block_on(member_client.messages(GroupId(group_id), None, 1, ClockReading::Unknown))
            .unwrap();
    assert_eq!(page.messages[0].text, "After rejoin");
    let older = foreign_block_on(member_client.messages(
        GroupId(group_id),
        page.next_before_sequence,
        1,
        ClockReading::Unknown,
    ))
    .unwrap();
    assert_eq!(older.messages[0].text, "Before removal");
    assert_eq!(
        foreign_block_on(run(
            &member_client,
            GroupActionInput::LeaveGroup {
                input: LeaveGroupInput {
                    group_id,
                    client_request_id: token(109)
                },
            }
        ))
        .unwrap(),
        GroupActionResult::LeftGroup
    );
    assert_eq!(
        foreign_block_on(member_client.conversations()).unwrap()[0].lifecycle,
        GroupLifecycle::Leaving
    );
    foreign_block_on(run(
        &member_client,
        GroupActionInput::DeleteLocalGroup {
            input: DeleteLocalGroupInput {
                group_id,
                client_request_id: token(110),
            },
        },
    ))
    .unwrap();
    assert!(foreign_block_on(member_client.conversations())
        .unwrap()
        .is_empty());
    drop(member_client);
    close(&member);
    let member = open_as(&root.path().join("member"), IdentityHash::new([92; 16]));
    let member_client = clients(&member).unwrap().0;
    exchange_enrollment(&member_client, &owner_client);
    let current = foreign_block_on(owner_client.conversations())
        .unwrap()
        .remove(0);
    assert_eq!(current.member_count, 1);
    assert!(foreign_block_on(member_client.conversations())
        .unwrap()
        .is_empty());
    drop(member_client);
    drop(owner_client);
    close(&member);
    close(&owner);
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
fn now() -> u64 {
    match SystemClock.now() {
        prns_groups_service::ClockReading::Trusted(now) => now.upper().0,
        _ => panic!("clock"),
    }
}
fn token(id: u8) -> [u8; 32] {
    prns_groups_runner_native::RequestToken::new(now(), [id; 20])
        .unwrap()
        .into_bytes()
}
async fn prepare_input(supervisor: &Supervisor, input: GroupActionInput) -> GroupActionOutcome {
    super::prepare_with(
        supervisor,
        PrepareGroupActionInput {
            owner_identity: *IDENTITY.as_bytes(),
            input,
        },
    )
    .await
}
async fn execute_input(supervisor: &Supervisor, id: [u8; 32]) -> GroupActionOutcome {
    super::execute_with(
        supervisor,
        GroupActionReference {
            owner_identity: *IDENTITY.as_bytes(),
            client_request_id: id,
        },
    )
    .await
}
fn ready(outcome: GroupActionOutcome) -> GroupAction {
    let GroupActionOutcome::Ready { action } = outcome else {
        panic!("unexpected outcome {outcome:?}")
    };
    action
}
fn list_with(supervisor: &Supervisor) -> Vec<GroupAction> {
    let Reply::List(actions) =
        foreign_block_on(clients(supervisor).unwrap().1.execute(Request::List)).unwrap()
    else {
        panic!("list")
    };
    actions
}

#[test]
fn close_and_delete_recover_exact_results_after_the_group_disappears() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let (groups, actions) = clients(&supervisor).unwrap();
    let GroupActionResult::Created { group_id } = foreign_block_on(run(
        &groups,
        GroupActionInput::Create {
            input: CreateGroupInput {
                client_request_id: token(81),
                title: "Management recovery".into(),
            },
        },
    ))
    .unwrap() else {
        panic!("create");
    };
    let conversation = foreign_block_on(groups.conversations()).unwrap().remove(0);
    let close_input = GroupActionInput::CloseGroup {
        input: CloseGroupInput {
            group_id,
            client_request_id: token(82),
            expected_control_head: conversation.control_head.0,
        },
    };
    ready(foreign_block_on(prepare_input(
        &supervisor,
        close_input.clone(),
    )));
    assert_eq!(
        foreign_block_on(run(&groups, close_input.clone())).unwrap(),
        GroupActionResult::ClosedGroup
    );
    drop(actions);
    drop(groups);
    close(&supervisor);
    let supervisor = open(root.path());
    let close_id = crate::group_actions::request_id(&close_input);
    let closed = ready(foreign_block_on(execute_input(&supervisor, close_id)));
    assert_eq!(
        closed.state,
        GroupActionState::Completed {
            result: GroupActionResult::ClosedGroup
        }
    );
    let (groups, actions) = clients(&supervisor).unwrap();
    foreign_block_on(actions.execute(Request::Remove {
        request_id: close_id,
        reviewed_expiry: false,
    }))
    .unwrap();
    let delete_input = GroupActionInput::DeleteLocalGroup {
        input: DeleteLocalGroupInput {
            group_id,
            client_request_id: token(83),
        },
    };
    ready(foreign_block_on(prepare_input(
        &supervisor,
        delete_input.clone(),
    )));
    assert_eq!(
        foreign_block_on(run(&groups, delete_input.clone())).unwrap(),
        GroupActionResult::DeletedLocalGroup
    );
    assert!(foreign_block_on(groups.conversations()).unwrap().is_empty());
    drop(actions);
    drop(groups);
    close(&supervisor);
    let supervisor = open(root.path());
    let deleted = ready(foreign_block_on(execute_input(
        &supervisor,
        crate::group_actions::request_id(&delete_input),
    )));
    assert_eq!(deleted.input, delete_input);
    assert_eq!(
        deleted.state,
        GroupActionState::Completed {
            result: GroupActionResult::DeletedLocalGroup
        }
    );
    assert!(
        foreign_block_on(clients(&supervisor).unwrap().0.conversations())
            .unwrap()
            .is_empty()
    );
    close(&supervisor);
}

#[test]
fn typed_missing_invitation_rejection_survives_reopen_without_new_execution() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let id = token(31);
    let input = GroupActionInput::Respond {
        input: RespondGroupInvitationInput {
            client_request_id: id,
            invitation_id: [77; 32],
            accept: true,
        },
    };
    ready(foreign_block_on(prepare_input(&supervisor, input.clone())));
    let rejected = ready(foreign_block_on(execute_input(&supervisor, id)));
    assert_eq!(rejected.input, input);
    assert!(matches!(
        rejected.state,
        GroupActionState::Completed {
            result: GroupActionResult::Rejected {
                reason: GroupRejectionReason::NotFound,
                ..
            }
        }
    ));
    close(&supervisor);
    let reopened = open(root.path());
    assert_eq!(list_with(&reopened), vec![rejected.clone()]);
    assert_eq!(
        ready(foreign_block_on(execute_input(&reopened, id))),
        rejected
    );
    assert!(
        foreign_block_on(clients(&reopened).unwrap().0.conversations())
            .unwrap()
            .is_empty()
    );
    close(&reopened);
}

#[test]
fn ambiguous_failures_are_never_completed_rejections() {
    use prns_groups_runner_native::{enrollment::EnrollmentError, Error as RunnerError};
    for error in [
        RunnerError::Unavailable,
        RunnerError::Busy,
        RunnerError::StorageFull,
        RunnerError::Enrollment(EnrollmentError::Corrupt),
        RunnerError::Enrollment(EnrollmentError::Crypto),
    ] {
        assert!(!is_rejected(&error));
    }
    for (error, reason) in [
        (
            EnrollmentError::Expired,
            GroupRejectionReason::InvitationExpired,
        ),
        (EnrollmentError::NotOwner, GroupRejectionReason::NotOwner),
        (EnrollmentError::NotJoined, GroupRejectionReason::NotJoined),
        (
            EnrollmentError::Conflict,
            GroupRejectionReason::MembershipConflict,
        ),
        (
            EnrollmentError::RequestConflict,
            GroupRejectionReason::RequestConflict,
        ),
        (
            EnrollmentError::Authentication,
            GroupRejectionReason::AuthenticationFailed,
        ),
    ] {
        let error = RunnerError::Enrollment(error);
        assert!(is_rejected(&error));
        assert_eq!(super::super::groups::rejection_reason(&error), reason);
    }
}

#[test]
fn canceled_prepare_commits_without_executing_and_reopen_restores_exact_input() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let (entered, ready_lane) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .admit_test_barrier(entered, released)
        .unwrap();
    ready_lane.recv().unwrap();
    let input = GroupActionInput::Create {
        input: CreateGroupInput {
            client_request_id: token(1),
            title: "Recovered crew".into(),
        },
    };
    let mut pending = Box::pin(prepare_input(&supervisor, input.clone()));
    assert!(pending
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    drop(pending);
    release.send(()).unwrap();
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .drain_blocking()
        .unwrap();
    let old_client = clients(&supervisor).unwrap().1;
    assert!(
        foreign_block_on(clients(&supervisor).unwrap().0.conversations())
            .unwrap()
            .is_empty()
    );
    close(&supervisor);
    assert!(matches!(
        foreign_block_on(old_client.execute(Request::List)),
        Err(Error::Unavailable(_))
    ));
    let reopened = open(root.path());
    let saved = list_with(&reopened);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].input, input);
    let result = ready(foreign_block_on(execute_input(
        &reopened,
        crate::group_actions::request_id(&input),
    )));
    assert!(matches!(
        result.state,
        GroupActionState::Completed {
            result: GroupActionResult::Created { .. }
        }
    ));
    assert_eq!(list_with(&reopened), vec![result.clone()]);
    assert_eq!(
        ready(foreign_block_on(execute_input(
            &reopened,
            crate::group_actions::request_id(&input)
        ))),
        result
    );
    close(&reopened);
}

#[test]
fn stop_after_read_admission_preserves_pending_action_without_executing() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let input = GroupActionInput::Create {
        input: CreateGroupInput {
            client_request_id: token(6),
            title: "Stop fence".into(),
        },
    };
    ready(foreign_block_on(prepare_input(&supervisor, input.clone())));
    let (entered, ready_lane) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .admit_test_barrier(entered, released)
        .unwrap();
    ready_lane.recv().unwrap();
    let mut execution = Box::pin(execute_input(
        &supervisor,
        crate::group_actions::request_id(&input),
    ));
    assert!(execution
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    supervisor
        .snapshots
        .set_runtime(DevelopmentNodeRuntime::Stopping);
    release.send(()).unwrap();
    assert_eq!(foreign_block_on(execution), GroupActionOutcome::Busy);
    supervisor
        .snapshots
        .set_runtime(DevelopmentNodeRuntime::Stopped);
    assert_eq!(list_with(&supervisor)[0].state, GroupActionState::Pending);
    assert!(
        foreign_block_on(clients(&supervisor).unwrap().0.conversations())
            .unwrap()
            .is_empty()
    );
    close(&supervisor);
}

#[test]
fn expiry_is_durable_and_ordinary_acknowledgement_cannot_discard_it() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let id = prns_groups_runner_native::RequestToken::new(now() - 30 * 86_400, [7; 20])
        .unwrap()
        .into_bytes();
    let input = GroupActionInput::Create {
        input: CreateGroupInput {
            client_request_id: id,
            title: "Expired intent".into(),
        },
    };
    ready(foreign_block_on(prepare_input(&supervisor, input)));
    assert_eq!(
        ready(foreign_block_on(execute_input(&supervisor, id))).state,
        GroupActionState::RetryExpired
    );
    close(&supervisor);
    let reopened = open(root.path());
    let actions = clients(&reopened).unwrap().1;
    assert!(matches!(
        foreign_block_on(actions.execute(Request::Remove {
            request_id: id,
            reviewed_expiry: false
        })),
        Err(Error::NeedsReview)
    ));
    assert_eq!(
        list_with(&reopened)[0].state,
        GroupActionState::RetryExpired
    );
    foreign_block_on(actions.execute(Request::Remove {
        request_id: id,
        reviewed_expiry: true,
    }))
    .unwrap();
    assert!(list_with(&reopened).is_empty());
    assert!(
        foreign_block_on(clients(&reopened).unwrap().0.conversations())
            .unwrap()
            .is_empty()
    );
    close(&reopened);
}

#[test]
fn competing_execution_is_busy_and_canceled_caller_releases_recovery_gate() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let input = GroupActionInput::Create {
        input: CreateGroupInput {
            client_request_id: token(8),
            title: "One recovery executor".into(),
        },
    };
    ready(foreign_block_on(prepare_input(&supervisor, input.clone())));
    let id = crate::group_actions::request_id(&input);
    let (entered, ready_lane) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    supervisor
        .lock_state()
        .application_owner
        .as_ref()
        .unwrap()
        .admit_test_barrier(entered, released)
        .unwrap();
    ready_lane.recv().unwrap();
    let mut first = Box::pin(execute_input(&supervisor, id));
    assert!(first
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    assert_eq!(
        foreign_block_on(execute_input(&supervisor, id)),
        GroupActionOutcome::Busy
    );
    drop(first);
    release.send(()).unwrap();
    let completed = ready(foreign_block_on(execute_input(&supervisor, id)));
    assert!(matches!(
        completed.state,
        GroupActionState::Completed {
            result: GroupActionResult::Created { .. }
        }
    ));
    assert_eq!(
        foreign_block_on(clients(&supervisor).unwrap().0.conversations())
            .unwrap()
            .len(),
        1
    );
    close(&supervisor);
}

#[test]
fn stale_caller_identity_cannot_prepare_execute_acknowledge_or_discard() {
    let root = tempfile::tempdir().unwrap();
    let supervisor = open(root.path());
    let input = GroupActionInput::Create {
        input: CreateGroupInput {
            client_request_id: token(9),
            title: "Identity owner".into(),
        },
    };
    let wrong = [90; 16];
    let reference = GroupActionReference {
        owner_identity: wrong,
        client_request_id: crate::group_actions::request_id(&input),
    };
    assert!(matches!(
        foreign_block_on(super::prepare_with(
            &supervisor,
            PrepareGroupActionInput {
                owner_identity: wrong,
                input: input.clone()
            }
        )),
        GroupActionOutcome::InvalidInput { .. }
    ));
    assert!(list_with(&supervisor).is_empty());
    ready(foreign_block_on(prepare_input(&supervisor, input.clone())));
    assert!(matches!(
        foreign_block_on(super::execute_with(&supervisor, reference.clone())),
        GroupActionOutcome::InvalidInput { .. }
    ));
    assert!(
        foreign_block_on(clients(&supervisor).unwrap().0.conversations())
            .unwrap()
            .is_empty()
    );
    let completed = ready(foreign_block_on(execute_input(
        &supervisor,
        reference.client_request_id,
    )));
    for reviewed in [false, true] {
        assert!(matches!(
            foreign_block_on(super::acknowledge_with(
                &supervisor,
                reference.clone(),
                reviewed
            )),
            GroupActionAcknowledgeOutcome::Unavailable { .. }
        ));
        assert_eq!(list_with(&supervisor), vec![completed.clone()]);
    }
    let correct = GroupActionReference {
        owner_identity: *IDENTITY.as_bytes(),
        ..reference
    };
    assert_eq!(
        foreign_block_on(super::acknowledge_with(&supervisor, correct, false)),
        GroupActionAcknowledgeOutcome::Removed
    );
    close(&supervisor);
}

/// Runs only when selected by the parent test. Exit skips database/owner drop,
/// approximating process death at actual durable transaction boundaries.
#[test]
fn crash_child() {
    let Some(root) = std::env::var_os("PRNS_GROUP_ACTION_CRASH_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let input: GroupActionInput =
        serde_json::from_slice(&std::fs::read(root.join("input.json")).unwrap()).unwrap();
    let stage = std::env::var("PRNS_GROUP_ACTION_CRASH_STAGE").unwrap();
    let supervisor = open(root);
    ready(foreign_block_on(prepare_input(&supervisor, input.clone())));
    if stage == "accepted" {
        // Accepted operation and UI result are separate transactions. Deliberately
        // lose the accepted reply before storing the UI result.
        foreign_block_on(run(&clients(&supervisor).unwrap().0, input.clone())).unwrap();
    } else if stage == "completed" {
        let result = ready(foreign_block_on(execute_input(
            &supervisor,
            crate::group_actions::request_id(&input),
        )));
        assert!(matches!(result.state, GroupActionState::Completed { .. }));
    }
    std::process::exit(73);
}

fn create(client: &Client, id: u8) -> [u8; 32] {
    let GroupActionResult::Created { group_id } = foreign_block_on(run(
        client,
        GroupActionInput::Create {
            input: CreateGroupInput {
                client_request_id: token(id),
                title: "Crash crew".into(),
            },
        },
    ))
    .unwrap() else {
        panic!("create")
    };
    group_id
}

fn setup(root: &Path, kind: &str) -> GroupActionInput {
    if kind == "send" {
        let fixtures = prns_groups_runner_native::fixture::generate(
            "Crash crew",
            &[IDENTITY, IdentityHash::new([92; 16])],
            prns_groups_service::Timestamp(now()),
        )
        .unwrap();
        std::fs::write(root.join("group-fixture.json"), &fixtures[0]).unwrap();
    }
    let supervisor = open(root);
    let client = clients(&supervisor).unwrap().0;
    let input = match kind {
        "create" => GroupActionInput::Create {
            input: CreateGroupInput {
                client_request_id: token(2),
                title: "New group survives".into(),
            },
        },
        "invite" => GroupActionInput::Invite {
            input: InviteToGroupInput {
                client_request_id: token(3),
                group_id: create(&client, 10),
                recipient_identity: [92; 16],
            },
        },
        "respond" => {
            // A real authenticated incoming offer, obtained through the same
            // enrollment handlers used by the network, without development data.
            let remote_root = root.join("inviter");
            std::fs::create_dir_all(&remote_root).unwrap();
            let remote =
                DevelopmentStoreOwner::open(&remote_root, &remote_root.join("application.redb"))
                    .unwrap();
            let remote_identity = IdentityHash::new([92; 16]);
            remote.initialize_groups_blocking(remote_identity).unwrap();
            let remote_client = remote.group_client();
            let group_id = create(&remote_client, 11);
            let GroupActionResult::Invited { invitation_id } = foreign_block_on(run(
                &remote_client,
                GroupActionInput::Invite {
                    input: InviteToGroupInput {
                        client_request_id: token(12),
                        group_id,
                        recipient_identity: *IDENTITY.as_bytes(),
                    },
                },
            ))
            .unwrap() else {
                panic!("invite")
            };
            let work = foreign_block_on(remote_client.enrollment_work(now()))
                .unwrap()
                .remove(0);
            let response =
                foreign_block_on(client.enrollment_handle(remote_identity, work.frame, now()))
                    .unwrap();
            foreign_block_on(remote_client.enrollment_complete(work.id, response, now())).unwrap();
            remote.close().unwrap();
            GroupActionInput::Respond {
                input: RespondGroupInvitationInput {
                    client_request_id: token(4),
                    invitation_id,
                    accept: true,
                },
            }
        }
        "send" => GroupActionInput::Send {
            input: SendGroupTextInput {
                client_request_id: token(5),
                group_id: foreign_block_on(client.network_config()).unwrap().group.0,
                text: "Exact recovered send".into(),
            },
        },
        _ => panic!("unknown kind"),
    };
    close(&supervisor);
    input
}

#[test]
fn actual_process_exit_replays_all_action_kinds_at_every_commit_boundary() {
    for kind in ["create", "invite", "respond", "send"] {
        for stage in ["prepared", "accepted", "completed"] {
            let root = tempfile::tempdir().unwrap();
            let input = setup(root.path(), kind);
            std::fs::write(
                root.path().join("input.json"),
                serde_json::to_vec(&input).unwrap(),
            )
            .unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "lifecycle::group_actions::tests::crash_child",
                    "--nocapture",
                ])
                .env("PRNS_GROUP_ACTION_CRASH_ROOT", root.path())
                .env("PRNS_GROUP_ACTION_CRASH_STAGE", stage)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(73),
                "{kind}/{stage}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let supervisor = open(root.path());
            let saved = list_with(&supervisor);
            assert_eq!(saved.len(), 1, "{kind}/{stage}");
            assert_eq!(saved[0].input, input);
            assert_eq!(
                matches!(saved[0].state, GroupActionState::Completed { .. }),
                stage == "completed"
            );
            let id = crate::group_actions::request_id(&input);
            let completed = ready(foreign_block_on(execute_input(&supervisor, id)));
            assert!(
                matches!(completed.state, GroupActionState::Completed { ref result } if !matches!(result, GroupActionResult::Rejected { .. })),
                "{kind}/{stage}: {completed:?}"
            );
            assert_eq!(
                ready(foreign_block_on(execute_input(&supervisor, id))),
                completed
            );
            let (client, actions) = clients(&supervisor).unwrap();
            // Independently replay the runner request: it must return the same
            // durable result even when our UI result had never been recorded.
            assert_eq!(
                completed.state,
                GroupActionState::Completed {
                    result: foreign_block_on(run(&client, input.clone())).unwrap()
                }
            );
            if let GroupActionInput::Send { input } = &input {
                let page = foreign_block_on(client.messages(
                    prns_groups_service::GroupId(input.group_id),
                    None,
                    50,
                    SystemClock.now(),
                ))
                .unwrap();
                assert_eq!(page.messages.len(), 1);
                assert_eq!(page.messages[0].client_request_id, Some(id));
            }
            foreign_block_on(actions.execute(Request::Remove {
                request_id: id,
                reviewed_expiry: false,
            }))
            .unwrap();
            assert!(list_with(&supervisor).is_empty());
            close(&supervisor);
        }
    }
}
