//! Bounded, disposable observations shared by native consumers. The durable
//! record/change/receipt APIs remain the authority for delivery and recovery.

use crate::{Error, NetworkConfig};
use prns_groups::sync::{SyncIssue, SyncPhase};
use prns_groups_service::{ControlReference, Endpoint, GroupId};
use prns_groups_store::StoreIncarnation;
use prns_host::IdentityHash;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

const MAX_PEERS: usize = 256;
const MAX_GROUPS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSyncProgress {
    /// The runner future currently owns the observation source. This does not
    /// mean a peer is connected, the OS will schedule it, or a record arrived.
    pub running: bool,
    /// A current runner configuration for this group has been observed. False
    /// also covers a bounded observation table that cannot represent the group.
    pub observed: bool,
    pub peers: Vec<PeerSyncProgress>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerSyncProgress {
    pub endpoint: Endpoint,
    pub identity: IdentityHash,
    pub phase: SyncPhase,
    /// Presence of durable authored work at the last bounded store query.
    /// None means unknown, not an empty outbox. It is not a current record count.
    pub pending: Option<bool>,
    /// Remaining local scheduling delay, not an estimated delivery time.
    pub retry_after_millis: Option<u64>,
    pub failures: u8,
    pub updated_ago_millis: u64,
    pub last_issue: Option<SyncIssue>,
}

struct Entry {
    group: GroupId,
    control: ControlReference,
    incarnation: StoreIncarnation,
    peer: PeerSyncProgress,
    observed: Instant,
    due: Option<Instant>,
}

#[derive(Default)]
struct State {
    running: bool,
    generation: u64,
    groups: Vec<GroupId>,
    peers: Vec<Entry>,
}

#[derive(Default)]
pub(crate) struct ProgressTracker(Mutex<State>);

pub(crate) struct ProgressLease(Arc<ProgressTracker>, u64);

pub(crate) struct PeerObserver {
    tracker: Arc<ProgressTracker>,
    generation: u64,
    group: GroupId,
    control: ControlReference,
    incarnation: StoreIncarnation,
    endpoint: Endpoint,
}
impl PeerObserver {
    pub fn new(tracker: Arc<ProgressTracker>, config: &NetworkConfig, endpoint: Endpoint) -> Self {
        Self {
            generation: tracker.generation(),
            tracker,
            group: config.group,
            control: config.control,
            incarnation: config.incarnation,
            endpoint,
        }
    }
    pub fn update(
        &self,
        phase: SyncPhase,
        pending: Option<bool>,
        due: Option<Instant>,
        failures: u8,
        issue: Option<SyncIssue>,
    ) {
        self.tracker.update(
            self.generation,
            self.group,
            self.endpoint,
            self.control,
            self.incarnation,
            phase,
            pending,
            due,
            failures,
            issue,
        );
    }
}
impl Drop for ProgressLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0 .0.lock() {
            if state.generation == self.1 {
                state.running = false;
            }
        }
    }
}

impl ProgressTracker {
    pub fn start(self: &Arc<Self>) -> Result<ProgressLease, Error> {
        let mut state = self.0.lock().map_err(|_| Error::Unavailable)?;
        // One scheduling owner per Client tree. Prevent two runners from
        // issuing conflicting status and doubling the same peer's attempts.
        if state.running {
            return Err(Error::Configuration);
        }
        state.generation = state.generation.checked_add(1).ok_or(Error::Unavailable)?;
        state.running = true;
        state.groups.clear();
        state.peers.clear();
        Ok(ProgressLease(Arc::clone(self), state.generation))
    }

    pub fn generation(&self) -> u64 {
        self.0.lock().map(|state| state.generation).unwrap_or(0)
    }

    pub fn configure(&self, configs: &[NetworkConfig]) {
        let Ok(mut state) = self.0.lock() else { return };
        state.groups.clear();
        state.peers.retain(|entry| {
            configs.iter().take(MAX_GROUPS).any(|config| {
                entry.group == config.group
                    && entry.control == config.control
                    && entry.incarnation == config.incarnation
                    && config.peers.iter().any(|peer| {
                        peer.endpoint != config.local
                            && entry.peer.endpoint == peer.endpoint
                            && entry.peer.identity == peer.identity
                    })
            })
        });
        for config in configs.iter().take(MAX_GROUPS) {
            for peer in config
                .peers
                .iter()
                .filter(|peer| peer.endpoint != config.local)
            {
                if state.peers.iter().any(|entry| {
                    entry.group == config.group
                        && entry.peer.endpoint == peer.endpoint
                        && entry.peer.identity == peer.identity
                }) {
                    continue;
                }
                if state.peers.len() == MAX_PEERS {
                    continue;
                }
                state.peers.push(Entry {
                    group: config.group,
                    control: config.control,
                    incarnation: config.incarnation,
                    peer: PeerSyncProgress {
                        endpoint: peer.endpoint,
                        identity: peer.identity,
                        phase: SyncPhase::Unknown,
                        pending: None,
                        retry_after_millis: None,
                        failures: 0,
                        updated_ago_millis: 0,
                        last_issue: None,
                    },
                    observed: Instant::now(),
                    due: None,
                });
            }
            if config
                .peers
                .iter()
                .filter(|peer| peer.endpoint != config.local)
                .all(|peer| {
                    state.peers.iter().any(|entry| {
                        entry.group == config.group
                            && entry.peer.endpoint == peer.endpoint
                            && entry.peer.identity == peer.identity
                    })
                })
            {
                state.groups.push(config.group);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        generation: u64,
        group: GroupId,
        endpoint: Endpoint,
        control: ControlReference,
        incarnation: StoreIncarnation,
        phase: SyncPhase,
        pending: Option<bool>,
        due: Option<Instant>,
        failures: u8,
        issue: Option<SyncIssue>,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if !state.running || state.generation != generation {
            return;
        }
        if let Some(entry) = state.peers.iter_mut().find(|entry| {
            entry.group == group
                && entry.peer.endpoint == endpoint
                && entry.control == control
                && entry.incarnation == incarnation
        }) {
            entry.peer.phase = phase;
            entry.peer.pending = pending;
            entry.peer.failures = failures;
            entry.peer.last_issue = issue;
            entry.due = due;
            entry.observed = Instant::now();
        }
    }

    pub fn unavailable(&self, issue: SyncIssue) {
        let Ok(mut state) = self.0.lock() else { return };
        for entry in &mut state.peers {
            entry.peer.phase = issue_phase(issue);
            entry.peer.pending = None;
            entry.peer.last_issue = Some(issue);
            entry.due = None;
            entry.observed = Instant::now();
        }
    }

    pub fn snapshot(&self, group: GroupId) -> GroupSyncProgress {
        let Ok(state) = self.0.lock() else {
            return GroupSyncProgress {
                running: false,
                observed: false,
                peers: Vec::new(),
            };
        };
        let now = Instant::now();
        GroupSyncProgress {
            running: state.running,
            observed: state.running && state.groups.contains(&group),
            peers: state
                .peers
                .iter()
                .filter(|entry| entry.group == group)
                .map(|entry| {
                    let mut peer = entry.peer.clone();
                    peer.updated_ago_millis =
                        crate::schedule::millis(now.saturating_duration_since(entry.observed));
                    peer.retry_after_millis = if state.running {
                        entry
                            .due
                            .map(|due| crate::schedule::millis(due.saturating_duration_since(now)))
                    } else {
                        None
                    };
                    if !state.running {
                        peer.phase = SyncPhase::Unknown;
                        peer.pending = None;
                    }
                    peer
                })
                .collect(),
        }
    }
}

pub(crate) fn issue(error: &Error) -> SyncIssue {
    if error.is_storage_full() {
        return SyncIssue::StorageFull;
    }
    match error {
        Error::Busy => SyncIssue::StorageBusy,
        Error::Clock => SyncIssue::TimeUncertain,
        Error::NotProvisioned => SyncIssue::MembershipChanged,
        Error::Configuration => SyncIssue::Configuration,
        Error::Transport(prns_groups_reticulum::Error::Service(error)) => service_issue(error),
        Error::Transport(prns_groups_reticulum::Error::Remote(
            prns_groups_reticulum::RemoteError::Unauthorized
            | prns_groups_reticulum::RemoteError::UnknownGroup,
        )) => SyncIssue::MembershipChanged,
        Error::Transport(_) => SyncIssue::Transport,
        Error::Storage(_) | Error::Unavailable | Error::Corrupt => SyncIssue::StorageUnavailable,
        Error::Service(error) => service_issue(error),
        _ => SyncIssue::Configuration,
    }
}

fn service_issue(
    error: &prns_groups_service::Error<prns_groups_store_native::NativeError>,
) -> SyncIssue {
    use prns_groups_service::{Error as ServiceError, Fault};
    match error {
        ServiceError::Fault(Fault::ConfigurationChanged) => SyncIssue::MembershipChanged,
        ServiceError::Fault(Fault::TimeUncertain) => SyncIssue::TimeUncertain,
        ServiceError::Fault(Fault::Capacity) => SyncIssue::StorageFull,
        ServiceError::Fault(Fault::ApplicationStorageFault | Fault::Corrupt)
        | ServiceError::Store(_) => SyncIssue::StorageUnavailable,
        _ => SyncIssue::Rejected,
    }
}

pub(crate) fn issue_phase(issue: SyncIssue) -> SyncPhase {
    match issue {
        SyncIssue::StorageBusy | SyncIssue::StorageFull | SyncIssue::StorageUnavailable => {
            SyncPhase::StorageBlocked
        }
        SyncIssue::MembershipChanged => SyncPhase::MembershipCatchUp,
        _ => SyncPhase::RetryScheduled,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use prns_groups::{AuthorizationEra, ControlId, EndpointId, EndpointIncarnation};

    fn config() -> NetworkConfig {
        let peer = Endpoint {
            id: EndpointId([2; 32]),
            incarnation: EndpointIncarnation::new(1).unwrap(),
        };
        NetworkConfig {
            group: GroupId([1; 32]),
            control: ControlReference {
                head: ControlId([3; 32]),
                era: AuthorizationEra(ControlId([3; 32])),
            },
            incarnation: StoreIncarnation::new([4; 24]).unwrap(),
            local: Endpoint {
                id: EndpointId([5; 32]),
                ..peer
            },
            local_reticulum_identity: IdentityHash::new([6; 16]),
            readers: vec![peer],
            peers: vec![prns_groups_reticulum::Peer {
                endpoint: peer,
                identity: IdentityHash::new([7; 16]),
                destination: prns_host::DestinationHash::new([8; 16]),
            }],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn observations_expire_on_owner_drop_and_restart_and_never_claim_receipts() {
        let tracker = Arc::new(ProgressTracker::default());
        let config = config();
        assert!(!tracker.snapshot(config.group).observed);
        let lease = tracker.start().unwrap();
        assert!(tracker.start().is_err());
        tracker.configure(std::slice::from_ref(&config));
        let observer = PeerObserver::new(Arc::clone(&tracker), &config, config.peers[0].endpoint);
        observer.update(
            SyncPhase::RetryScheduled,
            Some(true),
            Some(Instant::now() + std::time::Duration::from_secs(30)),
            3,
            Some(SyncIssue::Transport),
        );
        tokio::time::advance(std::time::Duration::from_secs(12)).await;
        let snapshot = tracker.snapshot(config.group);
        assert!(snapshot.running && snapshot.observed);
        assert_eq!(snapshot.peers[0].pending, Some(true));
        assert_eq!(snapshot.peers[0].retry_after_millis, Some(18_000));
        assert_eq!(snapshot.peers[0].updated_ago_millis, 12_000);
        drop(lease);
        let stopped = tracker.snapshot(config.group);
        assert!(!stopped.running && !stopped.observed);
        assert_eq!(stopped.peers[0].phase, SyncPhase::Unknown);
        assert_eq!(stopped.peers[0].pending, None);
        assert_eq!(stopped.peers[0].retry_after_millis, None);
        let _next = tracker.start().unwrap();
        assert!(tracker.snapshot(config.group).peers.is_empty());
        assert!(!tracker.snapshot(config.group).observed);
        tracker.configure(std::slice::from_ref(&config));
        // A late observation from the old owner cannot cross generations, even
        // when group/control/store/endpoint are all unchanged.
        observer.update(SyncPhase::Connecting, Some(true), None, 0, None);
        assert_eq!(
            tracker.snapshot(config.group).peers[0].phase,
            SyncPhase::Unknown
        );
    }

    #[test]
    fn old_control_and_store_observations_cannot_overwrite_current_scope() {
        let tracker = Arc::new(ProgressTracker::default());
        let _lease = tracker.start().unwrap();
        let mut config = config();
        tracker.configure(std::slice::from_ref(&config));
        let old = PeerObserver::new(Arc::clone(&tracker), &config, config.peers[0].endpoint);
        config.control.head = ControlId([9; 32]);
        tracker.configure(std::slice::from_ref(&config));
        old.update(SyncPhase::Connecting, Some(true), None, 0, None);
        assert_eq!(
            tracker.snapshot(config.group).peers[0].phase,
            SyncPhase::Unknown
        );
        let old_store = PeerObserver::new(Arc::clone(&tracker), &config, config.peers[0].endpoint);
        config.incarnation = StoreIncarnation::new([10; 24]).unwrap();
        tracker.configure(std::slice::from_ref(&config));
        old_store.update(SyncPhase::Connecting, Some(true), None, 0, None);
        assert_eq!(
            tracker.snapshot(config.group).peers[0].phase,
            SyncPhase::Unknown
        );
        tracker.configure(&[]);
        assert!(tracker.snapshot(config.group).peers.is_empty());
    }

    #[test]
    fn observation_capacity_is_bounded_and_storage_failure_is_not_idle() {
        let tracker = Arc::new(ProgressTracker::default());
        let _lease = tracker.start().unwrap();
        let base = config();
        let configs: Vec<_> = (0..MAX_GROUPS * 3)
            .map(|i| NetworkConfig {
                group: GroupId([i as u8; 32]),
                ..base.clone()
            })
            .collect();
        tracker.configure(&configs);
        assert_eq!(tracker.0.lock().unwrap().groups.len(), MAX_GROUPS);
        assert_eq!(tracker.0.lock().unwrap().peers.len(), MAX_GROUPS);
        tracker.unavailable(SyncIssue::StorageFull);
        let progress = tracker.snapshot(configs[0].group);
        assert_eq!(progress.peers[0].phase, SyncPhase::StorageBlocked);
        assert_eq!(progress.peers[0].pending, None);
        assert_eq!(progress.peers[0].last_issue, Some(SyncIssue::StorageFull));
        let crowded: Vec<_> = configs
            .into_iter()
            .take(MAX_GROUPS)
            .map(|mut config| {
                config.peers = (0..20)
                    .map(|i| prns_groups_reticulum::Peer {
                        endpoint: Endpoint {
                            id: EndpointId([i + 10; 32]),
                            ..base.peers[0].endpoint
                        },
                        identity: IdentityHash::new([i + 10; 16]),
                        destination: prns_host::DestinationHash::new([i + 10; 16]),
                    })
                    .collect();
                config
            })
            .collect();
        tracker.configure(&crowded);
        assert_eq!(tracker.0.lock().unwrap().peers.len(), MAX_PEERS);
        for (i, config) in crowded.iter().enumerate() {
            assert_eq!(tracker.snapshot(config.group).observed, i < MAX_PEERS / 20);
        }
    }
}
