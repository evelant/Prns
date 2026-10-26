//! Bounded UI recovery intents on the existing application database lane.
//! The runner owns accepted operations; this journal only retains their exact
//! inputs and confirmed local results until the user interface acknowledges them.

use crate::contract::{GroupAction, GroupActionInput, GroupActionResult, GroupActionState};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

pub(crate) mod execution;

const ACTIONS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("group_ui_actions_v1");
pub(crate) const MAX_ACTIONS: usize = 32;
const MAX_ACTION_BYTES: usize = 16_384;

#[derive(Debug)]
pub(crate) enum Error {
    Busy,
    StorageFull,
    InvalidInput(String),
    NeedsReview,
    Unavailable(String),
}

pub(crate) enum Request {
    Prepare(GroupActionInput),
    List,
    Read([u8; 32]),
    Finish {
        request_id: [u8; 32],
        state: GroupActionState,
    },
    Remove {
        request_id: [u8; 32],
        reviewed_expiry: bool,
    },
}

pub(crate) enum Reply {
    Action(GroupAction),
    List(Vec<GroupAction>),
    Removed,
}

pub(crate) fn request_id(input: &GroupActionInput) -> [u8; 32] {
    match input {
        GroupActionInput::Create { input } => input.client_request_id,
        GroupActionInput::Invite { input } => input.client_request_id,
        GroupActionInput::Respond { input } => input.client_request_id,
        GroupActionInput::Send { input } => input.client_request_id,
        GroupActionInput::RemoveMember { input } => input.client_request_id,
        GroupActionInput::LeaveGroup { input } => input.client_request_id,
        GroupActionInput::CloseGroup { input } => input.client_request_id,
        GroupActionInput::DeleteLocalGroup { input } => input.client_request_id,
        GroupActionInput::CancelInvitation { input } => input.client_request_id,
        GroupActionInput::DismissInvitation { input } => input.client_request_id,
    }
}

pub(crate) fn is_local_mutation(input: &GroupActionInput) -> bool {
    matches!(
        input,
        GroupActionInput::DeleteLocalGroup { .. } | GroupActionInput::DismissInvitation { .. }
    )
}

#[derive(PartialEq, Eq)]
enum Scope {
    Create,
    Group([u8; 32]),
    Invitation([u8; 32]),
}

fn scope(
    input: &GroupActionInput,
    resolve_invitation: &mut impl FnMut([u8; 32]) -> Result<Option<[u8; 32]>, Error>,
) -> Result<Scope, Error> {
    let scope = match input {
        GroupActionInput::Create { .. } => Scope::Create,
        GroupActionInput::Invite { input } => Scope::Group(input.group_id),
        GroupActionInput::Send { input } => Scope::Group(input.group_id),
        GroupActionInput::RemoveMember { input } => Scope::Group(input.group_id),
        GroupActionInput::LeaveGroup { input } => Scope::Group(input.group_id),
        GroupActionInput::CloseGroup { input } => Scope::Group(input.group_id),
        GroupActionInput::DeleteLocalGroup { input } => Scope::Group(input.group_id),
        GroupActionInput::Respond { input } => Scope::Invitation(input.invitation_id),
        GroupActionInput::CancelInvitation { input } => Scope::Invitation(input.invitation_id),
        GroupActionInput::DismissInvitation { input } => Scope::Invitation(input.invitation_id),
    };
    match scope {
        Scope::Invitation(id) => Ok(resolve_invitation(id)?
            .map(Scope::Group)
            .unwrap_or(Scope::Invitation(id))),
        scope => Ok(scope),
    }
}

fn unavailable(error: impl std::fmt::Display) -> Error {
    Error::Unavailable(format!("The saved group action is unavailable: {error}"))
}

fn validate(input: &GroupActionInput) -> Result<(), Error> {
    prns_groups_runner_native::RequestToken::parse(request_id(input))
        .map_err(|_| Error::InvalidInput("The group request identifier is invalid.".into()))?;
    let valid = match input {
        GroupActionInput::Create { input } => {
            !input.title.trim().is_empty() && input.title.len() <= 128
        }
        GroupActionInput::Send { input } => !input.text.is_empty() && input.text.len() <= 2048,
        GroupActionInput::RemoveMember { input } => input.endpoint_incarnation != 0,
        GroupActionInput::Invite { .. }
        | GroupActionInput::Respond { .. }
        | GroupActionInput::LeaveGroup { .. }
        | GroupActionInput::CloseGroup { .. }
        | GroupActionInput::DeleteLocalGroup { .. }
        | GroupActionInput::CancelInvitation { .. }
        | GroupActionInput::DismissInvitation { .. } => true,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidInput(
            "The group action contains an invalid title, message, or member incarnation.".into(),
        ))
    }
}

fn key(owner: [u8; 16], request_id: [u8; 32]) -> [u8; 48] {
    let mut key = [0; 48];
    key[..16].copy_from_slice(&owner);
    key[16..].copy_from_slice(&request_id);
    key
}

fn encode(action: &GroupAction) -> Result<Vec<u8>, Error> {
    let encoded = serde_json::to_vec(action).map_err(unavailable)?;
    if encoded.len() > MAX_ACTION_BYTES {
        return Err(unavailable("record exceeds its byte bound"));
    }
    Ok(encoded)
}

fn read_all(database: &Database) -> Result<Vec<GroupAction>, Error> {
    let read = database.begin_read().map_err(unavailable)?;
    let table = match read.open_table(ACTIONS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
        Err(error) => return Err(unavailable(error)),
    };
    if table.len().map_err(unavailable)? > MAX_ACTIONS as u64 {
        return Err(unavailable("action count exceeds its bound"));
    }
    let mut actions = Vec::new();
    for entry in table.iter().map_err(unavailable)? {
        let (stored_key, value) = entry.map_err(unavailable)?;
        if value.value().len() > MAX_ACTION_BYTES {
            return Err(unavailable("record exceeds its byte bound"));
        }
        let action: GroupAction = serde_json::from_slice(value.value()).map_err(unavailable)?;
        validate(&action.input).map_err(|_| unavailable("invalid saved input"))?;
        if stored_key.value() != key(action.owner_identity, request_id(&action.input)) {
            return Err(unavailable("identity or request identifier mismatch"));
        }
        actions.push(action);
    }
    Ok(actions)
}

fn write(database: &Database, action: &GroupAction) -> Result<(), Error> {
    let encoded = encode(action)?;
    let key = key(action.owner_identity, request_id(&action.input));
    let write = database.begin_write().map_err(unavailable)?;
    {
        let mut table = write.open_table(ACTIONS).map_err(unavailable)?;
        table
            .insert(key.as_slice(), encoded.as_slice())
            .map_err(unavailable)?;
    }
    write.commit().map_err(unavailable)
}

/// Called only by the existing serialized store owner. The identity comes from
/// that owner's initialized group manager, never from foreign caller input.
#[cfg(test)]
pub(crate) fn execute(
    database: &Database,
    owner: Option<prns_host::IdentityHash>,
    request: Request,
) -> Result<Reply, Error> {
    execute_scoped(database, owner, request, |_| Ok(None))
}

pub(crate) fn execute_scoped(
    database: &Database,
    owner: Option<prns_host::IdentityHash>,
    request: Request,
    mut resolve_invitation: impl FnMut([u8; 32]) -> Result<Option<[u8; 32]>, Error>,
) -> Result<Reply, Error> {
    let owner = *owner
        .ok_or_else(|| unavailable("native identity is not open"))?
        .as_bytes();
    let actions = read_all(database)?;
    let find = |id| {
        actions
            .iter()
            .find(|action| action.owner_identity == owner && request_id(&action.input) == id)
    };
    match request {
        Request::List => Ok(Reply::List(
            actions
                .into_iter()
                .filter(|action| action.owner_identity == owner)
                .collect(),
        )),
        Request::Read(id) => find(id)
            .cloned()
            .map(Reply::Action)
            .ok_or_else(|| Error::InvalidInput("The saved group action no longer exists.".into())),
        Request::Prepare(input) => {
            validate(&input)?;
            if let Some(action) = find(request_id(&input)) {
                return if action.input == input {
                    Ok(Reply::Action(action.clone()))
                } else {
                    Err(Error::InvalidInput(
                        "This identifier belongs to another saved action.".into(),
                    ))
                };
            }
            let requested_scope = scope(&input, &mut resolve_invitation)?;
            for action in actions
                .iter()
                .filter(|action| action.owner_identity == owner)
            {
                if scope(&action.input, &mut resolve_invitation)? == requested_scope {
                    return Err(Error::InvalidInput(
                        "Recover the existing action before starting another in this conversation."
                            .into(),
                    ));
                }
            }
            if actions.len() >= MAX_ACTIONS {
                return Err(Error::StorageFull);
            }
            let action = GroupAction {
                owner_identity: owner,
                input,
                state: GroupActionState::Pending,
            };
            write(database, &action)?;
            Ok(Reply::Action(action))
        }
        Request::Finish { request_id, state } => {
            let mut action = find(request_id)
                .cloned()
                .ok_or_else(|| unavailable("saved action no longer exists"))?;
            if matches!(state, GroupActionState::Pending) {
                return Err(unavailable("an execution result cannot become pending"));
            }
            // Concurrent reconciliations must not downgrade a known result.
            if matches!(action.state, GroupActionState::Completed { .. }) || action.state == state {
                return Ok(Reply::Action(action));
            }
            if let GroupActionState::Completed { result } = &state {
                let valid = matches!(
                    (&action.input, result),
                    (
                        GroupActionInput::Create { .. },
                        GroupActionResult::Created { .. }
                    ) | (
                        GroupActionInput::Invite { .. },
                        GroupActionResult::Invited { .. }
                    ) | (
                        GroupActionInput::Respond { .. },
                        GroupActionResult::Responded
                    ) | (
                        GroupActionInput::Send { .. },
                        GroupActionResult::Sent { .. }
                    ) | (
                        GroupActionInput::RemoveMember { .. },
                        GroupActionResult::RemovedMember
                    ) | (
                        GroupActionInput::LeaveGroup { .. },
                        GroupActionResult::LeftGroup
                    ) | (
                        GroupActionInput::CloseGroup { .. },
                        GroupActionResult::ClosedGroup
                    ) | (
                        GroupActionInput::DeleteLocalGroup { .. },
                        GroupActionResult::DeletedLocalGroup
                    ) | (
                        GroupActionInput::CancelInvitation { .. },
                        GroupActionResult::CancelledInvitation
                    ) | (
                        GroupActionInput::DismissInvitation { .. },
                        GroupActionResult::DismissedInvitation
                    )
                ) || matches!(result, GroupActionResult::Rejected { detail, .. } if detail.len() <= 512);
                if !valid {
                    return Err(unavailable("result does not match the saved action"));
                }
            }
            action.state = state;
            write(database, &action)?;
            Ok(Reply::Action(action))
        }
        Request::Remove {
            request_id,
            reviewed_expiry,
        } => {
            let Some(action) = find(request_id) else {
                return Ok(Reply::Removed);
            };
            match action.state {
                GroupActionState::Completed { .. } => {}
                GroupActionState::RetryExpired if reviewed_expiry => {}
                _ => return Err(Error::NeedsReview),
            }
            let key = key(owner, request_id);
            let write = database.begin_write().map_err(unavailable)?;
            {
                write
                    .open_table(ACTIONS)
                    .map_err(unavailable)?
                    .remove(key.as_slice())
                    .map_err(unavailable)?;
            }
            write.commit().map_err(unavailable)?;
            Ok(Reply::Removed)
        }
    }
}

#[cfg(test)]
mod tests;
