//! Presentation of shared runner observations; this layer never schedules work.

use crate::contract::{
    GroupPeerSynchronizationProgress, GroupSynchronizationIssue, GroupSynchronizationPhase,
    GroupSynchronizationProgress, GroupSynchronizationState,
};
use prns_groups_runner_native::{GroupSyncProgress, SyncIssue, SyncPhase};

pub(crate) fn project(
    progress: GroupSyncProgress,
    worker: &GroupSynchronizationState,
) -> GroupSynchronizationProgress {
    let running = progress.running && matches!(worker, GroupSynchronizationState::Running);
    GroupSynchronizationProgress {
        running,
        observed: progress.observed,
        peers: progress
            .peers
            .into_iter()
            .map(|peer| GroupPeerSynchronizationProgress {
                endpoint_id: peer.endpoint.id.0,
                endpoint_incarnation: peer.endpoint.incarnation.get(),
                identity: *peer.identity.as_bytes(),
                phase: if running {
                    match peer.phase {
                        SyncPhase::Unknown => GroupSynchronizationPhase::Unknown,
                        SyncPhase::Waiting => GroupSynchronizationPhase::Waiting,
                        SyncPhase::Connecting => GroupSynchronizationPhase::Connecting,
                        SyncPhase::Synchronizing => GroupSynchronizationPhase::Synchronizing,
                        SyncPhase::RetryScheduled => GroupSynchronizationPhase::RetryScheduled,
                        SyncPhase::StorageBlocked => GroupSynchronizationPhase::StorageBlocked,
                        SyncPhase::MembershipCatchUp => {
                            GroupSynchronizationPhase::MembershipCatchUp
                        }
                    }
                } else {
                    GroupSynchronizationPhase::Unknown
                },
                pending: peer.pending,
                retry_after_millis: running.then_some(peer.retry_after_millis).flatten(),
                failures: peer.failures,
                updated_ago_millis: peer.updated_ago_millis,
                last_issue: peer.last_issue.map(|issue| match issue {
                    SyncIssue::Transport => GroupSynchronizationIssue::Transport,
                    SyncIssue::StorageBusy => GroupSynchronizationIssue::StorageBusy,
                    SyncIssue::StorageFull => GroupSynchronizationIssue::StorageFull,
                    SyncIssue::StorageUnavailable => GroupSynchronizationIssue::StorageUnavailable,
                    SyncIssue::MembershipChanged => GroupSynchronizationIssue::MembershipChanged,
                    SyncIssue::TimeUncertain => GroupSynchronizationIssue::TimeUncertain,
                    SyncIssue::Rejected => GroupSynchronizationIssue::Rejected,
                    SyncIssue::UnsupportedPeer => GroupSynchronizationIssue::UnsupportedPeer,
                    SyncIssue::Configuration => GroupSynchronizationIssue::Configuration,
                }),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use prns_groups_runner_native::PeerSyncProgress;
    use prns_groups_service::{Endpoint, EndpointId, EndpointIncarnation};

    #[test]
    fn a_running_worker_without_observations_does_not_claim_connectivity() {
        let projected = project(
            GroupSyncProgress {
                running: true,
                observed: false,
                peers: Vec::new(),
            },
            &GroupSynchronizationState::Running,
        );
        assert!(projected.running);
        assert!(!projected.observed);
        assert!(projected.peers.is_empty());
    }

    #[test]
    fn worker_stop_or_failure_hides_live_retry_but_preserves_unknown_queue_state() {
        let progress = GroupSyncProgress {
            running: true,
            observed: true,
            peers: vec![PeerSyncProgress {
                endpoint: Endpoint {
                    id: EndpointId([1; 32]),
                    incarnation: EndpointIncarnation::new(1).expect("nonzero incarnation"),
                },
                identity: prns_host::IdentityHash::new([2; 16]),
                phase: SyncPhase::RetryScheduled,
                pending: None,
                retry_after_millis: Some(14_000),
                failures: 2,
                updated_ago_millis: 7_000,
                last_issue: Some(SyncIssue::Transport),
            }],
        };
        for worker in [
            GroupSynchronizationState::Stopped,
            GroupSynchronizationState::Unavailable {
                detail: "worker failed".into(),
            },
        ] {
            let projected = project(progress.clone(), &worker);
            assert!(!projected.running);
            assert_eq!(projected.peers[0].phase, GroupSynchronizationPhase::Unknown);
            assert_eq!(projected.peers[0].retry_after_millis, None);
            assert_eq!(projected.peers[0].pending, None);
        }
        let active = project(progress, &GroupSynchronizationState::Running);
        assert_eq!(
            active.peers[0].phase,
            GroupSynchronizationPhase::RetryScheduled
        );
        assert_eq!(active.peers[0].retry_after_millis, Some(14_000));
        assert_eq!(active.peers[0].updated_ago_millis, 7_000);
    }
}
