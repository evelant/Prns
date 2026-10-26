//! Bridge recovery retains the original owner and retry input across all awaits.

use super::{admission, supervisor, Supervisor};
use crate::{
    contract::*,
    development_store::GroupActionClient,
    group_actions::{Error, Reply, Request},
};
use prns_groups_runner_native::{
    Client, Clock, EnrollmentRequest, EnrollmentResult, SendText, SystemClock,
};

#[cfg(test)]
fn clients(supervisor: &Supervisor) -> Result<(Client, GroupActionClient), Error> {
    with_clients(supervisor, |groups, actions| Ok((groups, actions)))
}

fn with_clients<T>(
    supervisor: &Supervisor,
    action: impl FnOnce(Client, GroupActionClient) -> Result<T, Error>,
) -> Result<T, Error> {
    let state = admission::try_state(supervisor).map_err(|_| Error::Busy)?;
    if supervisor.snapshots.is_explicit_stop_in_progress()
        || matches!(
            supervisor.snapshots.read().runtime,
            DevelopmentNodeRuntime::Starting | DevelopmentNodeRuntime::Stopping
        )
    {
        return Err(Error::Busy);
    }
    let owner = state.application_owner.as_ref().ok_or_else(|| {
        Error::Unavailable("Open the local identity to recover saved group actions.".into())
    })?;
    action(owner.group_client(), owner.group_action_client())
}

fn checked_clients(
    supervisor: &Supervisor,
    expected_identity: [u8; 16],
) -> Result<(Client, GroupActionClient), Error> {
    with_clients(supervisor, |groups, actions| {
        if actions.identity()? != expected_identity {
            return Err(Error::InvalidInput(
                "The local identity changed. Reopen the group action before continuing.".into(),
            ));
        }
        Ok((groups, actions))
    })
}

fn admit_for_owner(
    supervisor: &Supervisor,
    actions: &GroupActionClient,
    request: Request,
) -> Result<impl std::future::Future<Output = Result<Reply, Error>> + Send + 'static, Error> {
    with_clients(supervisor, |_, current_actions| {
        if !actions.same_owner(&current_actions) {
            return Err(Error::InvalidInput(
                "The local identity changed while recovering this action.".into(),
            ));
        }
        actions.admit(request)
    })
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

pub(crate) async fn prepare(input: PrepareGroupActionInput) -> GroupActionOutcome {
    prepare_with(supervisor(), input).await
}

async fn prepare_with(
    supervisor: &Supervisor,
    input: PrepareGroupActionInput,
) -> GroupActionOutcome {
    outcome(
        async {
            let (_, actions) = checked_clients(supervisor, input.owner_identity)?;
            admit_for_owner(supervisor, &actions, Request::Prepare(input.input))?.await
        }
        .await,
    )
}

pub(crate) async fn list() -> GroupActionListOutcome {
    let result =
        async { with_clients(supervisor(), |_, actions| actions.admit(Request::List))?.await }
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

pub(crate) async fn execute(reference: GroupActionReference) -> GroupActionOutcome {
    execute_with(supervisor(), reference).await
}

async fn execute_with(
    supervisor: &Supervisor,
    reference: GroupActionReference,
) -> GroupActionOutcome {
    outcome(
        async {
            // Capture both clients once. An old action cannot cross an identity
            // replacement or reopened owner while an asynchronous call is pending.
            let (groups, actions) = checked_clients(supervisor, reference.owner_identity)?;
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
            let execution = with_clients(supervisor, |_, current_actions| {
                if !actions.same_owner(&current_actions) {
                    return Err(Error::Unavailable(
                        "The local identity changed while recovering this action.".into(),
                    ));
                }
                // Recheck the stop/identity fence after reading the journal, and
                // admit the native mutation under that same lifecycle lock.
                admit_run(&groups, action.input).map_err(|error| match error {
                    prns_groups_runner_native::Error::Busy => Error::Busy,
                    error => Error::Unavailable(error.to_string()),
                })
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

type Execution = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<GroupActionResult, prns_groups_runner_native::Error>,
            > + Send,
    >,
>;

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

#[cfg(test)]
async fn run(
    client: &Client,
    input: GroupActionInput,
) -> Result<GroupActionResult, prns_groups_runner_native::Error> {
    admit_run(client, input)?.await
}

pub(crate) async fn acknowledge(
    reference: GroupActionReference,
    reviewed_expiry: bool,
) -> GroupActionAcknowledgeOutcome {
    acknowledge_with(supervisor(), reference, reviewed_expiry).await
}

async fn acknowledge_with(
    supervisor: &Supervisor,
    reference: GroupActionReference,
    reviewed_expiry: bool,
) -> GroupActionAcknowledgeOutcome {
    let result = async {
        let (_, actions) = checked_clients(supervisor, reference.owner_identity)?;
        admit_for_owner(
            supervisor,
            &actions,
            Request::Remove {
                request_id: reference.client_request_id,
                reviewed_expiry,
            },
        )?
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

#[cfg(test)]
mod tests;
