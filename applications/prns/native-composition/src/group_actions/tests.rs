#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::contract::{CreateGroupInput, SendGroupTextInput};

fn input(id: u8, group: u8) -> GroupActionInput {
    GroupActionInput::Send {
        input: SendGroupTextInput {
            group_id: [group; 32],
            client_request_id: prns_groups_runner_native::RequestToken::new(
                1_800_000_000,
                [id; 20],
            )
            .unwrap()
            .into_bytes(),
            text: "Recover exactly once".into(),
        },
    }
}
fn owner(id: u8) -> Option<prns_host::IdentityHash> {
    Some(prns_host::IdentityHash::new([id; 16]))
}

#[test]
fn destructive_actions_cannot_bypass_an_unresolved_send_in_the_same_group() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::create(directory.path().join("management-actions.redb")).unwrap();
    execute(&db, owner(1), Request::Prepare(input(11, 1))).unwrap();
    let delete = GroupActionInput::DeleteLocalGroup {
        input: crate::contract::DeleteLocalGroupInput {
            group_id: [1; 32],
            client_request_id: request_id(&input(12, 1)),
        },
    };
    assert!(matches!(
        execute(&db, owner(1), Request::Prepare(delete.clone())),
        Err(Error::InvalidInput(_))
    ));
    // An independent named identity's group is a different action scope.
    assert!(matches!(
        execute(&db, owner(2), Request::Prepare(delete)),
        Ok(Reply::Action(_))
    ));
}

#[test]
fn fresh_invitation_and_saved_send_share_a_scope_in_both_admission_orders() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::create(directory.path().join("rejoin-actions.redb")).unwrap();
    let response = GroupActionInput::Respond {
        input: crate::contract::RespondGroupInvitationInput {
            invitation_id: [99; 32],
            client_request_id: request_id(&input(13, 1)),
            accept: true,
        },
    };
    let resolve = |id| Ok((id == [99; 32]).then_some([1; 32]));
    execute_scoped(&db, owner(1), Request::Prepare(input(11, 1)), resolve).unwrap();
    assert!(matches!(
        execute_scoped(&db, owner(1), Request::Prepare(response.clone()), resolve),
        Err(Error::InvalidInput(_))
    ));
    // A different identity has an independent journal. The reverse admission
    // order also blocks sending until the saved invitation response is resolved.
    execute_scoped(&db, owner(2), Request::Prepare(response), resolve).unwrap();
    assert!(matches!(
        execute_scoped(&db, owner(2), Request::Prepare(input(12, 1)), resolve),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        execute_scoped(&db, owner(2), Request::Prepare(input(14, 2)), resolve),
        Ok(Reply::Action(_))
    ));
}
fn action(reply: Reply) -> GroupAction {
    let Reply::Action(action) = reply else {
        panic!("missing action")
    };
    action
}

#[test]
fn legacy_rejected_action_reopens_without_changing_its_saved_input_or_result() {
    use crate::contract::GroupRejectionReason;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-actions.redb");
    let db = Database::create(&path).unwrap();
    let input = input(1, 1);
    let saved = GroupAction {
        owner_identity: [1; 16],
        input: input.clone(),
        state: GroupActionState::Completed {
            result: GroupActionResult::Rejected {
                reason: GroupRejectionReason::Unknown,
                detail: "Original rejected action".into(),
            },
        },
    };
    let mut old = serde_json::to_value(&saved).unwrap();
    old["state"]["Completed"]["result"]["Rejected"]
        .as_object_mut()
        .unwrap()
        .remove("reason");
    let bytes = serde_json::to_vec(&old).unwrap();
    let tx = db.begin_write().unwrap();
    {
        let mut table = tx.open_table(ACTIONS).unwrap();
        table
            .insert(
                key([1; 16], request_id(&input)).as_slice(),
                bytes.as_slice(),
            )
            .unwrap();
    }
    tx.commit().unwrap();
    drop(db);
    let db = Database::open(&path).unwrap();
    let read = action(execute(&db, owner(1), Request::Read(request_id(&input))).unwrap());
    assert_eq!(read, saved);
    let repeated = action(execute(&db, owner(1), Request::Prepare(input)).unwrap());
    assert_eq!(repeated, saved);
}

#[test]
fn immutable_inputs_scope_admission_and_owner_filter_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("actions.redb");
    let original = input(1, 1);
    let db = Database::create(&path).unwrap();
    let prepared = action(execute(&db, owner(1), Request::Prepare(original.clone())).unwrap());
    assert_eq!(prepared.state, GroupActionState::Pending);
    assert_eq!(prepared.owner_identity, [1; 16]);
    assert_eq!(
        action(execute(&db, owner(1), Request::Prepare(original.clone())).unwrap()),
        prepared
    );
    assert!(matches!(
        execute(&db, owner(1), Request::Prepare(input(2, 1))),
        Err(Error::InvalidInput(_))
    ));
    let mut changed = original.clone();
    if let GroupActionInput::Send { input } = &mut changed {
        input.text = "different".into();
    }
    assert!(matches!(
        execute(&db, owner(1), Request::Prepare(changed)),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        execute(
            &db,
            owner(1),
            Request::Remove {
                request_id: request_id(&original),
                reviewed_expiry: true
            }
        ),
        Err(Error::NeedsReview)
    ));
    drop(db);
    let db = Database::create(&path).unwrap();
    assert_eq!(
        action(execute(&db, owner(1), Request::Read(request_id(&original))).unwrap()),
        prepared
    );
    assert!(
        matches!(execute(&db, owner(2), Request::List).unwrap(), Reply::List(actions) if actions.is_empty())
    );
    assert!(matches!(
        execute(&db, owner(2), Request::Read(request_id(&original))),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        execute(&db, None, Request::List),
        Err(Error::Unavailable(_))
    ));
}

#[test]
fn bounded_actions_require_confirmation_and_expiry_review_before_retirement() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::create(directory.path().join("actions.redb")).unwrap();
    for id in 1..=MAX_ACTIONS {
        execute(&db, owner(1), Request::Prepare(input(id as u8, id as u8))).unwrap();
    }
    assert!(matches!(
        execute(&db, owner(1), Request::Prepare(input(99, 99))),
        Err(Error::StorageFull)
    ));
    // Retrying an existing row still succeeds when full.
    execute(&db, owner(1), Request::Prepare(input(1, 1))).unwrap();
    let id = request_id(&input(1, 1));
    let finish = GroupActionState::Completed {
        result: GroupActionResult::Sent {
            message_id: [7; 32],
        },
    };
    execute(
        &db,
        owner(1),
        Request::Finish {
            request_id: id,
            state: finish.clone(),
        },
    )
    .unwrap();
    // A late expired replay cannot erase a confirmed success.
    assert_eq!(
        action(
            execute(
                &db,
                owner(1),
                Request::Finish {
                    request_id: id,
                    state: GroupActionState::RetryExpired
                }
            )
            .unwrap()
        )
        .state,
        finish
    );
    execute(
        &db,
        owner(1),
        Request::Remove {
            request_id: id,
            reviewed_expiry: false,
        },
    )
    .unwrap();
    execute(
        &db,
        owner(1),
        Request::Remove {
            request_id: id,
            reviewed_expiry: false,
        },
    )
    .unwrap();
    execute(&db, owner(1), Request::Prepare(input(99, 99))).unwrap();
    let expired = request_id(&input(2, 2));
    execute(
        &db,
        owner(1),
        Request::Finish {
            request_id: expired,
            state: GroupActionState::RetryExpired,
        },
    )
    .unwrap();
    assert!(matches!(
        execute(
            &db,
            owner(1),
            Request::Remove {
                request_id: expired,
                reviewed_expiry: false
            }
        ),
        Err(Error::NeedsReview)
    ));
    execute(
        &db,
        owner(1),
        Request::Remove {
            request_id: expired,
            reviewed_expiry: true,
        },
    )
    .unwrap();
}

#[test]
fn rejects_oversized_or_conflicting_inputs_before_any_record_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::create(directory.path().join("actions.redb")).unwrap();
    let mut oversized = input(1, 1);
    if let GroupActionInput::Send { input } = &mut oversized {
        input.text = "é".repeat(1025);
    }
    assert!(matches!(
        execute(&db, owner(1), Request::Prepare(oversized)),
        Err(Error::InvalidInput(_))
    ));
    let title = GroupActionInput::Create {
        input: CreateGroupInput {
            client_request_id: request_id(&input(2, 2)),
            title: "x".repeat(129),
        },
    };
    assert!(matches!(
        execute(&db, owner(1), Request::Prepare(title)),
        Err(Error::InvalidInput(_))
    ));
    assert!(read_all(&db).unwrap().is_empty());
    execute(&db, owner(1), Request::Prepare(input(1, 1))).unwrap();
    assert!(matches!(
        execute(
            &db,
            owner(1),
            Request::Finish {
                request_id: request_id(&input(1, 1)),
                state: GroupActionState::Completed {
                    result: GroupActionResult::Responded
                }
            }
        ),
        Err(Error::Unavailable(_))
    ));
    assert_eq!(read_all(&db).unwrap()[0].state, GroupActionState::Pending);
}
