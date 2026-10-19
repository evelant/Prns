//! Local retained-content activity. One existing scoped database lane owns both
//! projection and seen cursors; neither operation submits network work.
use super::*;
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

const SEEN: TableDefinition<&[u8], u64> = TableDefinition::new("identity_activity_seen_v1");
const MAX_SEEN_SCOPES: usize = 260; // Total group/direct conversation cursor slots.
const MAX_DIRECT_CONVERSATIONS: usize = 64;
const MAX_DIRECT_SCAN: usize = 512;

pub(crate) enum Operation {
    Read,
    Mark(MarkIdentityActivitySeenInput),
}
pub(crate) enum Reply {
    Read(IdentityActivitySummary),
    Mark(IdentityActivitySeenOutcome),
}
fn key(scope: &IdentityActivityScope) -> Vec<u8> {
    match scope {
        IdentityActivityScope::Group { group_id } => [b"g".as_slice(), group_id].concat(),
        IdentityActivityScope::Direct { destination } => [b"d".as_slice(), destination].concat(),
    }
}
fn seen(database: &Database) -> Result<Vec<(Vec<u8>, u64)>, String> {
    let read = database.begin_read().map_err(detail)?;
    let table = match read.open_table(SEEN) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
        Err(error) => return Err(detail(error)),
    };
    if table.len().map_err(detail)? > MAX_SEEN_SCOPES as u64 {
        return Err("Activity cursor capacity exceeded.".into());
    }
    table
        .iter()
        .map_err(detail)?
        .map(|entry| {
            let (key, value) = entry.map_err(detail)?;
            let bytes = key.value();
            if !matches!(
                (bytes.first(), bytes.len()),
                (Some(b'g'), 33) | (Some(b'd'), 17)
            ) {
                return Err("Malformed activity cursor.".into());
            }
            Ok((bytes.to_vec(), value.value()))
        })
        .collect()
}
fn position(cursors: &[(Vec<u8>, u64)], scope: &IdentityActivityScope) -> u64 {
    cursors
        .iter()
        .find(|(candidate, _)| match scope {
            IdentityActivityScope::Group { group_id } => {
                candidate.first() == Some(&b'g') && candidate.get(1..) == Some(group_id.as_slice())
            }
            IdentityActivityScope::Direct { destination } => {
                candidate.first() == Some(&b'd')
                    && candidate.get(1..) == Some(destination.as_slice())
            }
        })
        .map_or(0, |(_, value)| *value)
}
pub(crate) fn execute(
    database: &Database,
    groups: Option<&mut prns_groups_runner_native::GroupManager<'_>>,
    operation: Operation,
) -> Result<Reply, String> {
    let cursors = seen(database)?;
    if let Operation::Mark(input) = &operation {
        // Repeated/older acknowledgments cannot clear a later arrival, and do
        // not write a transaction merely because a visible screen refreshes.
        if input.through_sequence <= position(&cursors, &input.scope) {
            return Ok(Reply::Mark(IdentityActivitySeenOutcome::Saved));
        }
        if let IdentityActivityScope::Direct { destination } = input.scope {
            if !prns_lxmf::mailbox::mailbox_activity_position(
                database,
                destination,
                input.through_sequence,
            )
            .map_err(detail)?
            {
                return Ok(Reply::Mark(IdentityActivitySeenOutcome::InvalidInput {
                    detail: "The displayed record does not belong to this conversation.".into(),
                }));
            }
            return mark(database, &cursors, input).map(Reply::Mark);
        }
    }
    let groups = groups
        .ok_or_else(|| "Local group storage is unavailable.".to_owned())?
        .local_activity(direct::now() / 1000)
        .map_err(detail)?;
    if let Operation::Mark(input) = operation {
        let IdentityActivityScope::Group { group_id } = input.scope else {
            return Err("Unexpected activity scope.".into());
        };
        if !groups.groups.iter().any(|group| {
            group.group.0 == group_id && input.through_sequence <= group.latest_sequence
        }) {
            return Ok(Reply::Mark(IdentityActivitySeenOutcome::InvalidInput {
                detail: "The displayed group position is not available.".into(),
            }));
        }
        return mark(database, &cursors, &input).map(Reply::Mark);
    }
    let mut summary = IdentityActivitySummary {
        actionable_invitations: groups.actionable_invitations as u32,
        unread_groups: 0,
        unread_direct: 0,
        conversations: Vec::new(),
        partial: false,
    };
    for group in groups.groups {
        let scope = IdentityActivityScope::Group {
            group_id: group.group.0,
        };
        let count = group
            .incoming_sequences
            .iter()
            .filter(|sequence| **sequence > position(&cursors, &scope))
            .count() as u32;
        summary.unread_groups += count;
        summary.conversations.push(IdentityConversationActivity {
            scope,
            unread_count: count,
            through_sequence: group.latest_sequence,
        });
    }
    let (records, partial) =
        prns_lxmf::mailbox::read_mailbox_activity(database, MAX_DIRECT_SCAN).map_err(detail)?;
    summary.partial = partial;
    let mut direct_count = 0;
    for record in records {
        let scope = IdentityActivityScope::Direct {
            destination: record.peer,
        };
        let unread = u32::from(record.incoming && record.position > position(&cursors, &scope));
        summary.unread_direct += unread;
        if let Some(conversation) = summary
            .conversations
            .iter_mut()
            .find(|item| item.scope == scope)
        {
            conversation.unread_count += unread;
        } else if direct_count < MAX_DIRECT_CONVERSATIONS {
            direct_count += 1;
            summary.conversations.push(IdentityConversationActivity {
                scope,
                unread_count: unread,
                through_sequence: record.position,
            });
        } else {
            summary.partial = true;
        }
    }
    Ok(Reply::Read(summary))
}
fn mark(
    database: &Database,
    cursors: &[(Vec<u8>, u64)],
    input: &MarkIdentityActivitySeenInput,
) -> Result<IdentityActivitySeenOutcome, String> {
    let key = key(&input.scope);
    if !cursors.iter().any(|(candidate, _)| *candidate == key) && cursors.len() == MAX_SEEN_SCOPES {
        return Ok(IdentityActivitySeenOutcome::Capacity);
    }
    let mut write = database.begin_write().map_err(detail)?;
    write
        .set_durability(redb::Durability::Immediate)
        .map_err(detail)?;
    {
        write
            .open_table(SEEN)
            .map_err(detail)?
            .insert(key.as_slice(), input.through_sequence)
            .map_err(detail)?;
    }
    write.commit().map_err(detail)?;
    Ok(IdentityActivitySeenOutcome::Saved)
}

pub(super) async fn operation(
    services: &MessagingIdentity,
    operation: Operation,
) -> Result<Reply, String> {
    services
        .store()
        .map_err(detail)?
        .admit_activity(operation)
        .map_err(detail)?
        .await
        .map_err(detail)?
}
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl NamedIdentityClient {
    pub async fn read_identity_activity(&self) -> IdentityActivityOutcome {
        self.read_activity_with_owner(global()).await
    }
    pub async fn mark_identity_activity_seen(
        &self,
        input: MarkIdentityActivitySeenInput,
    ) -> IdentityActivitySeenOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move {
                match operation(services, Operation::Mark(input)).await? {
                    Reply::Mark(outcome) => Ok(outcome),
                    _ => Err("Unexpected activity result.".into()),
                }
            })
        })
        .await
        .unwrap_or_else(|detail| IdentityActivitySeenOutcome::Unavailable { detail })
    }
}
impl NamedIdentityClient {
    async fn read_activity_with_owner(
        &self,
        owner: Result<Arc<NamedRuntime>, String>,
    ) -> IdentityActivityOutcome {
        let (reply, receive) = oneshot::channel();
        let work: Work = Box::new(move |services| {
            Box::pin(async move {
                let result = match services {
                    Ok(services) => match operation(services, Operation::Read).await {
                        Ok(Reply::Read(summary)) => IdentityActivityOutcome::Ready { summary },
                        Ok(_) => IdentityActivityOutcome::Unavailable {
                            detail: "Unexpected activity result.".into(),
                        },
                        Err(detail) => IdentityActivityOutcome::Unavailable { detail },
                    },
                    Err(detail) => IdentityActivityOutcome::Unavailable { detail },
                };
                let _ = reply.send(result);
            })
        });
        let result = owner.and_then(|owner| {
            owner
                .commands
                .try_send(Request::Activity {
                    identity: self.identity.clone(),
                    work,
                })
                .map_err(|_| "Activity lane is busy or stopped.".into())
        });
        if let Err(detail) = result {
            return IdentityActivityOutcome::Unavailable { detail };
        }
        receive
            .await
            .unwrap_or_else(|_| IdentityActivityOutcome::Unavailable {
                detail: "The identity owner stopped.".into(),
            })
    }
}
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub async fn list_named_identity_activity() -> NamedIdentityActivityListOutcome {
    let owner = match global() {
        Ok(owner) => owner,
        Err(detail) => return NamedIdentityActivityListOutcome::Unavailable { detail },
    };
    list(&owner).await
}
pub(super) async fn list(owner: &Arc<NamedRuntime>) -> NamedIdentityActivityListOutcome {
    let catalog = match owner.catalog_request(Request::Catalog).await {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog,
        NamedIdentityCatalogOutcome::Busy => return NamedIdentityActivityListOutcome::Busy,
        NamedIdentityCatalogOutcome::Unavailable { detail }
        | NamedIdentityCatalogOutcome::Rejected { detail } => {
            return NamedIdentityActivityListOutcome::Unavailable { detail };
        }
    };
    let mut activities = Vec::with_capacity(MAX_IDENTITIES);
    for entry in catalog.identities {
        let client = NamedIdentityClient {
            identity: entry.reference.clone(),
        };
        let activity = client.read_activity_with_owner(Ok(owner.clone())).await;
        activities.push(NamedIdentityActivity {
            identity: entry.reference,
            activity,
        });
    }
    NamedIdentityActivityListOutcome::Ready { activities }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
pub(super) mod tests;
