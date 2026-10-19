use crate::client::{Command, Reply};
use crate::{
    Conversation, ConversationMember, Error, Message, MessagePage, NetworkConfig, Provisioned,
    SendText,
};
use prns_groups_mls::Adapter;
use prns_groups_reticulum::{ProtocolError, SyncCursor};
use prns_groups_service::{
    ChangeCursor, ChangeKind, ClockReading, Descriptor, Endpoint, GroupService, Incoming,
    InventoryKind, Publication, PublicationValidity, RecordId, Timestamp,
};
use prns_groups_store::{Coordinator, OperationTicket};
use prns_groups_store_native::{NativeBackend, NativeRequest, NativeStoreOwner};
use redb::{Database, Durability, ReadableDatabase, TableDefinition};

mod records;

const CURSORS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("prns_groups_runner_cursors_v1");
// Keep one row per namespace/peer while binding progress to the exact local
// enrollment. This is a local persistence format, never a protocol frame.
const CURSOR_CONTEXT_BYTES: usize = 92;
const CURSOR_CONTEXT_MAGIC: &[u8; 4] = b"PGN\x01";
type Service<'a> = GroupService<'a, NativeBackend<'a>>;

/// Runs only on the caller's existing exclusive database lane. It creates no
/// worker, database or runtime. Every job ends before any network await begins.
pub struct StoreSession<'db> {
    database: &'db Database,
    owner: NativeStoreOwner<'db>,
    provisioned: Provisioned,
}
impl<'db> StoreSession<'db> {
    pub fn open(database: &'db Database, provisioned: Provisioned) -> Result<Self, Error> {
        let binding = provisioned.identity.binding();
        let local = Endpoint {
            id: prns_groups::EndpointId(binding.endpoint),
            incarnation: prns_groups::EndpointIncarnation::new(binding.incarnation)
                .map_err(|_| Error::Configuration)?,
        };
        if provisioned.readers.is_empty()
            || provisioned.title.is_empty()
            || provisioned.title.len() > 128
            || provisioned.peers.len() + 1 != provisioned.readers.len()
            || provisioned
                .peers
                .iter()
                .any(|peer| peer.endpoint == local || !provisioned.readers.contains(&peer.endpoint))
        {
            return Err(Error::Configuration);
        }
        let mut session = Self {
            database,
            owner: NativeStoreOwner::new(database),
            provisioned,
        };
        session.with_service(|_, _| Ok(()))?;
        Ok(session)
    }

    pub(crate) fn group(&self) -> prns_groups::GroupId {
        self.provisioned.group
    }
    pub(crate) fn control(&self) -> prns_groups::ControlReference {
        self.provisioned.control
    }
    pub(crate) fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        execute_with_owner(self.database, &mut self.owner, &self.provisioned, command)
    }
    fn with_service<T>(
        &mut self,
        action: impl FnOnce(&mut Service<'_>, &Provisioned) -> Result<T, Error>,
    ) -> Result<T, Error> {
        SessionView {
            database: self.database,
            owner: &mut self.owner,
            provisioned: &self.provisioned,
        }
        .with_service(action)
    }
    #[cfg(test)]
    fn config(&self) -> Result<NetworkConfig, Error> {
        configuration(&self.provisioned)
    }
}

pub(crate) fn execute_with_owner(
    database: &Database,
    owner: &mut NativeStoreOwner<'_>,
    provisioned: &Provisioned,
    command: Command,
) -> Result<Reply, Error> {
    SessionView {
        database,
        owner,
        provisioned,
    }
    .execute(command)
}
struct SessionView<'a, 'db> {
    database: &'a Database,
    owner: &'a mut NativeStoreOwner<'db>,
    provisioned: &'a Provisioned,
}
impl SessionView<'_, '_> {
    fn config(&self) -> Result<NetworkConfig, Error> {
        configuration(self.provisioned)
    }
    fn with_service<T>(
        &mut self,
        action: impl FnOnce(&mut Service<'_>, &Provisioned) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let cfg = &self.provisioned;
        // A request result must outlive its admission window. Otherwise a
        // custom short-retention profile could delete a still-valid request ID
        // and accidentally publish it a second time.
        if cfg.service_limits.maximum_window.0 < crate::profile::RETRY_POLICY.horizon_secs {
            return Err(Error::Configuration);
        }
        let backend = self
            .owner
            .open(cfg.namespace, cfg.incarnation, cfg.native_limits)?;
        let coordinator = Coordinator::new(
            backend,
            prns_groups_store::Limits {
                max_changes: cfg.native_limits.max_changes,
                max_key_bytes: cfg.native_limits.max_key_bytes,
                max_value_bytes: cfg.native_limits.max_value_bytes,
                max_staged_bytes: cfg.native_limits.max_staged_bytes,
            },
        )
        .map_err(|error| Error::Service(prns_groups_service::Error::Store(error)))?;
        let adapter = Adapter::new(
            coordinator,
            cfg.identity.clone(),
            cfg.policy.clone(),
            cfg.mls_limits,
        )
        .map_err(|error| Error::Service(prns_groups_service::Error::Security(error)))?;
        if cfg.owner_identity.is_none() {
            retire_fixture_outputs(&adapter, cfg)?;
        }
        let construct = if cfg.owner_identity.is_some() {
            GroupService::new_managed
        } else {
            GroupService::new
        };
        let mut service = construct(
            adapter,
            Descriptor {
                group: cfg.group,
                control: cfg.control,
                readers: &cfg.readers,
            },
            cfg.service_limits,
        )?;
        action(&mut service, cfg)
    }
    pub(crate) fn execute(&mut self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::Identity => Ok(Reply::Identity(self.provisioned.local_reticulum_identity)),
            Command::Configs => Ok(Reply::Configs(vec![self.config()?])),
            Command::Enrollment(_)
            | Command::Invitations(_)
            | Command::EnrollmentWork(_)
            | Command::EnrollmentHandle { .. }
            | Command::EnrollmentComplete { .. } => Err(Error::NotProvisioned),
            Command::Config => Ok(Reply::Config(self.config()?)),
            Command::LoadCursor(peer) => {
                let key = self.cursor_key(peer)?;
                let read = self.database.begin_read().map_err(db_error)?;
                let table = match read.open_table(CURSORS) {
                    Ok(table) => table,
                    Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Reply::Cursor(None)),
                    Err(error) => return Err(db_error(error)),
                };
                let value = table.get(key.as_slice()).map_err(db_error)?;
                let cursor = value
                    .map(|bytes| decode_cursor(self.provisioned, bytes.value()))
                    .transpose()?
                    .flatten();
                Ok(Reply::Cursor(cursor))
            }
            Command::SaveCursor(peer, cursor) => {
                let key = self.cursor_key(peer)?;
                let encoded = cursor.map(|cursor| encode_cursor(self.provisioned, cursor));
                {
                    let read = self.database.begin_read().map_err(db_error)?;
                    let table = match read.open_table(CURSORS) {
                        Ok(table) => Some(table),
                        Err(redb::TableError::TableDoesNotExist(_)) => None,
                        Err(error) => return Err(db_error(error)),
                    };
                    let existing = table
                        .as_ref()
                        .map(|table| table.get(key.as_slice()).map_err(db_error))
                        .transpose()?
                        .flatten();
                    if existing.as_ref().map(|value| value.value())
                        == encoded.as_ref().map(|value| value.as_slice())
                    {
                        // This lane is the sole writer. Unchanged snapshot polls
                        // need no commit or flash write merely to persist the
                        // same completed coverage again.
                        return Ok(Reply::Done);
                    }
                }
                let mut write = self.database.begin_write().map_err(db_error)?;
                write
                    .set_durability(Durability::Immediate)
                    .map_err(db_error)?;
                {
                    let mut table = write.open_table(CURSORS).map_err(db_error)?;
                    match encoded {
                        Some(encoded) => {
                            table
                                .insert(key.as_slice(), encoded.as_slice())
                                .map_err(db_error)?;
                        }
                        None => {
                            table.remove(key.as_slice()).map_err(db_error)?;
                        }
                    }
                }
                write.commit().map_err(db_error)?;
                Ok(Reply::Done)
            }
            Command::Conversations => self.with_service(|service, cfg| {
                let revision = service.changes(ChangeCursor(0), 1)?.head.0;
                let members = conversation_members(cfg)?;
                Ok(Reply::Conversations(vec![Conversation {
                    owner_identity: cfg.owner_identity,
                    can_invite: cfg.owner_identity == Some(cfg.local_reticulum_identity),
                    group: cfg.group,
                    title: cfg.title.clone(),
                    revision,
                    control_head: cfg.control.head,
                    member_count: members.len(),
                    members,
                }]))
            }),
            Command::Messages {
                group,
                before_sequence,
                limit,
                clock,
            } => self.with_service(|service, cfg| {
                records::messages(service, cfg, group, before_sequence, limit, clock)
                    .map(Reply::Messages)
            }),
            Command::Records {
                group,
                before_sequence,
                limit,
                clock,
            } => self.with_service(|service, cfg| {
                records::page(service, cfg, group, before_sequence, limit, clock)
                    .map(Reply::Records)
            }),
            Command::Record {
                group,
                record,
                clock,
            } => self.with_service(|service, cfg| {
                records::record(service, cfg, group, record, clock).map(Reply::Record)
            }),
            Command::Progress {
                group,
                record,
                clock,
            } => self.with_service(|service, cfg| {
                if group != cfg.group {
                    return Err(Error::InvalidInput);
                }
                if service.record(record)?.is_none() {
                    return Ok(Reply::Progress(None));
                }
                records::progress(service, record, clock).map(|value| Reply::Progress(Some(value)))
            }),
            Command::Changes {
                group,
                after,
                limit,
            } => self.with_service(|service, cfg| {
                records::changes(service, cfg, group, after, limit).map(Reply::Changes)
            }),
            Command::Publish(request) => {
                self.with_service(|service, cfg| publish(service, cfg, request).map(Reply::Sent))
            }
            Command::Maintain(clock) => self.with_service(|service, cfg| {
                maintain(service, cfg, clock)?;
                Ok(Reply::Done)
            }),
            Command::SendText(request) => self.with_service(|service, cfg| {
                records::publish_text(service, cfg, request).map(Reply::Sent)
            }),
            Command::RepairReceipts => self.with_service(|service, cfg| {
                let changes = service.changes(ChangeCursor(0), cfg.service_limits.max_changes)?;
                for change in changes.changes {
                    if let ChangeKind::Received(record) = change.kind {
                        repair_receipt(service, cfg, record)?;
                    }
                }
                Ok(Reply::Done)
            }),
            Command::PendingDelivery { peer, after, clock } => self.with_service(|service, _| {
                Ok(Reply::PendingDelivery(
                    service.pending_delivery(peer, after, clock)?,
                ))
            }),
            Command::Receipt(record) => self.with_service(|service, cfg| {
                repair_receipt(service, cfg, record)?;
                Ok(Reply::Envelope(
                    service.local_receipt(record)?.map(|wire| wire.to_vec()),
                ))
            }),
            Command::Snapshot => {
                self.with_service(|service, _| Ok(Reply::Snapshot(service.snapshot()?)))
            }
            Command::Inventory {
                snapshot,
                after,
                kind,
                recipient,
                scan_limit,
            } => self.with_service(|service, _| {
                Ok(Reply::Inventory(
                    service.inventory(snapshot, after, kind, recipient, scan_limit)?,
                ))
            }),
            Command::Envelope(id) => self.with_service(|service, _| {
                Ok(Reply::Envelope(
                    service.envelope(id)?.map(|bytes| bytes.to_vec()),
                ))
            }),
            Command::Known { info, incarnation } => self.with_service(|service, cfg| {
                if incarnation != cfg.incarnation {
                    return Err(Error::Configuration);
                }
                let Some(local) = service.envelope_info(info.id)? else {
                    return Ok(Reply::Known(false));
                };
                if local.kind != info.kind
                    || local.record != info.record
                    || local.bytes != info.bytes
                {
                    return Err(Error::Transport(ProtocolError::WrongEnvelope.into()));
                }
                if local.kind == InventoryKind::Record {
                    repair_receipt(service, cfg, local.record)?;
                }
                Ok(Reply::Known(true))
            }),
            Command::Receive { wire, options } => self.with_service(|service, cfg| {
                if options.incarnation != cfg.incarnation {
                    return Err(Error::Configuration);
                }
                maintain(service, cfg, options.clock)?;
                let incoming = service.receive(next(service, cfg)?, &wire, options.clock)?;
                if let Incoming::Record(record) = incoming {
                    repair_receipt(service, cfg, record)?;
                }
                Ok(Reply::Incoming(incoming))
            }),
        }
    }
    fn cursor_key(&self, peer: Endpoint) -> Result<Vec<u8>, Error> {
        if !self
            .provisioned
            .peers
            .iter()
            .any(|known| known.endpoint == peer)
        {
            return Err(Error::InvalidInput);
        }
        let mut key = self.provisioned.namespace.0.to_vec();
        key.extend_from_slice(&peer.id.0);
        key.extend_from_slice(&peer.incarnation.get().to_be_bytes());
        Ok(key)
    }
}
fn cursor_context(provisioned: &Provisioned) -> [u8; CURSOR_CONTEXT_BYTES] {
    let mut bytes = [0; CURSOR_CONTEXT_BYTES];
    bytes[..4].copy_from_slice(CURSOR_CONTEXT_MAGIC);
    bytes[4..36].copy_from_slice(&provisioned.control.head.0);
    bytes[36..68].copy_from_slice(&provisioned.control.era.0 .0);
    bytes[68..].copy_from_slice(provisioned.incarnation.as_bytes());
    bytes
}

fn encode_cursor(
    provisioned: &Provisioned,
    cursor: SyncCursor,
) -> [u8; CURSOR_CONTEXT_BYTES + prns_groups_reticulum::SYNC_CURSOR_BYTES] {
    let mut bytes = [0; CURSOR_CONTEXT_BYTES + prns_groups_reticulum::SYNC_CURSOR_BYTES];
    bytes[..CURSOR_CONTEXT_BYTES].copy_from_slice(&cursor_context(provisioned));
    bytes[CURSOR_CONTEXT_BYTES..].copy_from_slice(&cursor.to_bytes());
    bytes
}

fn decode_cursor(provisioned: &Provisioned, bytes: &[u8]) -> Result<Option<SyncCursor>, Error> {
    if bytes.starts_with(b"PGC") {
        // Earlier native rows had no enrollment binding. Validate the old
        // checkpoint, then perform one fresh scan instead of guessing its era.
        SyncCursor::from_bytes(bytes).map_err(|_| Error::Corrupt)?;
        return Ok(None);
    }
    if bytes.len() < CURSOR_CONTEXT_BYTES || &bytes[..4] != CURSOR_CONTEXT_MAGIC {
        return Err(Error::Corrupt);
    }
    let cursor =
        SyncCursor::from_bytes(&bytes[CURSOR_CONTEXT_BYTES..]).map_err(|_| Error::Corrupt)?;
    if bytes[..CURSOR_CONTEXT_BYTES] != cursor_context(provisioned) {
        return Ok(None);
    }
    Ok(Some(cursor))
}

#[cfg(test)]
mod checkpoint_tests;

fn conversation_members(cfg: &Provisioned) -> Result<Vec<ConversationMember>, Error> {
    let binding = cfg.identity.binding();
    let local = Endpoint {
        id: prns_groups::EndpointId(binding.endpoint),
        incarnation: prns_groups::EndpointIncarnation::new(binding.incarnation)
            .map_err(|_| Error::Configuration)?,
    };
    if cfg.readers.is_empty()
        || cfg.readers.len() > cfg.mls_limits.max_members
        || cfg.readers.len() > cfg.service_limits.max_targets
        || cfg.peers.len().checked_add(1) != Some(cfg.readers.len())
        || !cfg.readers.contains(&local)
        || cfg.readers.iter().enumerate().any(|(index, reader)| {
            cfg.readers[..index]
                .iter()
                .any(|other| other.id == reader.id)
        })
        || cfg
            .peers
            .iter()
            .any(|peer| peer.endpoint.id == local.id || !cfg.readers.contains(&peer.endpoint))
    {
        return Err(Error::Configuration);
    }
    let mut members = Vec::with_capacity(cfg.readers.len());
    for endpoint in &cfg.readers {
        let is_local = *endpoint == local;
        let identity = if is_local {
            cfg.local_reticulum_identity
        } else {
            let mut peers = cfg.peers.iter().filter(|peer| peer.endpoint == *endpoint);
            let identity = peers.next().ok_or(Error::Configuration)?.identity;
            if peers.next().is_some() {
                return Err(Error::Configuration);
            }
            identity
        };
        if members
            .iter()
            .any(|member: &ConversationMember| member.identity == identity)
        {
            return Err(Error::Configuration);
        }
        members.push(ConversationMember {
            endpoint: *endpoint,
            identity,
            is_local,
            is_owner: cfg.owner_identity == Some(identity),
        });
    }
    if cfg.owner_identity.is_some() && !members.iter().any(|member| member.is_owner) {
        return Err(Error::Configuration);
    }
    Ok(members)
}

pub(crate) fn configuration(provisioned: &Provisioned) -> Result<NetworkConfig, Error> {
    let binding = provisioned.identity.binding();
    Ok(NetworkConfig {
        control: provisioned.control,
        group: provisioned.group,
        local: Endpoint {
            id: prns_groups::EndpointId(binding.endpoint),
            incarnation: prns_groups::EndpointIncarnation::new(binding.incarnation)
                .map_err(|_| Error::Configuration)?,
        },
        readers: provisioned.readers.clone(),
        peers: provisioned.peers.clone(),
        incarnation: provisioned.incarnation,
        local_reticulum_identity: provisioned.local_reticulum_identity,
    })
}

// A fixture is installed as a complete fixed enrollment before StoreSession can
// open. It has no enrollment/control outbox. Only that development namespace may
// discard these bootstrap outputs without a managed enrollment Snapshot.
fn retire_fixture_outputs(
    adapter: &Adapter<NativeBackend<'_>>,
    cfg: &Provisioned,
) -> Result<(), Error> {
    let mut completed = Vec::new();
    for operation in adapter
        .pending_operations()
        .map_err(|e| Error::Service(prns_groups_service::Error::Security(e)))?
    {
        let mut fact = [0; 33];
        fact[0] = b'o';
        fact[1..].copy_from_slice(&operation.0);
        if adapter
            .application_read(&fact)
            .map_err(|e| Error::Service(prns_groups_service::Error::Security(e)))?
            .is_some()
        {
            continue;
        }
        let output = adapter
            .retry(operation)
            .map_err(|e| Error::Service(prns_groups_service::Error::Security(e)))?
            .ok_or(Error::Corrupt)?;
        use prns_groups_mls::Event;
        if matches!(
            output.event,
            Event::Created | Event::KeyPackage | Event::Joined | Event::Control
        ) && output
            .context
            .is_none_or(|context| context.group == cfg.group)
        {
            completed.push(operation);
            if completed.len() == 16 {
                break;
            }
        }
    }
    if !completed.is_empty() {
        let sequence = adapter
            .coordinator()
            .head()
            .map_err(|e| Error::Service(prns_groups_service::Error::Store(e)))?
            .0
            .checked_add(1)
            .ok_or(Error::Corrupt)?;
        let operation = OperationTicket::new(cfg.incarnation, sequence)
            .map_err(|_| Error::Corrupt)?
            .operation_id();
        adapter
            .maintain(operation, &completed, |_| {
                Ok::<_, core::convert::Infallible>(())
            })
            .map_err(|error| match error {
                prns_groups_mls::CompositionError::Mls(e) => {
                    Error::Service(prns_groups_service::Error::Security(e))
                }
                _ => Error::Corrupt,
            })?;
    }
    Ok(())
}

fn next(service: &Service<'_>, cfg: &Provisioned) -> Result<prns_groups::OperationId, Error> {
    let receipts = finalize_prefix(service, cfg)?;
    if receipts >= cfg.native_limits.max_receipts.saturating_sub(2) as u64 {
        return Err(Error::StorageFull);
    }
    next_maintenance(service, cfg)
}

fn next_maintenance(
    service: &Service<'_>,
    cfg: &Provisioned,
) -> Result<prns_groups::OperationId, Error> {
    let sequence = service
        .coordinator()
        .head()
        .map_err(|error| Error::Service(prns_groups_service::Error::Store(error)))?
        .0
        .checked_add(1)
        .ok_or(Error::Corrupt)?;
    Ok(OperationTicket::new(cfg.incarnation, sequence)
        .map_err(|_| Error::Corrupt)?
        .operation_id())
}
fn maintain(
    service: &mut Service<'_>,
    cfg: &Provisioned,
    clock: ClockReading,
) -> Result<(), Error> {
    if !matches!(clock, ClockReading::Trusted(_)) {
        return Ok(());
    }
    let receipts = finalize_prefix(service, cfg)?;
    let plan = service.maintenance_plan(clock, 1, 4)?;
    if !plan.is_empty() {
        let oldest = service.pending_operations()?.into_iter().min_by_key(|op| {
            OperationTicket::from_operation_id(*op)
                .map(|ticket| ticket.sequence())
                .unwrap_or(0)
        });
        // Preserve a cleanup transaction even when the oldest live result pins
        // all later receipts. Spending the last slot must release that prefix.
        let can_retire = receipts < cfg.native_limits.max_receipts.saturating_sub(1) as u64
            || (receipts < cfg.native_limits.max_receipts as u64
                && oldest.is_some_and(|op| plan.operations().contains(&op)));
        if can_retire {
            service.maintain(next_maintenance(service, cfg)?, &plan)?;
            finalize_prefix(service, cfg)?;
        }
    }
    Ok(())
}

fn finalize_prefix(service: &Service<'_>, cfg: &Provisioned) -> Result<u64, Error> {
    // Every retained result can still be needed for a retry or delivery. Only
    // finalize the contiguous prefix below the oldest surviving MLS result.
    let head = service
        .coordinator()
        .head()
        .map_err(|error| Error::Service(prns_groups_service::Error::Store(error)))?
        .0;
    let mut through = head;
    for operation in service.pending_operations()? {
        let ticket = OperationTicket::from_operation_id(operation).map_err(|_| Error::Corrupt)?;
        if ticket.incarnation() != cfg.incarnation || ticket.sequence() > head {
            return Err(Error::Corrupt);
        }
        through = through.min(ticket.sequence() - 1);
    }
    if through > 0 {
        let ticket = OperationTicket::new(cfg.incarnation, through).map_err(|_| Error::Corrupt)?;
        let outcome = service
            .coordinator()
            .maintain(NativeRequest::FinalizeThrough(ticket))
            .map_err(|error| Error::Service(prns_groups_service::Error::Store(error)))?;
        if !matches!(outcome, prns_groups_store::MaintenanceOutcome::Complete(_)) {
            return Err(Error::Corrupt);
        }
    }
    Ok(head - through)
}

fn repair_receipt(
    service: &mut Service<'_>,
    cfg: &Provisioned,
    record: RecordId,
) -> Result<(), Error> {
    if service.has_pending_receipt(record)? {
        service.emit_receipt(next(service, cfg)?, record)?;
    }
    Ok(())
}
fn publish(
    service: &mut Service<'_>,
    cfg: &Provisioned,
    request: crate::PublishRecord,
) -> Result<RecordId, Error> {
    if request.group != cfg.group {
        return Err(Error::InvalidInput);
    }
    if request.body.len() > cfg.service_limits.max_body_bytes
        || request.targets.len() > cfg.service_limits.max_targets
    {
        return Err(Error::InvalidInput);
    }
    if request.targets.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::InvalidInput);
    }
    let changes = service.changes(ChangeCursor(0), cfg.service_limits.max_changes)?;
    for change in changes.changes {
        let ChangeKind::Published(id) = change.kind else {
            continue;
        };
        let record = service.record(id)?.ok_or(Error::Corrupt)?;
        if record.nonce != request.client_request_id {
            continue;
        }
        if record.metadata.namespace != request.namespace
            || record.metadata.schema != request.schema
            || record.body.as_slice() != request.body.as_slice()
            || record.targets != request.targets
        {
            return Err(Error::IdempotencyConflict);
        }
        return Ok(id);
    }
    // Replaying an exact retained result is a read, even if unrelated cleanup
    // would fail. Maintenance belongs only to admission of new work.
    maintain(service, cfg, request.clock)?;
    let ClockReading::Trusted(interval) = request.clock else {
        return Err(Error::Clock);
    };
    crate::profile::RETRY_POLICY
        .validate_new(
            request.client_request_id,
            interval.lower().0,
            crate::profile::RETRY_POLICY.retirement_floor(service.expiry_floor()?.0, 0),
        )
        .map_err(Error::Retry)?;
    let token = crate::RequestToken::parse(request.client_request_id).map_err(Error::Retry)?;
    // Future clock skew must never let a message expire before its request ID.
    let created = Timestamp(interval.lower().0.max(token.issued_secs()));
    let deadline = Timestamp(
        created
            .0
            .checked_add(cfg.service_limits.maximum_window.0)
            .ok_or(Error::Clock)?,
    );
    let validity = PublicationValidity::new(created, deadline, cfg.service_limits.maximum_window)
        .map_err(|_| Error::Clock)?;
    if request.targets.is_empty() {
        return Err(Error::NoRecipients);
    }
    let published = service.publish(
        next(service, cfg)?,
        &Publication {
            nonce: request.client_request_id,
            namespace: request.namespace,
            schema: request.schema,
            validity,
            targets: &request.targets,
            body: &request.body,
        },
        request.clock,
    )?;
    Ok(published.record)
}
fn db_error(error: impl Into<redb::Error>) -> Error {
    Error::Storage(prns_groups_store_native::NativeError::Database(
        error.into(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use crate::fixture;
    use prns_groups_service::{ClockInterval, InventoryCursor};
    use prns_host::IdentityHash;

    #[test]
    fn conversation_roster_is_complete_and_rejects_ambiguous_identity_mappings() {
        let identities = [
            IdentityHash::new([1; 16]),
            IdentityHash::new([2; 16]),
            IdentityHash::new([3; 16]),
        ];
        let fixtures = fixture::generate("Roster", &identities, Timestamp(1_792_000_000)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let db = Database::create(directory.path().join("roster.redb")).unwrap();
        let cfg = fixture::load_and_install(&db, &fixtures[0], identities[0]).unwrap();
        let members = conversation_members(&cfg).unwrap();
        assert_eq!(members.len(), 3);
        assert_eq!(members.iter().filter(|member| member.is_local).count(), 1);
        assert!(members.iter().all(|member| !member.is_owner));
        for member in &members {
            assert!(cfg.readers.contains(&member.endpoint));
            if member.is_local {
                assert_eq!(member.identity, identities[0]);
            } else {
                assert!(cfg.peers.iter().any(
                    |peer| peer.endpoint == member.endpoint && peer.identity == member.identity
                ));
            }
        }
        let mut invalid = cfg.clone();
        invalid.peers[1] = invalid.peers[0].clone();
        assert!(matches!(
            conversation_members(&invalid),
            Err(Error::Configuration)
        ));
        invalid = cfg.clone();
        invalid.peers[0].identity = cfg.local_reticulum_identity;
        assert!(matches!(
            conversation_members(&invalid),
            Err(Error::Configuration)
        ));
        invalid = cfg.clone();
        invalid.readers[1] = invalid.readers[0];
        assert!(matches!(
            conversation_members(&invalid),
            Err(Error::Configuration)
        ));
        invalid = cfg.clone();
        invalid.mls_limits.max_members = 2;
        assert!(matches!(
            conversation_members(&invalid),
            Err(Error::Configuration)
        ));
        invalid = cfg.clone();
        invalid.owner_identity = Some(IdentityHash::new([99; 16]));
        assert!(matches!(
            conversation_members(&invalid),
            Err(Error::Configuration)
        ));
        let mut session = StoreSession::open(&db, cfg.clone()).unwrap();
        let Reply::Conversations(conversations) = session.execute(Command::Conversations).unwrap()
        else {
            panic!("conversation");
        };
        assert_eq!(conversations[0].members, members);
        assert_eq!(conversations[0].control_head, cfg.control.head);
    }

    #[test]
    fn receipt_repair_recovers_receive_commit_before_receipt_emission() {
        let now = Timestamp(1_792_000_000);
        let clock = ClockReading::Trusted(ClockInterval::new(now, now).unwrap());
        let identities = [IdentityHash::new([1; 16]), IdentityHash::new([2; 16])];
        let fixtures = fixture::generate("Pending receipt", &identities, now).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let alice_db = Database::create(directory.path().join("alice.redb")).unwrap();
        let bob_db = Database::create(directory.path().join("bob.redb")).unwrap();
        let alice_config =
            fixture::load_and_install(&alice_db, &fixtures[0], identities[0]).unwrap();
        let bob_config = fixture::load_and_install(&bob_db, &fixtures[1], identities[1]).unwrap();
        let group = alice_config.group;
        let mut alice = StoreSession::open(&alice_db, alice_config).unwrap();
        let mut bob = StoreSession::open(&bob_db, bob_config).unwrap();
        let recipient = bob.config().unwrap().local;
        let Reply::Sent(record) = alice
            .execute(Command::SendText(SendText {
                group,
                client_request_id: crate::RequestToken::new(now.0, [9; 20])
                    .unwrap()
                    .into_bytes(),
                text: "receipt recovery".into(),
                clock,
            }))
            .unwrap()
        else {
            panic!("send reply");
        };
        let wire = alice
            .with_service(|service, _| {
                let snapshot = service.snapshot()?.ok_or(Error::Corrupt)?;
                let page = service.inventory(
                    snapshot,
                    InventoryCursor(0),
                    InventoryKind::Record,
                    recipient,
                    32,
                )?;
                Ok(service
                    .envelope(page.items[0].id)?
                    .ok_or(Error::Corrupt)?
                    .to_vec())
            })
            .unwrap();
        bob.with_service(|service, cfg| {
            assert_eq!(
                service.receive(next(service, cfg)?, &wire, clock)?,
                Incoming::Record(record)
            );
            assert!(service.has_pending_receipt(record)?);
            Ok(())
        })
        .unwrap();
        drop(bob);
        drop(bob_db);
        let bob_db = Database::open(directory.path().join("bob.redb")).unwrap();
        let config = fixture::load_and_install(&bob_db, &fixtures[1], identities[1]).unwrap();
        let mut bob = StoreSession::open(&bob_db, config).unwrap();
        bob.execute(Command::RepairReceipts).unwrap();
        let revision = bob
            .with_service(|service, _| {
                assert!(!service.has_pending_receipt(record)?);
                Ok(service.changes(ChangeCursor(0), 32)?.head)
            })
            .unwrap();
        bob.execute(Command::RepairReceipts).unwrap();
        bob.with_service(|service, _| {
            assert_eq!(service.changes(ChangeCursor(0), 32)?.head, revision);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn bounded_maintenance_reuses_capacity_and_fences_old_retries_after_reopen() {
        let now = 1_792_000_000;
        let clock = |secs| {
            ClockReading::Trusted(ClockInterval::new(Timestamp(secs), Timestamp(secs)).unwrap())
        };
        let identities = [IdentityHash::new([1; 16]), IdentityHash::new([2; 16])];
        let fixtures = fixture::generate("Maintenance", &identities, Timestamp(now)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("maintain.redb");
        let db = Database::create(&path).unwrap();
        let cfg = fixture::load_and_install(&db, &fixtures[0], identities[0]).unwrap();
        let mut session = StoreSession::open(&db, cfg.clone()).unwrap();
        let window = crate::profile::RETRY_POLICY.horizon_secs;
        let old_request = SendText {
            group: cfg.group,
            client_request_id: crate::RequestToken::new(now, [9; 20]).unwrap().into_bytes(),
            text: "retry survives until retention ends".into(),
            clock: clock(now),
        };
        let Reply::Sent(first) = session
            .execute(Command::SendText(old_request.clone()))
            .unwrap()
        else {
            panic!("sent");
        };
        assert!(
            matches!(session.execute(Command::SendText(old_request.clone())).unwrap(), Reply::Sent(id) if id == first)
        );
        let mut delayed_retry = old_request.clone();
        delayed_retry.clock = clock(now + window + 1);
        assert!(
            matches!(session.execute(Command::SendText(delayed_retry)).unwrap(), Reply::Sent(id) if id == first)
        );
        // A retained exact retry must not run expiry maintenance first.
        session
            .with_service(|service, _| {
                assert!(service.record(first)?.is_some());
                Ok(())
            })
            .unwrap();
        for cycle in 1..=12_u64 {
            let time = now + cycle * (window + 1);
            session.execute(Command::Maintain(clock(time))).unwrap();
            let request = SendText {
                group: cfg.group,
                client_request_id: crate::RequestToken::new(time, [cycle as u8; 20])
                    .unwrap()
                    .into_bytes(),
                text: format!("window {cycle}"),
                clock: clock(time),
            };
            session.execute(Command::SendText(request)).unwrap();
            let usage = session
                .owner
                .open(cfg.namespace, cfg.incarnation, cfg.native_limits)
                .unwrap()
                .usage()
                .unwrap();
            assert!(usage.receipts <= 3, "retained receipts: {}", usage.receipts);
            assert!(usage.live_objects < 35, "objects: {}", usage.live_objects);
        }
        drop(session);
        drop(db);
        let db = Database::open(&path).unwrap();
        let mut session = StoreSession::open(&db, cfg).unwrap();
        assert!(
            matches!(session.execute(Command::SendText(old_request)), Err(error) if error.is_retry_expired())
        );
        let Reply::Messages(page) = session
            .execute(Command::Messages {
                group: session.group(),
                before_sequence: None,
                limit: 100,
                clock: clock(now),
            })
            .unwrap()
        else {
            panic!("messages");
        };
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].text, "window 12");
    }

    #[test]
    fn legacy_text_retry_replays_only_while_exact_result_is_retained() {
        let now = 1_792_000_000;
        let clock = |secs| {
            ClockReading::Trusted(ClockInterval::new(Timestamp(secs), Timestamp(secs)).unwrap())
        };
        let identities = [IdentityHash::new([1; 16]), IdentityHash::new([2; 16])];
        let fixtures = fixture::generate("Legacy retry", &identities, Timestamp(now)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let db = Database::create(directory.path().join("legacy.redb")).unwrap();
        let cfg = fixture::load_and_install(&db, &fixtures[0], identities[0]).unwrap();
        let group = cfg.group;
        let mut session = StoreSession::open(&db, cfg).unwrap();
        let id = session
            .with_service(|service, cfg| {
                let mut buffer = [0; prns_group_text::MAX_ENCODED_BYTES];
                let text = prns_group_text::Text::new("legacy durable send").unwrap();
                let body = text.encode(&mut buffer).unwrap();
                let targets: Vec<_> = cfg
                    .readers
                    .iter()
                    .copied()
                    .filter(|e| *e != service.local_endpoint())
                    .collect();
                Ok(service
                    .publish(
                        next(service, cfg)?,
                        &Publication {
                            nonce: [8; 32],
                            namespace: prns_group_text::NAMESPACE,
                            schema: prns_group_text::SCHEMA,
                            validity: PublicationValidity::new(
                                Timestamp(now),
                                Timestamp(now + cfg.service_limits.maximum_window.0),
                                cfg.service_limits.maximum_window,
                            )
                            .unwrap(),
                            targets: &targets,
                            body,
                        },
                        clock(now),
                    )?
                    .record)
            })
            .unwrap();
        let request = SendText {
            group,
            client_request_id: [8; 32],
            text: "legacy durable send".into(),
            clock: clock(now),
        };
        assert!(
            matches!(session.execute(Command::SendText(request.clone())).unwrap(), Reply::Sent(found) if found == id)
        );
        session
            .execute(Command::Maintain(clock(
                now + crate::profile::RETRY_POLICY.horizon_secs,
            )))
            .unwrap();
        assert!(
            matches!(session.execute(Command::SendText(request)), Err(error) if error.is_retry_expired())
        );
    }
    #[test]
    fn admission_reserves_the_last_receipt_for_cleanup_of_a_pinned_prefix() {
        let now = 1_792_000_000;
        let clock = |secs| {
            ClockReading::Trusted(ClockInterval::new(Timestamp(secs), Timestamp(secs)).unwrap())
        };
        let ids = [IdentityHash::new([1; 16]), IdentityHash::new([2; 16])];
        let fixtures = fixture::generate("Receipt reserve", &ids, Timestamp(now)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db = Database::create(dir.path().join("reserve.redb")).unwrap();
        let cfg = fixture::load_and_install(&db, &fixtures[0], ids[0]).unwrap();
        let mut session = StoreSession::open(&db, cfg.clone()).unwrap();
        session
            .execute(Command::SendText(SendText {
                group: cfg.group,
                client_request_id: crate::RequestToken::new(now, [1; 20]).unwrap().into_bytes(),
                text: "Pins the receipt prefix until expiry".into(),
                clock: clock(now),
            }))
            .unwrap();
        session
            .with_service(|service, cfg| {
                // Exercise the native admission watermark with a small policy;
                // the actual backend remains larger so reaching the cap is safe.
                let mut small = cfg.clone();
                small.native_limits.max_receipts = 6;
                while finalize_prefix(service, cfg)? < 5 {
                    let tx = service
                        .coordinator()
                        .begin(
                            next_maintenance(service, cfg)?,
                            service.coordinator().head().map_err(|e| {
                                Error::Service(prns_groups_service::Error::Store(e))
                            })?,
                        )
                        .map_err(|e| Error::Service(prns_groups_service::Error::Store(e)))?;
                    tx.commit()
                        .map_err(|e| Error::Service(prns_groups_service::Error::Store(e)))?;
                }
                assert!(matches!(next(service, &small), Err(Error::StorageFull)));
                maintain(service, &small, clock(now))?;
                assert_eq!(finalize_prefix(service, cfg)?, 5);
                maintain(
                    service,
                    &small,
                    clock(now + crate::profile::RETRY_POLICY.horizon_secs),
                )?;
                assert_eq!(finalize_prefix(service, cfg)?, 0);
                assert!(next(service, &small).is_ok());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn text_profile_rejects_retention_shorter_than_its_retry_window() {
        let ids = [IdentityHash::new([1; 16]), IdentityHash::new([2; 16])];
        let fixtures =
            fixture::generate("Short retention", &ids, Timestamp(1_792_000_000)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db = Database::create(dir.path().join("short.redb")).unwrap();
        let mut cfg = fixture::load_and_install(&db, &fixtures[0], ids[0]).unwrap();
        cfg.service_limits.maximum_window =
            prns_groups_service::Seconds(crate::profile::RETRY_POLICY.horizon_secs - 1);
        assert!(matches!(
            StoreSession::open(&db, cfg),
            Err(Error::Configuration)
        ));
    }
}
