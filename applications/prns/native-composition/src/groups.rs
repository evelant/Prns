//! Native-only projections and provisioning for the development group profile.

use crate::contract::{GroupRequestIdOutcome, GroupSynchronizationState};
use crate::development_store::DevelopmentStoreFailure;
use prns_groups_runner_native::{Client, Provisioned};
use std::{
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch};

const NETWORK_LIMITS: prns_groups_reticulum::Limits = prns_groups_reticulum::Limits {
    max_frame_bytes: 65_536,
    max_page_items: 64,
    max_peers: 16,
    inbound_jobs: 8,
    exchange_timeout: Duration::from_secs(10),
};

#[derive(Debug)]
pub(crate) enum GroupNetworkError {
    Runner(prns_groups_runner_native::Error),
    Network(prns_groups_reticulum::NetworkError),
    Session(prns_host_native::owner::SessionError),
    IdentityMismatch,
    UnexpectedExit,
}

impl std::fmt::Display for GroupNetworkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runner(error) => write!(formatter, "Group runner failed: {error}"),
            Self::Network(error) => write!(formatter, "Group transport failed: {error}"),
            Self::Session(error) => write!(formatter, "The shared host is unavailable: {error:?}"),
            Self::IdentityMismatch => {
                formatter.write_str("The group fixture belongs to another Reticulum identity.")
            }
            Self::UnexpectedExit => {
                formatter.write_str("The group synchronization worker stopped unexpectedly.")
            }
        }
    }
}

impl std::error::Error for GroupNetworkError {}

#[derive(Clone)]
pub(crate) struct GroupHealth(
    Arc<Mutex<GroupSynchronizationState>>,
    crate::group_announces::LiveAnnounces,
);

impl GroupHealth {
    pub(crate) fn new() -> Self {
        Self(
            Arc::new(Mutex::new(GroupSynchronizationState::Stopped)),
            crate::group_announces::LiveAnnounces::new(),
        )
    }

    pub(crate) fn announces(&self) -> crate::group_announces::LiveAnnounces {
        self.1.clone()
    }

    pub(crate) fn read(&self) -> GroupSynchronizationState {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set(&self, state: GroupSynchronizationState) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
    }
}

const ENROLLMENT_LIMITS: prns_groups_reticulum::enrollment::Limits =
    prns_groups_reticulum::enrollment::Limits {
        max_frame_bytes: 131_072,
        inbound_jobs: 8,
        exchange_timeout: Duration::from_secs(10),
    };

pub(crate) struct PreparedNetwork {
    client: Client,
    identity: prns_host::IdentityHash,
    destination: prns_host::DestinationHash,
    enrollment_destination: prns_host::DestinationHash,
    ingress: GroupIngress,
    requests: mpsc::Receiver<prns_groups_reticulum::RequestJob>,
    enrollment_requests: mpsc::Receiver<prns_groups_reticulum::enrollment::RequestJob>,
}

#[derive(Clone)]
pub(crate) struct GroupIngress {
    groups: prns_groups_reticulum::Ingress,
    enrollment: prns_groups_reticulum::enrollment::Ingress,
}
impl GroupIngress {
    pub(crate) fn on_event_ref(&self, event: &personal_rns::runtime::PrnsEvent<'_>) {
        let groups = self.groups.on_event_ref(event);
        let enrollment = self.enrollment.on_event_ref(event);
        if let personal_rns::runtime::PrnsEvent::Message(
            personal_rns::runtime::Message::Request {
                data, requester, ..
            },
        ) = event
        {
            crate::group_diagnostics::ingress(
                "sync_ingress",
                groups,
                data.len(),
                requester.is_some(),
            );
            crate::group_diagnostics::ingress(
                "enrollment_ingress",
                enrollment,
                data.len(),
                requester.is_some(),
            );
        }
    }
}
impl PreparedNetwork {
    pub(crate) fn ingress(&self) -> GroupIngress {
        self.ingress.clone()
    }
    pub(crate) fn destination_configs(
        &self,
    ) -> Result<Vec<prns_host::SingleDestinationConfig>, GroupNetworkError> {
        Ok(vec![
            prns_groups_reticulum::destination_config(NETWORK_LIMITS)
                .map_err(GroupNetworkError::Network)?,
            prns_groups_reticulum::enrollment::destination_config(ENROLLMENT_LIMITS)
                .map_err(GroupNetworkError::Network)?,
        ])
    }
}

pub(crate) async fn prepare_network(
    client: Client,
) -> Result<Option<PreparedNetwork>, GroupNetworkError> {
    let identity = client.identity().await.map_err(GroupNetworkError::Runner)?;
    let destination =
        prns_groups_reticulum::destination_for(identity).map_err(GroupNetworkError::Network)?;
    let enrollment_destination = prns_groups_reticulum::enrollment::destination_for(identity)
        .map_err(GroupNetworkError::Network)?;
    let (groups, requests) = prns_groups_reticulum::Ingress::new(destination, NETWORK_LIMITS)
        .map_err(GroupNetworkError::Network)?;
    let (enrollment, enrollment_requests) =
        prns_groups_reticulum::enrollment::Ingress::new(enrollment_destination, ENROLLMENT_LIMITS)
            .map_err(GroupNetworkError::Network)?;
    Ok(Some(PreparedNetwork {
        client,
        identity,
        destination,
        enrollment_destination,
        ingress: GroupIngress { groups, enrollment },
        requests,
        enrollment_requests,
    }))
}

pub(crate) async fn run_network(
    prepared: Option<PreparedNetwork>,
    host: prns_host_native::owner::HostClient,
    health: GroupHealth,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), GroupNetworkError> {
    if prepared.is_some() {
        health.set(GroupSynchronizationState::Running);
    }
    let result = drive_network(prepared, host, health.announces(), shutdown.clone()).await;
    health.announces().stop();
    if !*shutdown.borrow() {
        let error = match result {
            Ok(()) => GroupNetworkError::UnexpectedExit,
            Err(error) => error,
        };
        health.set(GroupSynchronizationState::Unavailable {
            detail: error.to_string(),
        });
        while !*shutdown.borrow() {
            if shutdown.changed().await.is_err() {
                break;
            }
        }
    }
    health.set(GroupSynchronizationState::Stopped);
    Ok(())
}
async fn drive_network(
    prepared: Option<PreparedNetwork>,
    host: prns_host_native::owner::HostClient,
    announces: crate::group_announces::LiveAnnounces,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), GroupNetworkError> {
    let Some(prepared) = prepared else {
        while !*shutdown.borrow() {
            if shutdown.changed().await.is_err() {
                break;
            }
        }
        return Ok(());
    };
    if host.identity_hash().map_err(GroupNetworkError::Session)? != prepared.identity {
        return Err(GroupNetworkError::IdentityMismatch);
    }
    let announcement_requests = announces.start().map_err(GroupNetworkError::Runner)?;
    prns_groups_runner_native::run_multi_with_announcements(
        prepared.client,
        host,
        prepared.destination,
        prepared.enrollment_destination,
        prepared.requests,
        prepared.enrollment_requests,
        NETWORK_LIMITS,
        ENROLLMENT_LIMITS,
        Default::default(),
        announcement_requests,
        shutdown,
    )
    .await
    .map_err(GroupNetworkError::Runner)
}

pub(crate) fn open_fixture(
    database: &redb::Database,
    path: &Path,
    identity: prns_host::IdentityHash,
) -> Result<Option<Provisioned>, DevelopmentStoreFailure> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(DevelopmentStoreFailure::unavailable(format!(
                "The development group fixture could not be opened: {error}"
            )))
        }
    };
    let mut encoded = zeroize::Zeroizing::new(Vec::new());
    file.take((prns_groups_runner_native::fixture::MAX_FIXTURE_BYTES + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|error| {
            DevelopmentStoreFailure::unavailable(format!(
                "The development group fixture could not be read: {error}"
            ))
        })?;
    if encoded.len() > prns_groups_runner_native::fixture::MAX_FIXTURE_BYTES {
        return Err(DevelopmentStoreFailure::unavailable(
            "The development group fixture exceeds its size limit.",
        ));
    }
    let provisioned =
        prns_groups_runner_native::fixture::load_and_install(database, &encoded, identity)
            .map_err(|error| {
                DevelopmentStoreFailure::unavailable(format!(
                    "The development group fixture could not be installed: {error}"
                ))
            })?;
    Ok(Some(provisioned))
}

pub(crate) fn create_request_id() -> GroupRequestIdOutcome {
    match prns_groups_runner_native::create_request_id() {
        Ok(client_request_id) => GroupRequestIdOutcome::Ready { client_request_id },
        Err(_) => GroupRequestIdOutcome::Unavailable {
            detail: "The platform could not generate a group send identifier.".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::development_store::DevelopmentStoreOwner;
    use prns_groups_runner_native::{Clock, SystemClock};

    #[tokio::test]
    async fn terminal_group_failure_keeps_the_borrowed_host_running() {
        let host = prns_host_native::owner::OwnedSession::open(prns_host::HostConfig {
            identity: prns_host::IdentityConfig::GenerateEphemeral,
            persistence: prns_host::PersistenceConfig::Ephemeral,
            role: prns_host::HostRole::Endpoint,
            destinations: Vec::new(),
            required_capabilities: Vec::new(),
            limits: prns_host::PrnsLimits::balanced(),
        })
        .await
        .unwrap();
        let client = host.client();
        let mut mismatched = *client.identity_hash().unwrap().as_bytes();
        mismatched[0] ^= 1;
        let identity = prns_host::IdentityHash::new(mismatched);
        let mut peer = mismatched;
        peer[0] ^= 2;
        let now = match SystemClock.now() {
            prns_groups_service::ClockReading::Trusted(interval) => interval.upper(),
            prns_groups_service::ClockReading::Unknown => panic!("clock unavailable"),
        };
        let fixtures = prns_groups_runner_native::fixture::generate(
            "Failure scope",
            &[identity, prns_host::IdentityHash::new(peer)],
            now,
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("group-fixture.json"), &fixtures[0]).unwrap();
        let store = DevelopmentStoreOwner::open(root.path(), &root.path().join("application.redb"))
            .unwrap();
        store.initialize_groups_blocking(identity).unwrap();
        let prepared = prepare_network(store.group_client()).await.unwrap();
        let health = store.group_health();
        let (stop, stopping) = watch::channel(false);
        let driver = run_network(prepared, client.clone(), health.clone(), stopping);
        tokio::pin!(driver);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    result = &mut driver => panic!("Group failure prematurely ended the lifetime: {result:?}"),
                    () = tokio::task::yield_now() => {
                        if matches!(health.read(), GroupSynchronizationState::Unavailable { .. }) { break; }
                    },
                }
            }
        }).await.unwrap();
        assert!(client.snapshot().await.is_ok());
        assert_eq!(store.group_client().conversations().await.unwrap().len(), 1);
        stop.send(true).unwrap();
        driver.await.unwrap();
        assert_eq!(health.read(), GroupSynchronizationState::Stopped);
        assert!(client.snapshot().await.is_ok());
        host.stop().await.unwrap();
        store.close().unwrap();
    }
}
