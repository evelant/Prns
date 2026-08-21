use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::signal::Signal;
use portable_atomic::{AtomicBool, AtomicU64, Ordering};

use crate::engine::{InstantMillis, RouteSnapshot};
use crate::wire::DestinationHash;

const NO_INSPECTION_ID: u64 = u64::MAX;

/// Failure to begin one live Embassy node inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionError {
    /// Another task currently owns the single best-effort inspection lane.
    Busy,
    /// This handle was constructed without an inspection lane.
    Unavailable,
}

/// One route read directly from the live engine and the logical time of that read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedRoute {
    pub observed_at: InstantMillis,
    pub route: RouteSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectionQuery {
    RouteCount,
    LinkCount,
    Route(DestinationHash),
    NextRouteAfter(Option<DestinationHash>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InspectionRequest {
    pub id: u64,
    pub query: InspectionQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InspectionValue {
    Count(u32),
    Route(Option<InspectedRoute>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectionResponse {
    pub id: u64,
    pub value: InspectionValue,
}

/// Static storage for one independent live-engine inspection lane.
///
/// The lane is deliberately separate from protocol commands and their
/// completion pool. It provides bounded operational introspection without
/// making inspection a Reticulum engine command or mirroring engine state.
pub struct EmbassyInspectionLane<M: RawMutex> {
    next_id: AtomicU64,
    claimed: AtomicBool,
    request: Signal<M, InspectionRequest>,
    response: Signal<M, InspectionResponse>,
}

impl<M: RawMutex> EmbassyInspectionLane<M> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_id: AtomicU64::new(0),
            claimed: AtomicBool::new(false),
            request: Signal::new(),
            response: Signal::new(),
        }
    }

    fn mint(&self) -> u64 {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if id != NO_INSPECTION_ID {
                return id;
            }
        }
    }

    pub(crate) async fn inspect(
        &self,
        query: InspectionQuery,
    ) -> Result<InspectionValue, InspectionError> {
        if self
            .claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(InspectionError::Busy);
        }
        let _guard = InspectionGuard { lane: self };
        let id = self.mint();
        self.response.reset();
        self.request.signal(InspectionRequest { id, query });
        loop {
            let response = self.response.wait().await;
            if response.id == id {
                return Ok(response.value);
            }
        }
    }

    pub(crate) fn responder(&self) -> InspectionResponder<'_, M> {
        InspectionResponder { lane: self }
    }
}

impl<M: RawMutex> Default for EmbassyInspectionLane<M> {
    fn default() -> Self {
        Self::new()
    }
}

struct InspectionGuard<'a, M: RawMutex> {
    lane: &'a EmbassyInspectionLane<M>,
}

impl<M: RawMutex> Drop for InspectionGuard<'_, M> {
    fn drop(&mut self) {
        self.lane.claimed.store(false, Ordering::Release);
    }
}

pub(crate) struct InspectionResponder<'a, M: RawMutex> {
    lane: &'a EmbassyInspectionLane<M>,
}

impl<M: RawMutex> Clone for InspectionResponder<'_, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: RawMutex> Copy for InspectionResponder<'_, M> {}

impl<M: RawMutex> InspectionResponder<'_, M> {
    pub async fn wait(&self) -> InspectionRequest {
        self.lane.request.wait().await
    }

    pub fn respond(&self, response: InspectionResponse) {
        self.lane.response.signal(response);
    }
}
