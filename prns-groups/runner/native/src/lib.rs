//! Native-only ownership and scheduling for generic group records and enrollment.
//! The application supplies its existing database lane, node and async runtime.
#![forbid(unsafe_code)]

mod announces;
mod client;
mod clock;
mod delivery;
mod diagnostics;
pub mod enrollment;
pub mod fixture;
mod hub;
mod manager;
pub mod profile;
mod progress;
mod records;
mod runner;
pub use prns_groups::sync::{SyncIssue, SyncPhase};
pub use progress::{GroupSyncProgress, PeerSyncProgress};
pub use records::*;
mod schedule;
mod session;
pub use manager::{GroupManager, LocalActivity, LocalMessageActivity};
mod types;

pub use announces::{
    announce_control, AnnounceControl, AnnounceOutcome, AnnounceRequests,
    MANUAL_ANNOUNCE_COOLDOWN_MILLIS,
};
pub use client::{Client, Job, Submit};
pub use clock::{Clock, SystemClock};
pub use diagnostics::{enable_metadata_diagnostics, metadata_schedule_snapshot};
pub use hub::{
    run_multi, run_multi_scoped_with_announcements, run_multi_with_announcements,
    run_multi_with_policy,
};
pub use prns_groups::retry::{RequestToken, RetryError};
pub use runner::{run, RunnerControl, RunnerOptions};
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
