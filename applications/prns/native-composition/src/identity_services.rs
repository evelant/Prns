//! Native Rust composition for identity-scoped direct/group services.
//!
//! The caller owns one shared Reticulum host and the durable identity coordinator.
//! No selected-identity global, host stop, board pairing or key generation occurs
//! here. The generated phone API will consume this composition in a later slice.
use crate::development_store::{DevelopmentStoreOwner, ScopedOpen};
use personal_rns::runtime::{PrnsEvent, ServiceEndpoint, ServiceRegistration};
use prns_core::identity::vault::IdentitySecretKey;
use prns_groups_runner_native::Client as GroupClient;
use prns_host::{DestinationHash, IdentityHash};
use prns_host_native::{owner::HostClient, service_scope::NativeServiceScope};
use prns_identities_core::{KeyReference, ServiceDependency, ServiceId};
use prns_identities_native::scoped_store::StorageScope;
use prns_lxmf::direct::{LocalLxmfIdentity, LxmfCallbacks, PrnsDirectNetwork};
use prns_lxmf::mailbox::{
    DurableDirectLxmfService, MailboxReply, MailboxRequest, SystemMailboxClock,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

mod driver;
pub use driver::MessagingDriver;

/// One application service owns direct messages, groups and their scoped stores.
pub const MESSAGING_SERVICE: ServiceId = ServiceId(0x5052_4e53);
const MAX_OPERATIONS: usize = 16;
const MAX_LINKS: usize = 32;
const LIMITS: prns_groups_reticulum::Limits = prns_groups_reticulum::Limits {
    max_frame_bytes: 65_536,
    max_page_items: 64,
    max_peers: 16,
    inbound_jobs: 8,
    exchange_timeout: Duration::from_secs(10),
};
const ENROLLMENT_LIMITS: prns_groups_reticulum::enrollment::Limits =
    prns_groups_reticulum::enrollment::Limits {
        max_frame_bytes: 131_072,
        inbound_jobs: 8,
        exchange_timeout: Duration::from_secs(10),
    };

#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
fn error(value: impl std::fmt::Debug) -> Error {
    Error(format!("identity service: {value:?}"))
}

#[derive(Default)]
struct Events {
    // Local profile projection only; never authority for identity or routing.
    // Retained through pause so catalog reads cannot block on a service job.
    published_name: Option<String>,
    enabled: bool,
    direct_enabled: bool,
    scope: Option<NativeServiceScope>,
    direct: Option<LxmfCallbacks>,
    groups: Option<prns_groups_reticulum::Ingress>,
    enrollment: Option<prns_groups_reticulum::enrollment::Ingress>,
}
/// Clone into the shared host callback before creating service registrations.
/// Each slot is immutable in ownership; callbacks cannot follow UI selection.
#[derive(Clone)]
pub struct MessagingEvents(Arc<Mutex<Vec<Arc<Mutex<Events>>>>>, usize);
impl MessagingEvents {
    pub fn new(maximum_identities: usize) -> Result<Self, Error> {
        if maximum_identities == 0 || maximum_identities > 64 {
            return Err(error("identity bound"));
        }
        Ok(Self(Arc::new(Mutex::new(Vec::new())), maximum_identities))
    }
    fn slot(&self) -> Result<Arc<Mutex<Events>>, Error> {
        let mut entries = self.0.lock().map_err(error)?;
        if entries.len() == self.1 {
            return Err(error("event capacity"));
        }
        let entry = Arc::new(Mutex::new(Events::default()));
        entries.push(entry.clone());
        Ok(entry)
    }
    pub fn on_event(&self, event: &PrnsEvent<'_>) {
        let Ok(entries) = self.0.lock() else {
            return;
        };
        for entry in entries.iter() {
            let Ok(entry) = entry.lock() else {
                continue;
            };
            // Link-close accounting must continue after fresh ingress is fenced.
            if let Some(scope) = &entry.scope {
                scope.on_event_ref(event);
            }
            if entry.direct_enabled {
                if let Some(direct) = &entry.direct {
                    direct.on_prns_event(event);
                }
            }
            if !entry.enabled {
                continue;
            }
            if let Some(groups) = &entry.groups {
                groups.on_event_ref(event);
            }
            if let Some(enrollment) = &entry.enrollment {
                enrollment.on_event_ref(event);
            }
        }
    }
    pub fn on_announce(&self, observation: &prns_host_native::AuthenticatedAnnounce<'_>) {
        let Ok(entries) = self.0.lock() else {
            return;
        };
        for entry in entries.iter() {
            let Ok(entry) = entry.lock() else {
                continue;
            };
            if entry.enabled {
                if let Some(direct) = &entry.direct {
                    direct.on_authenticated_announce(observation);
                }
            }
        }
    }
}

/// An immutable storage/key binding. Construct only after the catalog has
/// published this key and the caller has verified it through its vault adapter.
pub struct MessagingIdentity {
    root: PathBuf,
    storage_scope: StorageScope,
    key_reference: KeyReference,
    secret: IdentitySecretKey,
    host: HostClient,
    events: Arc<Mutex<Events>>,
    store: Option<DevelopmentStoreOwner>,
    endpoints: Vec<ServiceEndpoint>,
    registered: usize,
    network: Option<NativeServiceScope>,
    direct: Option<Arc<DurableDirectLxmfService>>,
    inbound: Option<(
        mpsc::Receiver<prns_groups_reticulum::RequestJob>,
        mpsc::Receiver<prns_groups_reticulum::RequestJob>,
    )>,
    stop: Option<watch::Sender<bool>>,
    task: Option<tokio::task::JoinHandle<Result<(), prns_groups_runner_native::Error>>>,
    dependencies: Option<ServiceDependency>,
    announcements: Option<prns_groups_runner_native::AnnounceControl>,
    paused_endpoints: usize,
    pause_outgoing: Option<Vec<personal_rns::routing::links::LinkId>>,
    prepared_pause: Option<personal_rns::runtime::ServicePause>,
    storage_close_failed: bool,
}
impl MessagingIdentity {
    pub fn open(
        root: &Path,
        scope: StorageScope,
        key_reference: KeyReference,
        secret: IdentitySecretKey,
        create: bool,
        host: HostClient,
        events: &MessagingEvents,
    ) -> Result<Self, Error> {
        let store = DevelopmentStoreOwner::open_scoped(
            root,
            scope,
            secret.clone(),
            if create {
                ScopedOpen::Create
            } else {
                ScopedOpen::Existing
            },
        )
        .map_err(error)?;
        store
            .initialize_groups_blocking(IdentityHash::new(scope.public_hash.0))
            .map_err(error)?;
        Ok(Self {
            root: root.into(),
            storage_scope: scope,
            key_reference,
            secret,
            host,
            events: events.slot()?,
            store: Some(store),
            endpoints: Vec::new(),
            registered: 0,
            network: None,
            direct: None,
            inbound: None,
            stop: None,
            task: None,
            dependencies: None,
            announcements: None,
            paused_endpoints: 0,
            pause_outgoing: None,
            prepared_pause: None,
            storage_close_failed: false,
        })
    }
    pub(crate) fn is_network_active(&self) -> bool {
        self.events
            .lock()
            .map(|events| events.enabled)
            .unwrap_or(false)
    }
    pub(crate) fn cache_published_name(&self, name: String) -> Result<(), Error> {
        self.events.lock().map_err(error)?.published_name = Some(name);
        Ok(())
    }
    pub(crate) fn network_scope(&self) -> Result<NativeServiceScope, Error> {
        self.network
            .clone()
            .ok_or_else(|| error("identity service is not active"))
    }
    pub(crate) fn host(&self) -> &HostClient {
        &self.host
    }
    pub(crate) fn store(&self) -> Result<&DevelopmentStoreOwner, Error> {
        self.store.as_ref().ok_or_else(|| error("storage paused"))
    }
    pub(crate) async fn close_local(&mut self) -> Result<(), Error> {
        if let Some(store) = self.store.take() {
            store.fence_all_mutations();
            store.drain().await.map_err(error)?;
            store.close().map_err(error)?;
        }
        Ok(())
    }
    pub fn scope(&self) -> StorageScope {
        self.storage_scope
    }
    pub fn groups(&self) -> Result<GroupClient, Error> {
        Ok(self
            .store
            .as_ref()
            .ok_or_else(|| error("storage paused"))?
            .group_client())
    }
    pub fn direct(&self) -> Result<Arc<DurableDirectLxmfService>, Error> {
        self.direct
            .clone()
            .ok_or_else(|| error("direct service is not prepared"))
    }
    /// Caller-owned announce controls for this activation only. Admission and
    /// aggregate identity scheduling remain the embedding's responsibility.
    pub fn announcements(&self) -> Result<prns_groups_runner_native::AnnounceControl, Error> {
        self.announcements
            .clone()
            .ok_or_else(|| error("group service is not running"))
    }
    fn fence(&self) -> Result<(), Error> {
        self.events.lock().map_err(error)?.enabled = false;
        if let Some(store) = &self.store {
            store.fence_fresh_mutations();
        }
        if let Some(network) = &self.network {
            network.fence();
        }
        if let Some(stop) = &self.stop {
            stop.send_replace(true);
        }
        Ok(())
    }
    pub(crate) async fn prepare(&mut self) -> Result<(), Error> {
        if self.storage_close_failed {
            return Err(error("storage close failed; restart recovery required"));
        }
        if self.store.is_none() {
            let store = DevelopmentStoreOwner::open_scoped(
                &self.root,
                self.storage_scope,
                self.secret.clone(),
                ScopedOpen::Existing,
            )
            .map_err(error)?;
            store
                .initialize_groups_blocking(IdentityHash::new(self.storage_scope.public_hash.0))
                .map_err(error)?;
            self.store = Some(store);
        }
        self.dependencies = None;
        Ok(())
    }
    async fn register(&mut self) -> Result<(), Error> {
        if self.storage_close_failed {
            return Err(error("storage close failed; restart recovery required"));
        }
        if self.network.is_some() {
            return Ok(());
        }
        let identity = IdentityHash::new(self.storage_scope.public_hash.0);
        let local = LocalLxmfIdentity::from_secret_bytes(&self.secret).map_err(error)?;
        let direct_destination = DestinationHash::new(local.destination());
        let sync_destination = prns_groups_reticulum::destination_for(identity).map_err(error)?;
        let enrollment_destination =
            prns_groups_reticulum::enrollment::destination_for(identity).map_err(error)?;
        let configs = [
            ("lxmf", vec!["delivery"], Vec::new()),
            (
                "prns",
                vec!["groups"],
                vec![(prns_groups_reticulum::REQUEST_PATH, false)],
            ),
            (
                "prns",
                vec!["groups", "enrollment"],
                vec![(prns_groups_reticulum::enrollment::REQUEST_PATH, true)],
            ),
        ];
        let handle = self
            .host
            .native_services()
            .map_err(error)?
            .protocols()
            .clone();
        while self.endpoints.len() < configs.len() {
            self.endpoints.push(handle.service_endpoint());
        }
        for (index, (app, aspects, handlers)) in configs.into_iter().enumerate() {
            if index < self.registered {
                continue;
            }
            let config = ServiceRegistration {
                initially_accepting: false,
                secret: self.secret.clone(),
                app_name: app.into(),
                aspects: aspects.into_iter().map(str::to_owned).collect(),
                app_data: if index == 0 {
                    let name = self
                        .store()?
                        .read_messaging_name_blocking()
                        .map_err(error)?;
                    if name.is_empty() {
                        Vec::new()
                    } else {
                        crate::messaging_profile::announce_data(&name).map_err(error)?
                    }
                } else {
                    Vec::new()
                },
                proof_strategy:
                    personal_rns::routing::upstream_app_destinations::ProofStrategy::ProveAll,
                link_request_policy:
                    personal_rns::routing::upstream_app_destinations::LinkRequestPolicy::AcceptAll,
                ratchet_policy: personal_rns::crypto::ratchets::RatchetPolicy::NoRatchets,
                resource_strategy:
                    personal_rns::routing::links::resources::ResourceStrategy::AcceptNone,
                request_handlers: handlers
                    .into_iter()
                    .map(|(path, allow_all)| {
                        (
                            personal_rns::routing::request_handlers::RequestPathHash::of(path),
                            if allow_all {
                                personal_rns::routing::request_handlers::RequestPolicy::AllowAll
                            } else {
                                personal_rns::routing::request_handlers::RequestPolicy::AllowList
                            },
                        )
                    })
                    .collect(),
                maximum_request_bytes: personal_rns::units::ByteLimit::Maximum(if index == 0 {
                    0
                } else if index == 1 {
                    65_600
                } else {
                    131_136
                }),
                restore_ratchets: None,
            };
            let actual = self.endpoints[index]
                .register(config)
                .await
                .map_err(error)?;
            let expected = [direct_destination, sync_destination, enrollment_destination][index];
            if actual.as_bytes() != expected.as_bytes() {
                return Err(error("service address mismatch"));
            }
            self.registered += 1;
        }
        let network = NativeServiceScope::new(
            self.host.clone(),
            identity,
            vec![direct_destination, sync_destination, enrollment_destination],
            MAX_OPERATIONS,
            MAX_LINKS,
        )
        .map_err(error)?;
        let (groups, group_jobs) =
            prns_groups_reticulum::Ingress::new(sync_destination, LIMITS).map_err(error)?;
        let (enrollment, enrollment_jobs) = prns_groups_reticulum::enrollment::Ingress::new(
            enrollment_destination,
            ENROLLMENT_LIMITS,
        )
        .map_err(error)?;
        let (pending, callbacks) = DurableDirectLxmfService::prepare(local);
        let direct = pending
            .start_paused(
                Arc::new(PrnsDirectNetwork::scoped(network.clone())),
                self.store
                    .as_ref()
                    .ok_or_else(|| error("store missing"))?
                    .mailbox_submitter(),
                Arc::new(SystemMailboxClock),
            )
            .await
            .map_err(error)?;
        let mut events = self.events.lock().map_err(error)?;
        let published_name = events.published_name.take();
        *events = Events {
            published_name,
            enabled: false,
            direct_enabled: false,
            scope: Some(network.clone()),
            direct: Some(callbacks),
            groups: Some(groups),
            enrollment: Some(enrollment),
        };
        drop(events);
        self.direct = Some(Arc::new(direct));
        self.inbound = Some((group_jobs, enrollment_jobs));
        self.network = Some(network);
        Ok(())
    }
    /// Called by the driver only after the core and runtime confirm Network admission.
    async fn start_network(&mut self) -> Result<(), Error> {
        let network = self
            .network
            .clone()
            .ok_or_else(|| error("not registered"))?;
        network.check_open().map_err(error)?;
        if let Some(task) = &self.task {
            if task.is_finished() {
                return Err(error("group worker exited; pause before recovery"));
            }
            // A canceled previous call can leave the group worker running while
            // the direct activation was still awaiting its transition gate.
            return if self
                .direct
                .as_ref()
                .ok_or_else(|| error("direct missing"))?
                .activate_queued_attempts()
                .await
            {
                Ok(())
            } else {
                Err(error("direct service cannot activate"))
            };
        }
        let client = self.groups()?;
        let identity = network.identity_hash();
        {
            let mut events = self.events.lock().map_err(error)?;
            events.enabled = true;
            events.direct_enabled = true;
        }
        for endpoint in &self.endpoints {
            endpoint.activate_ingress().await.map_err(error)?;
        }
        let (jobs, enrollment_jobs) = self
            .inbound
            .take()
            .ok_or_else(|| error("ingress missing"))?;
        let (stop, receiver) = watch::channel(false);
        let (announce_control, announces) =
            prns_groups_runner_native::announce_control(None).map_err(error)?;
        let sync_destination = prns_groups_reticulum::destination_for(identity).map_err(error)?;
        let enrollment_destination =
            prns_groups_reticulum::enrollment::destination_for(identity).map_err(error)?;
        let health = self.store()?.group_health();
        health.set(crate::contract::GroupSynchronizationState::Running);
        self.task = Some(tokio::spawn(async move {
            let result = prns_groups_runner_native::run_multi_scoped_with_announcements(
                client,
                network,
                sync_destination,
                enrollment_destination,
                jobs,
                enrollment_jobs,
                LIMITS,
                ENROLLMENT_LIMITS,
                Default::default(),
                announces,
                receiver,
            )
            .await;
            health.set(match &result {
                Ok(()) => crate::contract::GroupSynchronizationState::Stopped,
                Err(error) => crate::contract::GroupSynchronizationState::Unavailable {
                    detail: error.to_string(),
                },
            });
            result
        }));
        self.stop = Some(stop);
        self.announcements = Some(announce_control);
        if !self
            .direct
            .as_ref()
            .ok_or_else(|| error("direct missing"))?
            .activate_queued_attempts()
            .await
        {
            return Err(error("direct service cannot activate"));
        }
        Ok(())
    }
    async fn observe_dependencies(&self) -> Result<ServiceDependency, Error> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| error("store closed before dependency observation"))?;
        let MailboxReply::Listed { messages, revision } = store
            .mailbox_submitter()
            .submit(MailboxRequest::ListQueued)
            .await
            .map_err(error)?
        else {
            return Err(error("unexpected mailbox reply"));
        };
        let inspection = store.inspection_group_client();
        let mut groups = inspection.conversations().await.map_err(error)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(error)?
            .as_secs();
        let mut invitations = inspection.invitations(now).await.map_err(error)?;
        let mut work = inspection.enrollment_work(now).await.map_err(error)?;
        // Group-owner departure and complete generic-record retention review are
        // separate protocols. Conservatively retain any enrolled identity until
        // those profile-specific retirement hooks can prove obligations settled.
        let crate::group_actions::Reply::List(mut actions) = store
            .group_action_client()
            .execute(crate::group_actions::Request::List)
            .await
            .map_err(error)?
        else {
            return Err(error("unexpected action journal reply"));
        };
        groups.sort_by_key(|group| group.group.0);
        invitations.sort_by_key(|invitation| invitation.id);
        work.sort_by_key(|item| item.id);
        actions.sort_by_key(|action| crate::group_actions::request_id(&action.input));
        let pending_actions = actions
            .iter()
            .filter(|action| {
                !matches!(
                    action.state,
                    crate::contract::GroupActionState::Completed { .. }
                )
            })
            .count();
        // A durable observation revision changes only when dependency facts do.
        // It includes accepted UI intents, which can exist before a group does.
        let mut facts = revision.to_le_bytes().to_vec();
        facts.extend_from_slice(&(groups.len() as u64).to_le_bytes());
        for group in &groups {
            facts.extend_from_slice(&group.group.0);
            facts.extend_from_slice(&group.revision.to_le_bytes());
            facts.extend_from_slice(&group.control_head.0);
            facts.push(u8::from(group.can_invite));
        }
        facts.extend_from_slice(&(invitations.len() as u64).to_le_bytes());
        for invitation in &invitations {
            facts.extend_from_slice(&invitation.id);
            facts.push(invitation.state as u8);
        }
        facts.extend_from_slice(&(work.len() as u64).to_le_bytes());
        for item in &work {
            facts.extend_from_slice(&item.id);
            facts.extend_from_slice(&prns_core::crypto::sha256(&item.frame));
        }
        facts.extend_from_slice(&serde_json::to_vec(&actions).map_err(error)?);
        let revision = store
            .admit_dependency_revision(prns_core::crypto::sha256(&facts))
            .map_err(error)?
            .await
            .map_err(error)?
            .map_err(error)?;
        let mut result = ServiceDependency::clear(MESSAGING_SERVICE, revision);
        result.pending_messages = u32::try_from(messages.len()).map_err(error)?;
        result.authority_count =
            u32::try_from(groups.iter().filter(|group| group.can_invite).count()).map_err(error)?;
        result.unsettled_control = u32::try_from(
            work.len()
                .checked_add(pending_actions)
                .ok_or_else(|| error("dependency overflow"))?,
        )
        .map_err(error)?;
        result.recovery_pending =
            !groups.is_empty() || !invitations.is_empty() || pending_actions != 0;
        Ok(result)
    }
    pub(crate) async fn pause(&mut self) -> Result<ServiceDependency, Error> {
        self.fence()?;
        if self.storage_close_failed {
            return Err(error("storage close failed; restart recovery required"));
        }
        if let Some(task) = self.task.as_mut() {
            // Keep the JoinHandle on cancellation/error; never infer join from dropping it.
            let _completed = task.await;
            self.task = None;
        }
        if self.pause_outgoing.is_none() {
            self.pause_outgoing = Some(if let Some(network) = &self.network {
                network.wait_drained().await;
                network
                    .outgoing_links()
                    .map_err(error)?
                    .into_iter()
                    .map(|link| personal_rns::routing::links::LinkId::new(link.into_bytes()))
                    .collect()
            } else {
                Vec::new()
            });
        }
        for index in self.paused_endpoints..self.endpoints.len() {
            // Retain both the original Link list and returned receipt through
            // errors. LinkClosed callbacks change live tracking after prepare.
            let pause = if let Some(pause) = &self.prepared_pause {
                pause.clone()
            } else {
                let pause = match self.endpoints[index]
                    .prepare_pause(if index == 0 {
                        self.pause_outgoing
                            .clone()
                            .ok_or_else(|| error("pause snapshot missing"))?
                    } else {
                        Vec::new()
                    })
                    .await
                {
                    Ok(pause) => pause,
                    Err(personal_rns::runtime::ServiceLifecycleError::NotRegistered) => {
                        self.paused_endpoints += 1;
                        continue;
                    }
                    Err(failure) => return Err(error(failure)),
                };
                self.prepared_pause = Some(pause.clone());
                pause
            };
            if let Some(bytes) = pause.ratchets() {
                // Current direct/group profiles explicitly use NoRatchets. Keep
                // unexpected secret state durable rather than silently discarding it.
                self.store
                    .as_ref()
                    .ok_or_else(|| error("store missing"))?
                    .admit_receive_state(
                        *pause.destination().as_bytes(),
                        Some(zeroize::Zeroizing::new(bytes.to_vec())),
                    )
                    .map_err(error)?
                    .await
                    .map_err(error)?
                    .map_err(error)?;
            }
            while !pause.is_drained() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.endpoints[index]
                .finish_pause(&pause)
                .await
                .map_err(error)?;
            self.prepared_pause = None;
            self.paused_endpoints += 1;
        }
        // The host delivers callbacks synchronously before acknowledging ingress
        // retirement. Keep direct callbacks alive until then, and drain their
        // accepted writes before closing the durable lane.
        self.events.lock().map_err(error)?.direct_enabled = false;
        if let Some(direct) = &self.direct {
            direct.stop().await.map_err(error)?;
        }
        self.registered = 0;
        self.paused_endpoints = 0;
        self.pause_outgoing = None;
        self.endpoints.clear();
        self.inbound = None;
        self.direct = None;
        self.network = None;
        self.stop = None;
        self.announcements = None;
        {
            let mut events = self.events.lock().map_err(error)?;
            let published_name = events.published_name.take();
            *events = Events {
                published_name,
                ..Events::default()
            };
        }
        if let Some(store) = &self.store {
            store.fence_all_mutations();
            store.drain().await.map_err(error)?;
            self.dependencies = Some(self.observe_dependencies().await?);
        }
        if let Some(store) = self.store.take() {
            if let Err(failure) = store.close() {
                self.storage_close_failed = true;
                return Err(error(failure));
            }
        }
        self.dependencies
            .ok_or_else(|| error("missing retained dependency evidence"))
    }
}
impl Drop for MessagingIdentity {
    fn drop(&mut self) {
        // Dropping is an emergency admission fence, never a successful pause.
        let _ = self.fence();
    }
}

#[cfg(test)]
mod tests;
