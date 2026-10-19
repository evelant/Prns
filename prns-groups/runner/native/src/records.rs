//! Generic native record consumption. Text is one optional application profile.
use prns_groups::{DeliverySummary, Endpoint, GroupId, NamespaceId, SchemaId};
use prns_groups_service::{Change, ClockReading, Record};
use prns_groups_store::StoreIncarnation;

/// Absolute native admission ceilings, before a job enters the bounded lane.
/// The enrolled group's persisted limits can be lower.
pub const MAX_PUBLICATION_BODY_BYTES: usize = 16_384;
pub const MAX_PUBLICATION_TARGETS: usize = crate::profile::MAX_MEMBERS;

/// A bounded inline publication. Retention uses the enrolled service profile.
/// Persist the request ID and unchanged input until its outcome is known.
#[derive(Clone)]
pub struct PublishRecord {
    pub group: GroupId,
    pub client_request_id: [u8; 32],
    pub namespace: NamespaceId,
    pub schema: SchemaId,
    /// Strictly sorted unique receipt targets. Every MLS reader can decrypt,
    /// including readers outside this list.
    pub targets: Vec<Endpoint>,
    pub body: Vec<u8>,
    pub clock: ClockReading,
}
impl core::fmt::Debug for PublishRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PublishRecord")
            .field("namespace", &self.namespace)
            .field("schema", &self.schema)
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}

/// Authenticated durable device evidence, never human read or command execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordDeliveryProgress {
    pub summary: DeliverySummary,
    /// Remaining old-era obligations cannot currently be forwarded or repaired.
    pub membership_changed: bool,
}

#[derive(Debug)]
pub struct StoredRecord {
    pub record: Record,
    /// Local durable insertion order; receipt updates do not move the record.
    pub sequence: u64,
    pub outgoing: bool,
    pub client_request_id: Option<[u8; 32]>,
    pub progress: RecordDeliveryProgress,
}
#[derive(Debug)]
pub struct RecordPage {
    pub records: Vec<StoredRecord>,
    /// Current feed head, including invalidation gaps created by maintenance.
    /// Pages are live reads, not a snapshot at this revision.
    pub revision: u64,
    pub next_before_sequence: Option<u64>,
}

/// Resume token bound to one local group store. Never reuse it after importing
/// another endpoint or initializing a replacement store incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeToken {
    pub group: GroupId,
    pub incarnation: StoreIncarnation,
    pub position: u64,
}
#[derive(Debug)]
pub struct RecordChanges {
    pub changes: Vec<Change>,
    pub next: ChangeToken,
    pub head: ChangeToken,
    /// The cursor crossed a retired event or a maintenance invalidation gap.
    /// Rebuild from retained records, even when `changes` is empty. A caught-up
    /// cursor still observes retention removal: maintenance advances the head
    /// atomically with deletion without retaining unbounded deletion events.
    /// This bounded feed is not an eternal audit log.
    pub history_pruned: bool,
}
