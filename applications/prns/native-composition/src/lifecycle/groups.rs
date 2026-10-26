use super::{admission, supervisor, Supervisor, SupervisorState};
use crate::contract::*;
use prns_groups_runner_native::{Client, Clock, Error, SendText, SystemClock};
use prns_groups_service::GroupId;

pub(crate) mod progress;

pub(super) fn rejection_reason(error: &Error) -> GroupRejectionReason {
    use prns_groups_runner_native::RejectionReason as Reason;
    match error.rejection_reason() {
        Some(Reason::InvalidInput) => GroupRejectionReason::InvalidInput,
        Some(Reason::NoRecipients) => GroupRejectionReason::NoRecipients,
        Some(Reason::RequestConflict) => GroupRejectionReason::RequestConflict,
        Some(Reason::InvitationExpired) => GroupRejectionReason::InvitationExpired,
        Some(Reason::NotFound) => GroupRejectionReason::NotFound,
        Some(Reason::MembershipConflict) => GroupRejectionReason::MembershipConflict,
        Some(Reason::NotOwner) => GroupRejectionReason::NotOwner,
        Some(Reason::NotJoined) => GroupRejectionReason::NotJoined,
        Some(Reason::AuthenticationFailed) => GroupRejectionReason::AuthenticationFailed,
        Some(Reason::AlreadyJoined) => GroupRejectionReason::AlreadyJoined,
        Some(Reason::GroupClosed) => GroupRejectionReason::GroupClosed,
        Some(Reason::OwnerMustClose) => GroupRejectionReason::OwnerMustClose,
        Some(Reason::MemberNotFound) => GroupRejectionReason::MemberNotFound,
        Some(Reason::StaleMembership) => GroupRejectionReason::StaleMembership,
        Some(Reason::UnsupportedVersion) => GroupRejectionReason::UnsupportedVersion,
        None => GroupRejectionReason::Unknown,
    }
}

async fn bounded_query<T>(
    query: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::select! {
        result = query => result,
        () = futures_timer::Delay::new(std::time::Duration::from_secs(5)) => Err(Error::Unavailable),
    }
}

fn client(supervisor: &Supervisor) -> Result<Client, Error> {
    let state = admission::try_state(supervisor).map_err(|_| Error::Busy)?;
    client_locked(supervisor, &state)
}

fn client_locked(supervisor: &Supervisor, state: &SupervisorState) -> Result<Client, Error> {
    if supervisor.snapshots.is_explicit_stop_in_progress()
        || matches!(
            supervisor.snapshots.read().runtime,
            DevelopmentNodeRuntime::Starting | DevelopmentNodeRuntime::Stopping
        )
    {
        return Err(Error::Busy);
    }
    state
        .application_owner
        .as_ref()
        .map(|owner| owner.group_client())
        .ok_or(Error::Unavailable)
}

pub(crate) async fn list_conversations() -> GroupConversationListOutcome {
    conversations_with_supervisor(supervisor()).await
}

async fn conversations_with_supervisor(supervisor: &Supervisor) -> GroupConversationListOutcome {
    let result = async {
        let (client, health) = {
            let state = admission::try_state(supervisor).map_err(|_| Error::Busy)?;
            let client = client_locked(supervisor, &state)?;
            let health = state
                .application_owner
                .as_ref()
                .ok_or(Error::Unavailable)?
                .group_health();
            (client, health)
        };
        bounded_query(client.conversations())
            .await
            .map(|conversations| (conversations, health.read(), client))
    }
    .await;
    match result {
        Ok((conversations, synchronization, client)) => GroupConversationListOutcome::Ready {
            conversations: conversations
                .into_iter()
                .map(|conversation| GroupConversation {
                    progress: progress::project(
                        client.synchronization_progress(conversation.group),
                        &synchronization,
                    ),
                    owner_identity: conversation
                        .owner_identity
                        .map(|identity| *identity.as_bytes()),
                    can_invite: conversation.can_invite,
                    lifecycle: match conversation.lifecycle {
                        prns_groups_runner_native::GroupLifecycle::Active => GroupLifecycle::Active,
                        prns_groups_runner_native::GroupLifecycle::Leaving => {
                            GroupLifecycle::Leaving
                        }
                        prns_groups_runner_native::GroupLifecycle::Removed => {
                            GroupLifecycle::Removed
                        }
                        prns_groups_runner_native::GroupLifecycle::Closed => GroupLifecycle::Closed,
                    },
                    can_send: conversation.can_send,
                    can_remove_members: conversation.can_remove_members,
                    can_leave: conversation.can_leave,
                    can_close: conversation.can_close,
                    can_delete_local: conversation.can_delete_local,
                    group_id: conversation.group.0,
                    title: conversation.title,
                    revision: conversation.revision,
                    control_head: conversation.control_head.0,
                    members: conversation
                        .members
                        .into_iter()
                        .map(|member| GroupMember {
                            endpoint_id: member.endpoint.id.0,
                            endpoint_incarnation: member.endpoint.incarnation.get(),
                            identity: *member.identity.as_bytes(),
                            is_local: member.is_local,
                            is_owner: member.is_owner,
                        })
                        .collect(),
                    member_count: conversation.member_count as u32,
                    synchronization: synchronization.clone(),
                })
                .collect(),
        },
        Err(Error::NotProvisioned) => GroupConversationListOutcome::NotProvisioned,
        Err(Error::Busy) => GroupConversationListOutcome::Busy,
        Err(error) => GroupConversationListOutcome::Unavailable {
            detail: error.to_string(),
        },
    }
}

pub(crate) async fn list_messages(input: ListGroupMessagesInput) -> GroupMessageListOutcome {
    let result = async {
        bounded_query(client(supervisor())?.messages(
            GroupId(input.group_id),
            input.before_sequence,
            input.limit as usize,
            SystemClock.now(),
        ))
        .await
    }
    .await;
    match result {
        Ok(page) => GroupMessageListOutcome::Ready {
            revision: page.revision,
            next_before_sequence: page.next_before_sequence,
            messages: page
                .messages
                .into_iter()
                .map(|message| GroupTextMessage {
                    membership_changed: message.membership_changed,
                    message_id: message.id.0,
                    client_request_id: message.client_request_id,
                    sequence: message.sequence,
                    author: *message.author.as_bytes(),
                    text: message.text,
                    outgoing: message.outgoing,
                    received_count: message.received_count as u32,
                    recipient_count: message.recipient_count as u32,
                    sent_at_millis: message.sent_at_millis,
                })
                .collect(),
        },
        Err(Error::NotProvisioned) => GroupMessageListOutcome::NotProvisioned,
        Err(Error::Busy) => GroupMessageListOutcome::Busy,
        Err(Error::InvalidInput | Error::IdempotencyConflict) => {
            GroupMessageListOutcome::InvalidInput {
                detail: "The group or history page is invalid.".into(),
            }
        }
        Err(error) => GroupMessageListOutcome::Unavailable {
            detail: error.to_string(),
        },
    }
}

pub(crate) async fn send_text(input: SendGroupTextInput) -> GroupTextSendOutcome {
    send_with_supervisor(supervisor(), input).await
}

async fn send_with_supervisor(
    supervisor: &Supervisor,
    input: SendGroupTextInput,
) -> GroupTextSendOutcome {
    let result = async {
        let waiter = {
            let state = admission::try_state(supervisor).map_err(|_| Error::Busy)?;
            client_locked(supervisor, &state)?.admit_send_text(SendText {
                group: GroupId(input.group_id),
                client_request_id: input.client_request_id,
                text: input.text,
                clock: SystemClock.now(),
            })?
        };
        waiter.await
    }
    .await;
    match result {
        Ok(record) => GroupTextSendOutcome::Accepted {
            message_id: record.0,
        },
        Err(Error::NotProvisioned) => GroupTextSendOutcome::NotProvisioned,
        Err(Error::Busy) => GroupTextSendOutcome::Busy,
        Err(error) if error.is_retry_expired() => GroupTextSendOutcome::RetryExpired,
        Err(error) if error.is_storage_full() => GroupTextSendOutcome::StorageFull,
        Err(Error::NoRecipients) => GroupTextSendOutcome::InvalidInput {
            reason: GroupRejectionReason::NoRecipients,
            detail: "Invite at least one device before sending a group message.".into(),
        },
        Err(Error::InvalidInput) => GroupTextSendOutcome::InvalidInput {
            reason: GroupRejectionReason::InvalidInput,
            detail: "A group text must contain 1 to 2048 UTF-8 bytes.".into(),
        },
        Err(Error::IdempotencyConflict) => GroupTextSendOutcome::InvalidInput {
            reason: GroupRejectionReason::RequestConflict,
            detail: "This send identifier already belongs to a different message.".into(),
        },
        Err(error) => GroupTextSendOutcome::Unavailable {
            detail: error.to_string(),
        },
    }
}

fn now_secs() -> Result<u64, Error> {
    match SystemClock.now() {
        prns_groups_service::ClockReading::Trusted(interval) => Ok(interval.upper().0),
        _ => Err(Error::Clock),
    }
}
async fn enroll(
    request: prns_groups_runner_native::EnrollmentRequest,
) -> Result<prns_groups_runner_native::EnrollmentResult, Error> {
    enroll_with_supervisor(supervisor(), request).await
}
async fn enroll_with_supervisor(
    supervisor: &Supervisor,
    request: prns_groups_runner_native::EnrollmentRequest,
) -> Result<prns_groups_runner_native::EnrollmentResult, Error> {
    let waiter = {
        let state = admission::try_state(supervisor).map_err(|_| Error::Busy)?;
        client_locked(supervisor, &state)?.admit_enrollment(request)?
    };
    waiter.await
}
pub(crate) async fn create_group(input: CreateGroupInput) -> CreateGroupOutcome {
    create_with_supervisor(supervisor(), input).await
}
async fn create_with_supervisor(
    supervisor: &Supervisor,
    input: CreateGroupInput,
) -> CreateGroupOutcome {
    let result = async {
        enroll_with_supervisor(
            supervisor,
            prns_groups_runner_native::EnrollmentRequest::Create {
                request_id: input.client_request_id,
                title: input.title,
                now: now_secs()?,
            },
        )
        .await
    }
    .await;
    match result {
        Ok(prns_groups_runner_native::EnrollmentResult::Created(group)) => {
            CreateGroupOutcome::Accepted { group_id: group.0 }
        }
        Err(Error::Busy) => CreateGroupOutcome::Busy,
        Err(error) if error.is_retry_expired() => CreateGroupOutcome::RetryExpired,
        Err(error) if error.is_storage_full() => CreateGroupOutcome::StorageFull,
        Err(error @ (Error::InvalidInput | Error::IdempotencyConflict)) => {
            CreateGroupOutcome::InvalidInput {
                reason: rejection_reason(&error),
                detail: "The group title or request identifier is invalid.".into(),
            }
        }
        Err(Error::Enrollment(error)) if error.is_invalid_input() => {
            CreateGroupOutcome::InvalidInput {
                reason: rejection_reason(&Error::Enrollment(error)),
                detail: "The group or request no longer permits this action.".into(),
            }
        }
        Err(error) => CreateGroupOutcome::Unavailable {
            detail: error.to_string(),
        },
        _ => CreateGroupOutcome::Unavailable {
            detail: "Unexpected enrollment result.".into(),
        },
    }
}
pub(crate) async fn invite_to_group(input: InviteToGroupInput) -> InviteToGroupOutcome {
    let result = async {
        enroll(prns_groups_runner_native::EnrollmentRequest::Invite {
            request_id: input.client_request_id,
            group: GroupId(input.group_id),
            recipient: prns_host::IdentityHash::new(input.recipient_identity),
            now: now_secs()?,
        })
        .await
    }
    .await;
    match result {
        Ok(prns_groups_runner_native::EnrollmentResult::Invited(id)) => {
            InviteToGroupOutcome::Queued { invitation_id: id }
        }
        Err(Error::Busy) => InviteToGroupOutcome::Busy,
        Err(error) if error.is_retry_expired() => InviteToGroupOutcome::RetryExpired,
        Err(error) if error.is_storage_full() => InviteToGroupOutcome::StorageFull,
        Err(error @ (Error::InvalidInput | Error::IdempotencyConflict)) => {
            InviteToGroupOutcome::InvalidInput {
                reason: rejection_reason(&error),
                detail: "The invitation or request identifier is invalid.".into(),
            }
        }
        Err(Error::Enrollment(error)) if error.is_invalid_input() => {
            InviteToGroupOutcome::InvalidInput {
                reason: rejection_reason(&Error::Enrollment(error)),
                detail: "The group or invitation no longer permits this action.".into(),
            }
        }
        Err(error) => InviteToGroupOutcome::Unavailable {
            detail: error.to_string(),
        },
        _ => InviteToGroupOutcome::Unavailable {
            detail: "Unexpected enrollment result.".into(),
        },
    }
}
pub(crate) async fn respond_group_invitation(
    input: RespondGroupInvitationInput,
) -> RespondGroupInvitationOutcome {
    let result = async {
        enroll(prns_groups_runner_native::EnrollmentRequest::Respond {
            request_id: input.client_request_id,
            invitation: input.invitation_id,
            accept: input.accept,
            now: now_secs()?,
        })
        .await
    }
    .await;
    match result {
        Ok(prns_groups_runner_native::EnrollmentResult::Saved) => {
            RespondGroupInvitationOutcome::Saved
        }
        Err(Error::Busy) => RespondGroupInvitationOutcome::Busy,
        Err(error) if error.is_retry_expired() => RespondGroupInvitationOutcome::RetryExpired,
        Err(error) if error.is_storage_full() => RespondGroupInvitationOutcome::StorageFull,
        Err(error @ (Error::InvalidInput | Error::IdempotencyConflict)) => {
            RespondGroupInvitationOutcome::InvalidInput {
                reason: rejection_reason(&error),
                detail: "The invitation decision or request identifier is invalid.".into(),
            }
        }
        Err(Error::Enrollment(error)) if error.is_invalid_input() => {
            RespondGroupInvitationOutcome::InvalidInput {
                reason: rejection_reason(&Error::Enrollment(error)),
                detail: "The invitation no longer permits this action.".into(),
            }
        }
        Err(error) => RespondGroupInvitationOutcome::Unavailable {
            detail: error.to_string(),
        },
        _ => RespondGroupInvitationOutcome::Unavailable {
            detail: "Unexpected enrollment result.".into(),
        },
    }
}
pub(crate) async fn list_group_invitations() -> GroupInvitationListOutcome {
    use prns_groups_runner_native::enrollment::InvitationState as State;
    let result =
        async { bounded_query(client(supervisor())?.invitations(now_secs()?)).await }.await;
    match result {
        Ok(invitations) => GroupInvitationListOutcome::Ready {
            invitations: invitations
                .into_iter()
                .map(|invitation| GroupInvitation {
                    invitation_id: invitation.id,
                    group_id: invitation.group.0,
                    title: invitation.title,
                    inviter_identity: *invitation.inviter.as_bytes(),
                    recipient_identity: *invitation.recipient.as_bytes(),
                    expires_at_millis: invitation.expires_at_secs.saturating_mul(1000),
                    outgoing: invitation.outgoing,
                    waiting_for_delivery: invitation.waiting_for_delivery,
                    can_cancel: invitation.can_cancel,
                    can_dismiss: invitation.can_dismiss,
                    state: match invitation.state {
                        State::Pending => GroupInvitationState::Offered,
                        State::JoinRequested => GroupInvitationState::Joining,
                        State::Joined => GroupInvitationState::Joined,
                        State::Declined => GroupInvitationState::Declined,
                        State::Expired => GroupInvitationState::Expired,
                        State::Cancelled => GroupInvitationState::Cancelled,
                    },
                })
                .collect(),
        },
        Err(Error::Busy) => GroupInvitationListOutcome::Busy,
        Err(error) => GroupInvitationListOutcome::Unavailable {
            detail: error.to_string(),
        },
    }
}

#[cfg(test)]
mod enrollment_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::{
        development_store::DevelopmentStoreOwner, snapshot::SnapshotStore,
        test_support::foreign_block_on,
    };
    use std::{
        future::Future,
        sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
        task::{Context, Waker},
    };

    #[test]
    fn offline_stop_and_reopen_preserve_pending_send_after_foreign_caller_drop() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("prns/development");
        std::fs::create_dir_all(&root).unwrap();
        let identity = prns_host::IdentityHash::new([81; 16]);
        let now = match SystemClock.now() {
            prns_groups_service::ClockReading::Trusted(interval) => interval.upper(),
            prns_groups_service::ClockReading::Unknown => panic!("clock unavailable"),
        };
        let fixtures = prns_groups_runner_native::fixture::generate(
            "Foreign lifetime",
            &[identity, prns_host::IdentityHash::new([82; 16])],
            now,
        )
        .unwrap();
        std::fs::write(root.join("group-fixture.json"), &fixtures[0]).unwrap();
        let store = DevelopmentStoreOwner::open(&root, &root.join("application.redb")).unwrap();
        store.initialize_groups_blocking(identity).unwrap();
        let client = store.group_client();
        let group_id = foreign_block_on(client.network_config()).unwrap().group.0;
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        store.admit_test_barrier(entered_tx, release_rx).unwrap();
        entered_rx.recv().unwrap();
        let supervisor = Arc::new(Supervisor {
            snapshots: Arc::new(SnapshotStore::new()),
            operation_admitted: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(SupervisorState {
                application_owner: Some(store),
                ..SupervisorState::default()
            }),
        });
        let input = SendGroupTextInput {
            group_id,
            client_request_id: prns_groups_runner_native::RequestToken::new(now.0, [83; 20])
                .unwrap()
                .into_bytes(),
            text: "Native owns the accepted send".into(),
        };
        let mut caller = Box::pin(send_with_supervisor(&supervisor, input.clone()));
        assert!(caller
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        drop(caller);
        let stopping = Arc::clone(&supervisor);
        let (stop_entered, stop_ready) = mpsc::sync_channel(1);
        let (stop_reply, stop_result) = mpsc::sync_channel(1);
        let stopped = std::thread::spawn(move || {
            let mut state = stopping.lock_state();
            stop_entered.send(()).unwrap();
            stop_reply
                .send(super::super::stop_locked(&stopping, &mut state))
                .unwrap();
        });
        stop_ready.recv().unwrap();
        assert!(matches!(
            stop_result.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release_tx.send(()).unwrap();
        assert_eq!(
            stop_result.recv().unwrap(),
            DevelopmentNodeStopOutcome::AlreadyStopped
        );
        stopped.join().unwrap();
        let outcome = foreign_block_on(send_with_supervisor(&supervisor, input.clone()));
        let GroupTextSendOutcome::Accepted { message_id } = outcome else {
            panic!("The admitted send must recover its original result: {outcome:?}");
        };
        let conversations = foreign_block_on(conversations_with_supervisor(&supervisor));
        assert!(
            matches!(conversations, GroupConversationListOutcome::Ready { conversations } if conversations.len() == 1)
        );
        let page =
            foreign_block_on(client.messages(GroupId(group_id), None, 50, SystemClock.now()))
                .unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].id.0, message_id);
        assert_eq!(
            page.messages[0].client_request_id,
            Some(input.client_request_id)
        );
        assert_eq!(page.messages[0].received_count, 0);
        assert_eq!(page.messages[0].recipient_count, 1);
        supervisor
            .lock_state()
            .application_owner
            .take()
            .unwrap()
            .close()
            .unwrap();

        // No recipient was reachable before the owner closed. Reopen the real
        // application store, then reconcile the lost foreign result with the
        // same request. This must retain the original pending delivery rather
        // than publish another record or fabricate a receipt.
        let reopened = DevelopmentStoreOwner::open(&root, &root.join("application.redb")).unwrap();
        reopened.initialize_groups_blocking(identity).unwrap();
        let reopened_client = reopened.group_client();
        let reopened_supervisor = Supervisor {
            snapshots: Arc::new(SnapshotStore::new()),
            operation_admitted: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(SupervisorState {
                application_owner: Some(reopened),
                ..SupervisorState::default()
            }),
        };
        assert_eq!(
            foreign_block_on(send_with_supervisor(&reopened_supervisor, input)),
            GroupTextSendOutcome::Accepted { message_id }
        );
        let recovered = foreign_block_on(reopened_client.messages(
            GroupId(group_id),
            None,
            50,
            SystemClock.now(),
        ))
        .unwrap();
        assert_eq!(recovered.messages.len(), 1);
        assert_eq!(recovered.messages[0].id.0, message_id);
        assert_eq!(recovered.messages[0].received_count, 0);
        assert_eq!(recovered.messages[0].recipient_count, 1);
        reopened_supervisor
            .lock_state()
            .application_owner
            .take()
            .unwrap()
            .close()
            .unwrap();
    }
}
