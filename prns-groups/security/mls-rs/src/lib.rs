//! Bounded MLS operations over one recoverable crypto/application transaction.
//!
//! This crate uses `alloc`; the shared `prns-groups` core does not. Internal MLS
//! decoding/crypto allocation is not yet fallible. Platform heap, stack, entropy,
//! durable backend and physical erasure still require qualification before firmware use.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
mod adapter;
mod limits;
mod output;
mod policy;
mod provider;

pub use adapter::{
    Adapter, Admission, ApplicationTransaction, Command, Committed, CompositionError, Error,
    MAX_APPLICATION_KEY_BYTES,
};
pub use limits::{Limit, Limits};
pub use output::{Event, Output, OutputContext};
pub use policy::{LocalIdentity, MemberBinding, Policy, PolicyError};
pub use prns_groups::{GroupId, OperationId};
pub use prns_groups_store::{CommitReceipt, RecoveryOutcome};
pub use provider::ProviderError;
