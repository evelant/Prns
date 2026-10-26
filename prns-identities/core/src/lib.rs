//! Bounded identity publication and lifecycle coordination.
//!
//! This crate owns no keys, runtime, network, clock or filesystem. Adapters execute
//! explicit, identity-scoped effects and acknowledge verified durable outcomes.
//! Storage errors fence the coordinator until an authoritative reload. Caller-
//! supplied working and staging catalogs keep snapshot placement explicit.
#![no_std]
#![forbid(unsafe_code)]

mod coordinator;
mod model;

pub use coordinator::{validate_catalog, Coordinator, Storage};
pub use model::*;
