//! Typed projections shared by the immutable named-owner bridge.
use crate::contract::*;
use crate::identity_services::MessagingIdentity;
use crate::lifecycle::groups::progress;
use prns_groups_runner_native::{Client, Clock, Error, SendText, SystemClock};
use prns_groups_service::GroupId;
fn client(services: &MessagingIdentity) -> Result<Client, Error> {
    services.groups().map_err(|_| Error::Unavailable)
}
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

pub(super) async fn conversations(services: &MessagingIdentity) -> GroupConversationListOutcome {
    let result = async {
        let (client, health) = {
            let client = client(services)?;
            let health = services
                .store()
                .map_err(|_| Error::Unavailable)?
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

pub(super) async fn list_messages(
    services: &MessagingIdentity,
    input: ListGroupMessagesInput,
) -> GroupMessageListOutcome {
    let result = async {
        bounded_query(client(services)?.messages(
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

pub(super) async fn send_text(
    services: &MessagingIdentity,
    input: SendGroupTextInput,
) -> GroupTextSendOutcome {
    let result = async {
        let waiter = {
            client(services)?.admit_send_text(SendText {
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
    services: &MessagingIdentity,
    request: prns_groups_runner_native::EnrollmentRequest,
) -> Result<prns_groups_runner_native::EnrollmentResult, Error> {
    let waiter = { client(services)?.admit_enrollment(request)? };
    waiter.await
}

pub(super) async fn create_group(
    services: &MessagingIdentity,
    input: CreateGroupInput,
) -> CreateGroupOutcome {
    let result = async {
        enroll(
            services,
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

pub(super) async fn invite_to_group(
    services: &MessagingIdentity,
    input: InviteToGroupInput,
) -> InviteToGroupOutcome {
    let result = async {
        enroll(
            services,
            prns_groups_runner_native::EnrollmentRequest::Invite {
                request_id: input.client_request_id,
                group: GroupId(input.group_id),
                recipient: prns_host::IdentityHash::new(input.recipient_identity),
                now: now_secs()?,
            },
        )
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

pub(super) async fn respond_group_invitation(
    services: &MessagingIdentity,
    input: RespondGroupInvitationInput,
) -> RespondGroupInvitationOutcome {
    let result = async {
        enroll(
            services,
            prns_groups_runner_native::EnrollmentRequest::Respond {
                request_id: input.client_request_id,
                invitation: input.invitation_id,
                accept: input.accept,
                now: now_secs()?,
            },
        )
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

pub(super) async fn list_group_invitations(
    services: &MessagingIdentity,
) -> GroupInvitationListOutcome {
    use prns_groups_runner_native::enrollment::InvitationState as State;
    let result = async { bounded_query(client(services)?.invitations(now_secs()?)).await }.await;
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
                    state: match invitation.state {
                        State::Pending => GroupInvitationState::Offered,
                        State::JoinRequested => GroupInvitationState::Joining,
                        State::Joined => GroupInvitationState::Joined,
                        State::Declined => GroupInvitationState::Declined,
                        State::Expired => GroupInvitationState::Expired,
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
