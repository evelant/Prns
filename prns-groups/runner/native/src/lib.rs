//! Native-only durable ownership for generic group records and enrollment.
//! The application supplies its existing database lane and authenticates enrollment peers.
#![forbid(unsafe_code)]

mod client;
mod clock;
pub mod enrollment;
pub mod fixture;
mod manager;
pub mod profile;
mod records;
pub use records::*;
mod session;
pub use manager::{GroupManager, LocalActivity, LocalMessageActivity};
mod types;

pub use client::{Client, Job, Submit};
pub use clock::{Clock, SystemClock};
pub use prns_groups::retry::{RequestToken, RetryError};
pub use session::StoreSession;
pub use types::*;

/// Mint an application retry token using platform entropy and wall-clock time.
/// Persist/reuse the returned ID with unchanged input after an ambiguous result.
pub fn create_request_id() -> Result<[u8; 32], Error> {
    let prns_groups::ClockReading::Trusted(now) = SystemClock.now() else {
        return Err(Error::Clock);
    };
    let mut entropy = [0; 20];
    getrandom::getrandom(&mut entropy).map_err(|_| Error::Unavailable)?;
    RequestToken::new(now.lower().0, entropy)
        .map(RequestToken::into_bytes)
        .map_err(Error::Retry)
}
