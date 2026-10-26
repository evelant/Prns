use crate::contract::{GroupActionInput, GroupActionResult};
use prns_groups_runner_native::{
    Client, Clock, EnrollmentRequest, EnrollmentResult, Error, SendText, SystemClock,
};
use prns_groups_service::{ClockReading, Endpoint, EndpointId, EndpointIncarnation, GroupId};

pub(crate) type Execution =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<GroupActionResult, Error>> + Send>>;

pub(crate) fn admit_run(client: &Client, input: GroupActionInput) -> Result<Execution, Error> {
    if let GroupActionInput::Send { input } = input {
        let waiter = client.admit_send_text(SendText {
            group: GroupId(input.group_id),
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
    let ClockReading::Trusted(now) = SystemClock.now() else {
        return Err(Error::Clock);
    };
    let now = now.upper().0;
    let saved = match &input {
        GroupActionInput::RemoveMember { .. } => GroupActionResult::RemovedMember,
        GroupActionInput::LeaveGroup { .. } => GroupActionResult::LeftGroup,
        GroupActionInput::CloseGroup { .. } => GroupActionResult::ClosedGroup,
        GroupActionInput::DeleteLocalGroup { .. } => GroupActionResult::DeletedLocalGroup,
        GroupActionInput::CancelInvitation { .. } => GroupActionResult::CancelledInvitation,
        GroupActionInput::DismissInvitation { .. } => GroupActionResult::DismissedInvitation,
        GroupActionInput::Create { .. }
        | GroupActionInput::Invite { .. }
        | GroupActionInput::Respond { .. }
        | GroupActionInput::Send { .. } => GroupActionResult::Responded,
    };
    let request = match input {
        GroupActionInput::Create { input } => EnrollmentRequest::Create {
            request_id: input.client_request_id,
            title: input.title,
            now,
        },
        GroupActionInput::Invite { input } => EnrollmentRequest::Invite {
            request_id: input.client_request_id,
            group: GroupId(input.group_id),
            recipient: prns_host::IdentityHash::new(input.recipient_identity),
            now,
        },
        GroupActionInput::Respond { input } => EnrollmentRequest::Respond {
            request_id: input.client_request_id,
            invitation: input.invitation_id,
            accept: input.accept,
            now,
        },
        GroupActionInput::RemoveMember { input } => EnrollmentRequest::RemoveMember {
            request_id: input.client_request_id,
            group: GroupId(input.group_id),
            member: Endpoint {
                id: EndpointId(input.endpoint_id),
                incarnation: EndpointIncarnation::new(input.endpoint_incarnation)
                    .map_err(|_| Error::InvalidInput)?,
            },
            expected_head: input.expected_control_head,
            now,
        },
        GroupActionInput::LeaveGroup { input } => EnrollmentRequest::Leave {
            request_id: input.client_request_id,
            group: GroupId(input.group_id),
            now,
        },
        GroupActionInput::CloseGroup { input } => EnrollmentRequest::Close {
            request_id: input.client_request_id,
            group: GroupId(input.group_id),
            expected_head: input.expected_control_head,
            now,
        },
        GroupActionInput::DeleteLocalGroup { input } => EnrollmentRequest::DeleteLocal {
            request_id: input.client_request_id,
            group: GroupId(input.group_id),
            now,
        },
        GroupActionInput::CancelInvitation { input } => EnrollmentRequest::CancelInvitation {
            request_id: input.client_request_id,
            invitation: input.invitation_id,
            now,
        },
        GroupActionInput::DismissInvitation { input } => EnrollmentRequest::DismissInvitation {
            request_id: input.client_request_id,
            invitation: input.invitation_id,
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
            EnrollmentResult::Saved => Ok(saved),
        }
    }))
}
