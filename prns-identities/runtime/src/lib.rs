//! Identity-scoped runtime ownership, admission, and quiescence.
//!
//! The caller retains [`RuntimeState`] across controller recreation and serializes
//! controller access with actual dispatch. No executor, allocator, key material,
//! shared-host shutdown or interface teardown is owned by this crate. Drivers
//! must honor scopes at execution, not merely when submitting asynchronous work.
#![no_std]
#![forbid(unsafe_code)]

pub mod announce;
mod controller;
mod model;

pub use controller::*;
pub use model::*;
