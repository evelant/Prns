use crate::{
    contract::*,
    development_store::GroupActionClient,
    group_actions::{Error, Reply, Request},
    identity_services::MessagingIdentity,
};
use prns_groups_runner_native::{
    Client, Clock, EnrollmentRequest, EnrollmentResult, SendText, SystemClock,
};
fn checked_clients(
    services: &MessagingIdentity,
    expected: [u8; 16],
) -> Result<(Client, GroupActionClient), Error> {
    let store = services
        .store()
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    let actions = store.group_action_client();
    if actions.identity()? != expected {
        return Err(Error::InvalidInput(
            "This action belongs to another identity.".into(),
        ));
    }
    Ok((store.group_client(), actions))
}
fn outcome(result: Result<Reply, Error>) -> GroupActionOutcome {
    match result {
        Ok(Reply::Action(action)) => GroupActionOutcome::Ready { action },
        Err(Error::Busy) => GroupActionOutcome::Busy,
        Err(Error::StorageFull) => GroupActionOutcome::StorageFull,
        Err(Error::InvalidInput(detail)) => GroupActionOutcome::InvalidInput { detail },
        Err(Error::Unavailable(detail)) => GroupActionOutcome::Unavailable { detail },
        _ => GroupActionOutcome::Unavailable {
            detail: "Unexpected group action reply.".into(),
        },
    }
}

pub(super) async fn prepare(
    services: &MessagingIdentity,
    input: PrepareGroupActionInput,
) -> GroupActionOutcome {
    outcome(
        async {
            let (_, actions) = checked_clients(services, input.owner_identity)?;
            actions.admit(Request::Prepare(input.input))?.await
        }
        .await,
    )
}

pub(super) async fn list(services: &MessagingIdentity) -> GroupActionListOutcome {
    let result = async {
        services
            .store()
            .map_err(|e| Error::Unavailable(e.to_string()))?
            .group_action_client()
            .admit(Request::List)?
            .await
    }
    .await;
    match result {
        Ok(Reply::List(actions)) => GroupActionListOutcome::Ready { actions },
        Err(Error::Busy) => GroupActionListOutcome::Busy,
        Err(Error::Unavailable(detail) | Error::InvalidInput(detail)) => {
            GroupActionListOutcome::Unavailable { detail }
        }
        _ => GroupActionListOutcome::Unavailable {
            detail: "The saved group actions could not be read.".into(),
        },
    }
}

/// Exact journal reads are permitted while paused. This path cannot compose a
/// message, enroll a member, or invoke a runner mutation.
pub(super) async fn read(
    services: &MessagingIdentity,
    reference: GroupActionReference,
) -> GroupActionOutcome {
    outcome(
        async {
            let (_, actions) = checked_clients(services, reference.owner_identity)?;
            actions
                .admit(Request::Read(reference.client_request_id))?
                .await
        }
        .await,
    )
}

pub(super) async fn execute(
    services: &MessagingIdentity,
    reference: GroupActionReference,
) -> GroupActionOutcome {
    outcome(
        async {
            // Capture both clients once. An old action cannot cross an identity
            // replacement or reopened owner while an asynchronous call is pending.
            let (groups, actions) = checked_clients(services, reference.owner_identity)?;
            let _execution = actions.start_execution()?;
            let request_id = reference.client_request_id;
            let Reply::Action(action) = actions.execute(Request::Read(request_id)).await? else {
                return Err(Error::Unavailable(
                    "The saved group action could not be read.".into(),
                ));
            };
            if !matches!(action.state, GroupActionState::Pending) {
                return Ok(Reply::Action(action));
            }
            let execution = admit_run(&groups, action.input).map_err(|error| match error {
                prns_groups_runner_native::Error::Busy => Error::Busy,
                error => Error::Unavailable(error.to_string()),
            })?;
            let result = execution.await;
            let state = match result {
                Ok(result) => GroupActionState::Completed { result },
                Err(error) if error.is_retry_expired() => GroupActionState::RetryExpired,
                Err(error) if error.is_storage_full() => return Err(Error::StorageFull),
                Err(prns_groups_runner_native::Error::Busy) => return Err(Error::Busy),
                Err(error) if is_rejected(&error) => GroupActionState::Completed {
                    result: GroupActionResult::Rejected {
                        reason: super::groups::rejection_reason(&error),
                        detail: rejection_detail(&error).into(),
                    },
                },
                Err(error) => return Err(Error::Unavailable(error.to_string())),
            };
            // If execution committed but this write/result is lost, Pending remains
            // and replay uses the exact same runner idempotency identifier.
            actions.execute(Request::Finish { request_id, state }).await
        }
        .await,
    )
}

fn is_rejected(error: &prns_groups_runner_native::Error) -> bool {
    error.rejection_reason().is_some()
}

fn rejection_detail(error: &prns_groups_runner_native::Error) -> &'static str {
    use prns_groups_runner_native::RejectionReason;
    match error.rejection_reason() {
        Some(RejectionReason::NoRecipients) => {
            "Invite at least one device before sending a group message."
        }
        Some(RejectionReason::RequestConflict) => {
            "This identifier already belongs to a different group action."
        }
        Some(RejectionReason::InvitationExpired) => {
            "This invitation has expired. Ask the owner for a new invitation."
        }
        Some(RejectionReason::NotFound) => {
            "This group or invitation is no longer saved on this device."
        }
        Some(RejectionReason::MembershipConflict) => {
            "Membership or invitation state no longer permits this action."
        }
        Some(RejectionReason::NotOwner) => "Only the group owner can invite devices.",
        Some(RejectionReason::NotJoined) => "Group membership has not finished synchronizing.",
        Some(RejectionReason::AuthenticationFailed) => {
            "The group or invitation could not be authenticated."
        }
        _ => "The saved group action contains invalid input.",
    }
}

fn admit_run(
    client: &Client,
    input: GroupActionInput,
) -> Result<Execution, prns_groups_runner_native::Error> {
    use prns_groups_runner_native::Error;
    if let GroupActionInput::Send { input } = input {
        let waiter = client.admit_send_text(SendText {
            group: prns_groups_service::GroupId(input.group_id),
            client_request_id: input.client_request_id,
            text: input.text,
            clock: SystemClock.now(),
        })?;
        return Ok(Box::pin(async move {
            Ok(GroupActionResult::Sent {
                message_id: waiter.await?.0,
            })
        }));
    }
    let prns_groups_service::ClockReading::Trusted(now) = SystemClock.now() else {
        return Err(Error::Clock);
    };
    let now = now.upper().0;
    let request = match input {
        GroupActionInput::Create { input } => EnrollmentRequest::Create {
            request_id: input.client_request_id,
            title: input.title,
            now,
        },
        GroupActionInput::Invite { input } => EnrollmentRequest::Invite {
            request_id: input.client_request_id,
            group: prns_groups_service::GroupId(input.group_id),
            recipient: prns_host::IdentityHash::new(input.recipient_identity),
            now,
        },
        GroupActionInput::Respond { input } => EnrollmentRequest::Respond {
            request_id: input.client_request_id,
            invitation: input.invitation_id,
            accept: input.accept,
            now,
        },
        GroupActionInput::Send { .. } => return Err(Error::InvalidInput),
    };
    let waiter = client.admit_enrollment(request)?;
    Ok(Box::pin(async move {
        match waiter.await? {
            EnrollmentResult::Created(group) => {
                Ok(GroupActionResult::Created { group_id: group.0 })
            }
            EnrollmentResult::Invited(invitation_id) => {
                Ok(GroupActionResult::Invited { invitation_id })
            }
            EnrollmentResult::Saved => Ok(GroupActionResult::Responded),
        }
    }))
}

pub(super) async fn acknowledge(
    services: &MessagingIdentity,
    reference: GroupActionReference,
    reviewed_expiry: bool,
) -> GroupActionAcknowledgeOutcome {
    let result = async {
        let (_, actions) = checked_clients(services, reference.owner_identity)?;
        actions
            .admit(Request::Remove {
                request_id: reference.client_request_id,
                reviewed_expiry,
            })?
            .await
    }
    .await;
    match result {
        Ok(Reply::Removed) => GroupActionAcknowledgeOutcome::Removed,
        Err(Error::NeedsReview) => GroupActionAcknowledgeOutcome::NeedsReview,
        Err(Error::Busy) => GroupActionAcknowledgeOutcome::Busy,
        Err(Error::Unavailable(detail) | Error::InvalidInput(detail)) => {
            GroupActionAcknowledgeOutcome::Unavailable { detail }
        }
        _ => GroupActionAcknowledgeOutcome::Unavailable {
            detail: "The saved group action could not be acknowledged.".into(),
        },
    }
}

type Execution = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<GroupActionResult, prns_groups_runner_native::Error>,
            > + Send,
    >,
>;
