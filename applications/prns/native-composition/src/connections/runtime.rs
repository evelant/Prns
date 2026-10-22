//! One generation's applied attachments. Durable settings remain the authority.
use super::{endpoint, StoredSettings};
use crate::contract::*;
use prns_host::{
    Bitrate, CommandOutcome, HostCommand, InterfaceConfig, InterfaceHealth, InterfaceId,
};
use prns_host_native::owner::HostClient;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex as StateMutex, MutexGuard};
use std::task::Poll;
use tokio::sync::Mutex;

pub(crate) struct ConnectionBootstrap {
    pub(crate) owner_id: u64,
    pub(crate) saved: Result<StoredSettings, String>,
}

#[derive(Clone)]
struct Attachment {
    config: InterfaceConfig,
    interface: Option<InterfaceId>,
    detail: Option<String>,
    retiring: bool,
    profile: Option<TcpPeerProfile>,
}

#[derive(Clone)]
struct State {
    saved: StoredSettings,
    applied_revision: Option<u64>,
    attachments: BTreeMap<String, Attachment>,
    inspection: CachedInspection,
}

pub(crate) struct ConnectionRuntime {
    owner_id: u64,
    generation_id: u64,
    host: HostClient,
    active: AtomicBool,
    state: StateMutex<State>,
    reconciliation: Mutex<()>,
}

impl ConnectionRuntime {
    pub(crate) fn new(host: HostClient, owner_id: u64, generation_id: u64) -> Self {
        Self {
            owner_id,
            generation_id,
            host,
            active: AtomicBool::new(true),
            state: StateMutex::new(State {
                saved: StoredSettings::default(),
                applied_revision: None,
                attachments: BTreeMap::new(),
                inspection: CachedInspection::default(),
            }),
            reconciliation: Mutex::new(()),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Called synchronously by the storage completion, before its apply task is
    /// spawned. Retry and queued work immediately observe committed newer intent.
    pub(crate) fn publish_desired(&self, saved: &StoredSettings) {
        let mut state = self.state();
        if saved.revision >= state.saved.revision && *saved != state.saved {
            state.saved = saved.clone();
            state.applied_revision = None;
        }
    }

    fn is_current(&self, revision: u64) -> bool {
        self.active.load(Ordering::Acquire) && self.state().saved.revision == revision
    }

    pub(crate) async fn apply(&self, saved: StoredSettings) {
        self.publish_desired(&saved);
        let _reconciliation = self.reconciliation.lock().await;
        self.reconcile_latest().await;
    }

    async fn reconcile_latest(&self) {
        while self.active.load(Ordering::Acquire) {
            let saved = self.state().saved.clone();
            self.reconcile_revision(&saved).await;
            if self.state().saved.revision == saved.revision {
                return;
            }
        }
    }

    async fn reconcile_revision(&self, saved: &StoredSettings) {
        let mut desired = BTreeMap::new();
        if saved.settings.lan_enabled {
            desired.insert("lan".to_owned(), (lan_config(), None));
        }
        for profile in &saved.settings.tcp_peers {
            if profile.enabled {
                desired.insert(
                    profile.id.clone(),
                    (tcp_config(profile), Some(profile.clone())),
                );
            }
        }
        let obsolete: Vec<_> = self
            .state()
            .attachments
            .iter()
            .filter(|(id, entry)| {
                desired
                    .get(*id)
                    .is_none_or(|(config, _)| *config != entry.config)
                    || entry.retiring
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in obsolete {
            if !self.is_current(saved.revision) {
                return;
            }
            self.detach(&id).await;
        }
        for (id, (config, profile)) in &desired {
            {
                let mut state = self.state();
                if !self.active.load(Ordering::Acquire) || state.saved.revision != saved.revision {
                    return;
                }
                if let Some(existing) = state.attachments.get_mut(id) {
                    if existing.interface.is_some() || existing.retiring {
                        if !existing.retiring && existing.config == *config {
                            existing.profile = profile.clone();
                        }
                        continue;
                    }
                }
                // Runtime IDs derive from effective configuration, not profile
                // IDs. A removed/recreated profile must not bypass retirement.
                if retained_config_owner(&state.attachments, id, config).is_some() {
                    continue;
                }
                state.attachments.insert(
                    id.clone(),
                    Attachment {
                        config: config.clone(),
                        interface: None,
                        detail: None,
                        retiring: false,
                        profile: profile.clone(),
                    },
                );
            }
            let Some(outcome) = self
                .execute_current(
                    saved.revision,
                    HostCommand::AttachInterface {
                        config: config.clone(),
                        routing: None,
                    },
                )
                .await
            else {
                self.state().attachments.remove(id);
                return;
            };
            let (interface, detail) = match outcome {
                Ok(Ok(CommandOutcome::InterfaceAttached { interface })) => (Some(interface), None),
                outcome => (
                    None,
                    Some(format!("Could not attach this connection: {outcome:?}")),
                ),
            };
            // Even an attachment admitted before a newer commit belongs to this
            // owner until the following reconciliation retires it.
            if let Some(entry) = self.state().attachments.get_mut(id) {
                entry.interface = interface;
                entry.detail = detail;
            }
        }
        let mut state = self.state();
        if state.saved.revision == saved.revision
            && state.attachments.len() == desired.len()
            && desired.iter().all(|(id, (config, _))| {
                state.attachments.get(id).is_some_and(|entry| {
                    entry.interface.is_some() && !entry.retiring && entry.config == *config
                })
            })
        {
            state.applied_revision = Some(saved.revision);
        }
    }

    /// Poll submission once while holding the same short lock as publication.
    /// A command is admitted before a later intent publication or skipped; the
    /// network wait itself never holds the snapshot/storage lock.
    async fn execute_current(
        &self,
        revision: u64,
        command: HostCommand,
    ) -> Option<
        Result<
            Result<CommandOutcome, prns_host::CommandFailure>,
            prns_host_native::owner::SessionError,
        >,
    > {
        let mut operation = Box::pin(self.host.execute(command));
        let first = std::future::poll_fn(|context| {
            let state = self.state();
            if !self.active.load(Ordering::Acquire) || state.saved.revision != revision {
                return Poll::Ready(None);
            }
            Poll::Ready(Some(operation.as_mut().poll(context)))
        })
        .await;
        match first {
            None => None,
            Some(Poll::Ready(outcome)) => Some(outcome),
            Some(Poll::Pending) => Some(operation.await),
        }
    }

    async fn detach(&self, id: &str) -> bool {
        let interface = {
            let mut state = self.state();
            let Some(entry) = state.attachments.get_mut(id) else {
                return true;
            };
            entry.retiring = true;
            let interface = entry.interface;
            state.applied_revision = None;
            if interface.is_none() {
                state.attachments.remove(id);
            }
            interface
        };
        let Some(interface) = interface else {
            return true;
        };
        let outcome = self
            .host
            .execute(HostCommand::DetachInterface { interface })
            .await;
        let mut state = self.state();
        match outcome {
            Ok(Ok(CommandOutcome::InterfaceDetached { .. })) => {
                state.attachments.remove(id);
                state.inspection.retire(interface);
                true
            }
            outcome => {
                if let Some(entry) = state.attachments.get_mut(id) {
                    entry.detail =
                        Some(format!("Connection shutdown is not confirmed: {outcome:?}"));
                }
                false
            }
        }
    }

    pub(crate) async fn retry(&self, input: &RetryConnectionInput) -> Result<(), String> {
        let _reconciliation = self.reconciliation.lock().await;
        {
            let state = self.state();
            if !self.active.load(Ordering::Acquire)
                || self.owner_id != input.owner_id
                || self.generation_id != input.generation_id
                || state.saved.revision != input.expected_revision
            {
                return Err(
                    "Connection settings or the native owner changed. Refresh before retrying."
                        .into(),
                );
            }
            let enabled = if input.profile_id == "lan" {
                state.saved.settings.lan_enabled
            } else {
                state
                    .saved
                    .settings
                    .tcp_peers
                    .iter()
                    .any(|peer| peer.id == input.profile_id && peer.enabled)
            };
            if !enabled {
                return Err("This connection is disabled or no longer saved.".into());
            }
        }
        if !self.detach(&input.profile_id).await {
            return Err("The previous connection is still retiring. Refresh its status.".into());
        }
        let changed = !self.is_current(input.expected_revision);
        self.reconcile_latest().await;
        if changed {
            Err("Connection settings changed during retry. The latest saved settings were retained.".into())
        } else {
            Ok(())
        }
    }

    pub(crate) async fn stop(&self) {
        self.active.store(false, Ordering::Release);
        let _reconciliation = self.reconciliation.lock().await;
        let ids: Vec<_> = self.state().attachments.keys().cloned().collect();
        for id in ids {
            self.detach(&id).await;
        }
        self.state().applied_revision = None;
    }

    pub(crate) async fn snapshot(
        &self,
        settings: ConnectionSettingsSnapshot,
    ) -> Result<ConnectionSnapshot, String> {
        if settings.owner_id != self.owner_id {
            return Err(
                "The native connection settings owner changed. Refresh connections.".into(),
            );
        }
        // A read may race a completed commit or a queued apply. Publish the
        // committed read too, and always project the newest known desired state.
        self.publish_desired(&StoredSettings {
            version: super::VERSION,
            revision: settings.revision,
            settings: settings.settings,
        });
        if let Ok(_idle) = self.reconciliation.try_lock() {
            let inspection = self
                .host
                .inspection()
                .await
                .map_err(|error| format!("Connection inspection unavailable: {error:?}"))?;
            let attachments = inspection.interfaces.iter().filter_map(|entry| {
                inspection
                    .host
                    .interfaces
                    .iter()
                    .find(|interface| {
                        interface.interface_id.as_bytes() == entry.snapshot.id.as_bytes()
                    })
                    .and_then(|interface| interface.kind)
                    .map(|kind| {
                        prns_host_native::HostInterfaceAttachment::new(entry.snapshot.id, kind)
                    })
            });
            let topology =
                prns_host_native::detailed_interface_topology(&inspection.interfaces, attachments)
                    .map_err(|error| format!("Connection topology unavailable: {error:?}"))?;
            self.state().inspection = CachedInspection {
                interfaces: inspection.host.interfaces,
                topology,
            };
        }
        // While retirement is awaiting a host/provider receipt, a cached wire
        // observation remains available. Desired and retiring state is fresh.
        let state = self.state().clone();
        let settings = ConnectionSettingsSnapshot {
            owner_id: self.owner_id,
            revision: state.saved.revision,
            settings: state.saved.settings.clone(),
        };
        let inspection = &state.inspection;
        let running = self.active.load(Ordering::Acquire);
        let mut tcp_peers = Vec::new();
        for profile in &settings.settings.tcp_peers {
            tcp_peers.push(tcp_snapshot(
                profile.clone(),
                state.attachments.get(&profile.id),
                &inspection.interfaces,
                running,
            ));
        }
        for (id, attachment) in &state.attachments {
            if !settings
                .settings
                .tcp_peers
                .iter()
                .any(|profile| &profile.id == id)
            {
                if let Some(profile) = attachment.profile.clone() {
                    let mut retiring = attachment.clone();
                    retiring.retiring = true;
                    tcp_peers.push(tcp_snapshot(
                        profile,
                        Some(&retiring),
                        &inspection.interfaces,
                        running,
                    ));
                }
            }
        }
        let lan_attachment = state.attachments.get("lan");
        let mut physical_connections = Vec::new();
        // Cached detailed rows retain raw membership and actual endpoint facts.
        let topology = inspection.topology.clone();
        for connection in topology {
            if matches!(
                connection.role,
                prns_host_native::HostConnectionRole::Supervisor
            ) {
                continue;
            }
            let Some(kind) = connection.runtime_kind else {
                continue;
            };
            if !matches!(
                kind,
                prns_core::interfaces::InterfaceKind::TcpClient
                    | prns_core::interfaces::InterfaceKind::TcpServerPeer
                    | prns_core::interfaces::InterfaceKind::WifiPeer
            ) {
                continue;
            }
            let association = state.attachments.iter().find_map(|(id, entry)| {
                entry
                    .interface
                    .filter(|interface| {
                        *interface == connection.logical_interface
                            || *interface == connection.runtime_interface
                            || Some(*interface) == connection.attachment
                    })
                    .map(|_| {
                        (
                            id.clone(),
                            entry.retiring || wants_retirement(&settings.settings, id, entry),
                        )
                    })
            });
            let profile_id = association.as_ref().map(|(id, _)| id.clone());
            let local_only = connection
                .endpoint
                .as_deref()
                .is_some_and(is_loopback_endpoint);
            physical_connections.push(PhysicalConnectionSnapshot {
                interface_id: connection.runtime_interface.as_bytes().to_vec(),
                logical_interface_id: connection
                    .attachment
                    .unwrap_or(connection.logical_interface)
                    .as_bytes()
                    .to_vec(),
                parent_interface_id: connection.parent.map(|id| id.as_bytes().to_vec()),
                profile_id,
                kind: format!("{kind:?}"),
                name: connection.name,
                state: if !running {
                    LocalConnectionState::Stopped
                } else if association.is_some_and(|(_, retiring)| retiring) {
                    LocalConnectionState::Retiring
                } else {
                    health(connection.health)
                },
                rx_bytes: connection.rx_bytes,
                tx_bytes: connection.tx_bytes,
                endpoint: connection.endpoint,
                local_only,
            });
        }
        let count = physical_connections
            .iter()
            .filter(|peer| {
                peer.profile_id.as_deref() == Some("lan")
                    && peer.state == LocalConnectionState::Connected
                    && !peer.local_only
            })
            .count() as u32;
        let (discovery_method, stock_multicast_available) = discovery_capability();
        let discovery = prns_host_native::platform::mobile_auto_lan_status();
        let lan = LanConnectionSnapshot {
            desired_enabled: settings.settings.lan_enabled,
            state: lan_state(
                running,
                settings.settings.lan_enabled,
                lan_attachment,
                count,
                discovery.discovery_ready,
                discovery.observed_error.is_some(),
            ),
            discovery_method,
            stock_multicast_available,
            connected_peer_count: count,
            detail: lan_attachment
                .and_then(|entry| entry.detail.clone())
                .or(discovery.observed_error)
                .or(discovery.detail),
        };
        Ok(ConnectionSnapshot {
            generation_id: self.generation_id,
            settings,
            applied_revision: state
                .applied_revision
                .filter(|revision| *revision == state.saved.revision),
            lan,
            tcp_peers,
            physical_connections,
        })
    }
}

fn lan_state(
    running: bool,
    desired: bool,
    attachment: Option<&Attachment>,
    connected_peers: u32,
    discovery_ready: Option<bool>,
    observed_error: bool,
) -> LocalConnectionState {
    if !running {
        LocalConnectionState::Stopped
    } else if attachment.is_some_and(|entry| entry.retiring)
        || (!desired && attachment.is_some_and(|entry| entry.interface.is_some()))
    {
        LocalConnectionState::Retiring
    } else if !desired {
        LocalConnectionState::Disabled
    } else if attachment.is_none() {
        LocalConnectionState::Applying
    } else if attachment.is_some_and(|entry| entry.interface.is_none() && entry.detail.is_some()) {
        LocalConnectionState::Failed
    } else if connected_peers > 0 {
        LocalConnectionState::Connected
    } else if discovery_ready == Some(false) && observed_error {
        LocalConnectionState::Unavailable
    } else if discovery_ready == Some(false)
        || attachment.is_some_and(|entry| entry.interface.is_none())
    {
        LocalConnectionState::Applying
    } else {
        // Opportunistic gateway dials and other child connections can retry
        // while discovery is healthy. Their folded health is not readiness.
        LocalConnectionState::Discovering
    }
}

#[derive(Clone, Default)]
struct CachedInspection {
    interfaces: Vec<prns_host::InterfaceSnapshot>,
    topology: Vec<prns_host_native::HostConnectionSnapshot>,
}

impl CachedInspection {
    fn retire(&mut self, interface: InterfaceId) {
        self.interfaces
            .retain(|entry| entry.interface_id != interface);
        self.topology.retain(|entry| {
            entry.runtime_interface != interface
                && entry.logical_interface != interface
                && entry.attachment != Some(interface)
        });
    }
}

fn tcp_config(profile: &TcpPeerProfile) -> InterfaceConfig {
    InterfaceConfig::TcpClient {
        target: endpoint(profile),
        bitrate: Bitrate::Auto,
    }
}

fn wants_retirement(settings: &ConnectionSettings, id: &str, attachment: &Attachment) -> bool {
    if id == "lan" {
        !settings.lan_enabled
    } else {
        !settings.tcp_peers.iter().any(|profile| {
            profile.id == id && profile.enabled && tcp_config(profile) == attachment.config
        })
    }
}

fn retained_config_owner<'a>(
    attachments: &'a BTreeMap<String, Attachment>,
    id: &str,
    config: &InterfaceConfig,
) -> Option<&'a str> {
    attachments.iter().find_map(|(other, entry)| {
        (other != id && entry.config == *config && (entry.interface.is_some() || entry.retiring))
            .then_some(other.as_str())
    })
}

pub(crate) fn stopped_snapshot(
    generation_id: u64,
    settings: ConnectionSettingsSnapshot,
) -> ConnectionSnapshot {
    let tcp_peers = settings
        .settings
        .tcp_peers
        .iter()
        .cloned()
        .map(|profile| tcp_snapshot(profile, None, &[], false))
        .collect();
    let (discovery_method, stock_multicast_available) = discovery_capability();
    ConnectionSnapshot {
        generation_id,
        applied_revision: None,
        lan: LanConnectionSnapshot {
            desired_enabled: settings.settings.lan_enabled,
            state: LocalConnectionState::Stopped,
            discovery_method,
            detail: None,
            stock_multicast_available,
            connected_peer_count: 0,
        },
        settings,
        tcp_peers,
        physical_connections: Vec::new(),
    }
}

fn tcp_snapshot(
    profile: TcpPeerProfile,
    attachment: Option<&Attachment>,
    interfaces: &[prns_host::InterfaceSnapshot],
    running: bool,
) -> TcpPeerConnectionSnapshot {
    let matching = attachment.filter(|entry| entry.config == tcp_config(&profile));
    let interface = matching.and_then(|entry| entry.interface).and_then(|id| {
        interfaces
            .iter()
            .find(|interface| interface.interface_id == id)
    });
    TcpPeerConnectionSnapshot {
        state: if !running {
            LocalConnectionState::Stopped
        } else if attachment.is_some_and(|entry| entry.retiring) {
            LocalConnectionState::Retiring
        } else if !profile.enabled && attachment.is_some_and(|entry| entry.interface.is_some()) {
            LocalConnectionState::Retiring
        } else if !profile.enabled {
            LocalConnectionState::Disabled
        } else if matching.is_none() {
            LocalConnectionState::Applying
        } else if matching.is_some_and(|entry| entry.interface.is_none() && entry.detail.is_some())
        {
            LocalConnectionState::Failed
        } else {
            interface
                .map(|interface| health(interface.health))
                .unwrap_or(LocalConnectionState::Applying)
        },
        interface_id: matching
            .and_then(|entry| entry.interface)
            .map(|id| id.as_bytes().to_vec()),
        detail: attachment
            .and_then(|entry| entry.detail.clone())
            .or_else(|| interface.and_then(|interface| interface.failure_detail.clone())),
        rx_bytes: interface.map_or(0, |interface| interface.rx_bytes),
        tx_bytes: interface.map_or(0, |interface| interface.tx_bytes),
        profile,
    }
}

fn lan_config() -> InterfaceConfig {
    InterfaceConfig::AutoLan {
        group_id: None,
        discovery_scope: None,
        discovery_port: None,
        data_port: None,
        devices: Vec::new(),
        ignored_devices: Vec::new(),
        multicast_address_type: None,
    }
}

pub(crate) fn discovery_capability() -> (LanDiscoveryMethod, bool) {
    let status = prns_host_native::platform::mobile_auto_lan_status();
    let method = match status.method {
        "Bonjour" => LanDiscoveryMethod::Bonjour,
        "MulticastAndBonjour" => LanDiscoveryMethod::Combined,
        _ => LanDiscoveryMethod::StockMulticast,
    };
    (method, status.stock_multicast_available)
}

fn is_loopback_endpoint(endpoint: &str) -> bool {
    endpoint
        .parse::<std::net::SocketAddr>()
        .is_ok_and(|address| address.ip().is_loopback())
}

fn health(health: InterfaceHealth) -> LocalConnectionState {
    match health {
        InterfaceHealth::Connected => LocalConnectionState::Connected,
        InterfaceHealth::Reconnecting => LocalConnectionState::Reconnecting,
        InterfaceHealth::Disconnected => LocalConnectionState::Reconnecting,
        InterfaceHealth::Disabled => LocalConnectionState::Disabled,
        InterfaceHealth::Failed | InterfaceHealth::Degraded => LocalConnectionState::Failed,
        InterfaceHealth::Initializing => LocalConnectionState::Connecting,
        InterfaceHealth::Unknown => LocalConnectionState::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prns_host_native::owner::OwnedSession;
    use prns_host_native::{NativeEmbedding, NativePreparedAttachment};
    use std::sync::Arc;
    use std::time::Duration;

    fn peer(id: &str, host: &str) -> TcpPeerProfile {
        TcpPeerProfile {
            id: id.into(),
            label: id.into(),
            host: host.into(),
            port: 4242,
            enabled: true,
        }
    }

    fn saved(revision: u64, lan: bool) -> StoredSettings {
        StoredSettings {
            version: super::super::VERSION,
            revision,
            settings: ConnectionSettings {
                lan_enabled: lan,
                tcp_peers: vec![],
            },
        }
    }

    fn connected(interface: InterfaceId) -> prns_host::InterfaceSnapshot {
        prns_host::InterfaceSnapshot {
            interface_id: interface,
            name: None,
            kind: Some(prns_host::InterfaceKind::TcpClient),
            health: InterfaceHealth::Connected,
            failure_detail: None,
            rx_bytes: 89,
            tx_bytes: 34,
            rx_bps: None,
            tx_bps: None,
            route_count: 0,
            link_count: 0,
            transported_link_count: 0,
        }
    }

    fn attachment(profile: &TcpPeerProfile) -> Attachment {
        Attachment {
            config: tcp_config(profile),
            interface: Some(InterfaceId::new([7; 8])),
            detail: None,
            retiring: false,
            profile: Some(profile.clone()),
        }
    }

    #[test]
    fn discovery_readiness_is_independent_of_reconnecting_gateway_probes() {
        let mut lan = attachment(&peer("lan", "gateway.example"));
        lan.config = lan_config();
        assert_eq!(
            lan_state(true, true, Some(&lan), 0, Some(true), false),
            LocalConnectionState::Discovering
        );
        // A Bonjour warning does not disable Android's healthy multicast lane.
        assert_eq!(
            lan_state(true, true, Some(&lan), 0, Some(true), true),
            LocalConnectionState::Discovering
        );
        assert_eq!(
            lan_state(true, true, Some(&lan), 0, Some(false), true),
            LocalConnectionState::Unavailable
        );
        assert_eq!(
            lan_state(true, true, Some(&lan), 1, Some(false), true),
            LocalConnectionState::Connected
        );
        assert_eq!(
            lan_state(true, true, Some(&lan), 0, Some(false), false),
            LocalConnectionState::Applying
        );
        assert_eq!(
            lan_state(true, false, Some(&lan), 0, Some(true), false),
            LocalConnectionState::Retiring
        );
    }

    #[test]
    fn new_profile_id_cannot_bypass_unconfirmed_effective_config_retirement() {
        let old = peer("old", "peer.example");
        let replacement = peer("new", "peer.example");
        let mut old_attachment = attachment(&old);
        old_attachment.retiring = true;
        old_attachment.detail = Some("provider shutdown unconfirmed".into());
        let mut entries = BTreeMap::new();
        entries.insert(old.id.clone(), old_attachment);
        assert_eq!(
            retained_config_owner(&entries, &replacement.id, &tcp_config(&replacement)),
            Some("old")
        );
        entries.remove("old");
        assert_eq!(
            retained_config_owner(&entries, &replacement.id, &tcp_config(&replacement)),
            None
        );
    }

    #[test]
    fn edited_endpoint_cannot_inherit_connected_state_or_counters_from_old_socket() {
        let old = peer("same-id", "old.example");
        let new = peer("same-id", "new.example");
        let wire = attachment(&old);
        let projected = tcp_snapshot(
            new,
            Some(&wire),
            &[connected(wire.interface.unwrap())],
            true,
        );
        assert_eq!(projected.state, LocalConnectionState::Applying);
        assert_eq!(projected.interface_id, None);
        assert_eq!((projected.rx_bytes, projected.tx_bytes), (0, 0));
    }

    #[test]
    fn committed_disable_remains_retiring_until_socket_retirement_is_confirmed() {
        let mut desired = peer("same-id", "peer.example");
        let wire = attachment(&desired);
        desired.enabled = false;
        let projected = tcp_snapshot(
            desired,
            Some(&wire),
            &[connected(wire.interface.unwrap())],
            true,
        );
        assert_eq!(projected.state, LocalConnectionState::Retiring);
    }

    struct WaitingSupervisor;
    impl personal_rns::interfaces::ReportsStatus for WaitingSupervisor {}
    impl personal_rns::runtime::InterfaceSupervisor for WaitingSupervisor {
        const KIND: personal_rns::interfaces::InterfaceKind =
            personal_rns::interfaces::InterfaceKind::AutoWifi;
        fn channel_tag(&self) -> &[u8] {
            b"connection-retry-fence"
        }
        fn policy(&self) -> personal_rns::interfaces::EffectiveInterfacePolicy {
            personal_rns::interfaces::tcp::policy_for_bitrate(
                personal_rns::interfaces::tcp::TCP_BITRATE_ESTIMATE,
            )
        }
        async fn run(self, _fleet: personal_rns::Fleet) {
            std::future::pending::<()>().await;
        }
    }

    #[tokio::test]
    async fn pending_retirement_is_observable_and_late_disable_fences_retry_reattachment() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let cleanup_entered = entered.clone();
        let cleanup_release = release.clone();
        let embedding = NativeEmbedding {
            prepare_interfaces: Some(Box::new(move |client| {
                let attachment = client.protocols().supervise(WaitingSupervisor);
                let entered = cleanup_entered.clone();
                let release = cleanup_release.clone();
                Ok(vec![NativePreparedAttachment::ManagedSupervisor {
                    attachment,
                    kind: prns_host::InterfaceKind::AutoLan,
                    cleanup: Arc::new(move || {
                        let entered = entered.clone();
                        let release = release.clone();
                        Box::pin(async move {
                            entered.notify_one();
                            tokio::time::timeout(Duration::from_secs(5), release.notified())
                                .await
                                .map_err(|_| "test cleanup release timed out".to_owned())?;
                            Ok(())
                        })
                    }),
                }])
            })),
            ..NativeEmbedding::default()
        };
        let owner = OwnedSession::open_with_embedding(
            prns_host::HostConfig {
                identity: prns_host::IdentityConfig::GenerateEphemeral,
                persistence: prns_host::PersistenceConfig::Ephemeral,
                role: prns_host::HostRole::Endpoint,
                destinations: vec![],
                required_capabilities: vec![],
                limits: prns_host::PrnsLimits::balanced(),
            },
            embedding,
        )
        .await
        .expect("shared test host");
        let manager = Arc::new(ConnectionRuntime::new(owner.client(), 5, 8));
        manager.publish_desired(&saved(1, true));
        let interface = personal_rns::interfaces::InterfaceId::from_channel_tag(
            personal_rns::interfaces::InterfaceKind::AutoWifi,
            b"connection-retry-fence",
        );
        manager.state().attachments.insert(
            "lan".into(),
            Attachment {
                config: lan_config(),
                interface: Some(InterfaceId::new(*interface.as_bytes())),
                detail: None,
                retiring: false,
                profile: None,
            },
        );
        let manager_retry = manager.clone();
        let retry = tokio::spawn(async move {
            manager_retry
                .retry(&RetryConnectionInput {
                    owner_id: 5,
                    generation_id: 8,
                    expected_revision: 1,
                    profile_id: "lan".into(),
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("retirement reached provider");
        // Inspection must not queue behind the host command waiting for cleanup.
        let snapshot = tokio::time::timeout(
            Duration::from_millis(250),
            manager.snapshot(ConnectionSettingsSnapshot {
                owner_id: 5,
                revision: 1,
                settings: saved(1, true).settings,
            }),
        )
        .await
        .expect("retiring snapshot remains responsive")
        .expect("snapshot");
        assert_eq!(snapshot.lan.state, LocalConnectionState::Retiring);
        manager.publish_desired(&saved(2, false));
        release.notify_one();
        assert!(retry.await.expect("retry joined").is_err());
        let snapshot = manager
            .snapshot(ConnectionSettingsSnapshot {
                owner_id: 5,
                revision: 1,
                settings: saved(1, true).settings,
            })
            .await
            .expect("coherent latest snapshot");
        assert_eq!(snapshot.settings.revision, 2);
        assert!(!snapshot.settings.settings.lan_enabled);
        assert_eq!(snapshot.lan.state, LocalConnectionState::Disabled);
        assert_eq!(snapshot.applied_revision, Some(2));
        assert!(manager.state().attachments.is_empty());
        owner.stop().await.expect("joined shared host stop");
    }
}
