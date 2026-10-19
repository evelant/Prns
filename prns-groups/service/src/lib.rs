//! Recoverable records over an already enrolled, fixed-membership MLS group.
//!
//! The service commits application facts with MLS state and exact output. The
//! supplied descriptor is trusted setup, not a membership-discovery protocol.
//! Its small internal body codec is provisional; Reticulum framing, enrollment,
//! changing membership and relay authorization remain separate integration work.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod codec;
mod service;
mod storage;
mod types;

pub use prns_groups::{
    ClockInterval, ClockReading, ControlReference, DeliverySummary, DurableReceipt, Endpoint,
    EndpointId, EndpointIncarnation, EnvelopeId, GroupId, NamespaceId, OperationId,
    PublicationMetadata, PublicationValidity, RecordId, SchemaId, Seconds, Timestamp,
};
pub use service::{GroupService, MaintenanceOutcome, MaintenancePlan};
pub use types::{
    Change, ChangeCursor, ChangeKind, ChangePage, Descriptor, EnvelopeInfo, Error, Fault, Incoming,
    InventoryCursor, InventoryKind, InventoryPage, InventorySnapshot, Limits, Publication,
    Published, ReceiptOutput, Record, RejectReason, MAX_RECEIPT_WIRE_BYTES,
};

/// Canonical identifier for exact MLS ciphertext within one logical group.
pub fn envelope_id(group: GroupId, wire: &[u8]) -> EnvelopeId {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"prns.groups.service.operation.v1\0");
    hash.update(b"receive");
    hash.update(group.0);
    hash.update(wire);
    EnvelopeId(hash.finalize().into())
}
