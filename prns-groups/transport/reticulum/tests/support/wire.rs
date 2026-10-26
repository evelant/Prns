//! Controlled carrier only: every delivered byte still traverses a real node,
//! path discovery, authenticated Link, request codec and Resource state machine.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use personal_rns::interfaces::{
    AnnounceBandwidthCap, BitrateBps, EgressCapability, IngressCapability, InterfaceCapabilities,
    InterfaceCommonPolicy, InterfaceDescriptor, InterfaceGravity, InterfaceId, InterfaceKind,
    InterfaceMode, ReportsStatus, TransportCapability,
};
use personal_rns::manifold::interface_seam::{Interface, InterfaceSeam};
use personal_rns::runtime::{Attachable, AttachedInterface};
use personal_rns::wire::{PacketType, WireContext, WirePacketHeader, BROADCAST_MTU};
use personal_rns::PrnsNodeHandle;
use prns_groups_reticulum::{
    destination_config, enrollment, Ingress, Limits, Network, Peer, RequestJob,
};
use prns_host::{
    DestinationConfig, HostConfig, HostRole, IdentityConfig, PersistenceConfig, PrnsLimits,
};
use prns_host_native::owner::OwnedSession;
use prns_host_native::service_scope::NativeServiceScope;
use prns_host_native::{ApplicationEventDispatch, NativeEmbedding, NativePreparedAttachment};
use tokio::sync::mpsc;

pub const PEERS: usize = 4;

struct ControlledInterface {
    descriptor: InterfaceDescriptor,
    peer: usize,
    inbound: mpsc::UnboundedReceiver<Vec<u8>>,
    outbound: mpsc::UnboundedSender<(usize, Vec<u8>)>,
}

impl Interface for ControlledInterface {
    const HW_MTU: usize = BROADCAST_MTU;
    const KIND: InterfaceKind = InterfaceKind::Loopback;
    fn descriptor(&self) -> InterfaceDescriptor {
        self.descriptor
    }
    fn channel_tag(&self) -> &[u8] {
        self.descriptor.id.as_bytes()
    }
    async fn run<Seam: InterfaceSeam>(mut self, mut seam: Seam) {
        loop {
            tokio::select! {
                input = self.inbound.recv() => match input {
                    Some(bytes) => seam.next_inbound(&bytes).await,
                    None => return,
                },
                output = seam.next_outbound() => {
                    if self.outbound.send((self.peer, output.to_vec())).is_err() { return; }
                }
            }
        }
    }
}
impl ReportsStatus for ControlledInterface {
    fn status_view(&self) -> Option<personal_rns::interfaces::StatusView> {
        let id = self.descriptor.id;
        Some(Arc::new(move || {
            vec![personal_rns::interfaces::InterfaceVitals {
                endpoint: None,
                id,
                connection: personal_rns::interfaces::ConnectionState::Connected,
                failure_reason: None,
                rx_bytes: 0,
                tx_bytes: 0,
                transfer_rates: None,
                frame_accounting: None,
                radio: personal_rns::interfaces::RadioIndication::for_kind(Some(
                    InterfaceKind::Loopback,
                )),
                details: personal_rns::interfaces::PeerDetails::NotApplicable,
            }]
        }))
    }
}
impl Attachable for ControlledInterface {
    type Attached = AttachedInterface;
    fn attach_to(self, node: &PrnsNodeHandle) -> Self::Attached {
        node.add_interface(self)
    }
    fn attach_to_with_ifac(
        self,
        node: &PrnsNodeHandle,
        ifac: personal_rns::interfaces::IfacContext,
        name: Option<String>,
    ) -> Self::Attached {
        node.add_interface_with_ifac_name(self, ifac, name)
    }
}

fn descriptor(peer: usize) -> InterfaceDescriptor {
    InterfaceDescriptor {
        id: InterfaceId::new([peer as u8 + 0xA0; 8]),
        capabilities: InterfaceCapabilities {
            ingress: IngressCapability::Enabled,
            egress: EgressCapability::Enabled(TransportCapability::CrossInterfaceOnly),
        },
        mode: InterfaceMode::Full,
        gravity: InterfaceGravity::ZERO,
        bitrate: BitrateBps::guess(1_000_000),
        hardware_mtu: None,
        announce_rate_limit: None,
        announce_bandwidth_cap: AnnounceBandwidthCap::Unlimited,
        airtime_duty_cycle: None,
        common: InterfaceCommonPolicy::RNS_DEFAULT,
    }
}

pub fn limits() -> Limits {
    Limits {
        max_frame_bytes: 32_768,
        max_page_items: 8,
        max_peers: PEERS,
        inbound_jobs: 16,
        exchange_timeout: Duration::from_secs(5),
    }
}

pub fn enrollment_limits() -> enrollment::Limits {
    enrollment::Limits {
        max_frame_bytes: enrollment::MAX_FRAME_BYTES,
        inbound_jobs: 4,
        exchange_timeout: Duration::from_secs(5),
    }
}

pub struct WireState {
    online: [AtomicBool; PEERS],
    pub unsolicited_announces: AtomicUsize,
    pub path_responses: AtomicUsize,
    pub resources: AtomicUsize,
    pub resource_proofs: AtomicUsize,
    pub frames: [AtomicUsize; PEERS],
    pub jobs: [AtomicUsize; PEERS],
}
impl WireState {
    pub fn set_online(&self, peer: usize, online: bool) {
        self.online[peer].store(online, Ordering::SeqCst);
    }
}

pub struct Nodes {
    _host_slots: tokio::sync::OwnedSemaphorePermit,
    pub owners: Vec<Arc<OwnedSession>>,
    #[allow(dead_code)] // Used by the scoped enrollment integration test.
    pub scopes: Vec<NativeServiceScope>,
    pub networks: Vec<Arc<Network>>,
    pub jobs: Vec<Option<mpsc::Receiver<RequestJob>>>,
    #[allow(dead_code)] // Used by the enrollment integration binary.
    pub enrollment_networks: Vec<Arc<enrollment::Network>>,
    #[allow(dead_code)] // Used by the enrollment integration binary.
    pub enrollment_jobs: Vec<Option<mpsc::Receiver<RequestJob>>>,
    pub state: Arc<WireState>,
    forwarder: tokio::task::JoinHandle<()>,
}
impl Nodes {
    pub async fn open() -> Self {
        Self::open_impl(false).await
    }
    #[allow(dead_code)] // Used by the scoped enrollment integration test.
    pub async fn open_scoped() -> Self {
        Self::open_impl(true).await
    }
    async fn open_impl(scoped: bool) -> Self {
        // Real host actors use a process-wide bounded registry. Keep concurrent
        // test fixtures below that registry instead of racing unrelated tests.
        static FIXTURES: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
        let host_slots = FIXTURES
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
            .clone()
            .acquire_owned()
            .await
            .unwrap();
        let state = Arc::new(WireState {
            online: std::array::from_fn(|_| AtomicBool::new(true)),
            unsolicited_announces: AtomicUsize::new(0),
            path_responses: AtomicUsize::new(0),
            resources: AtomicUsize::new(0),
            resource_proofs: AtomicUsize::new(0),
            frames: std::array::from_fn(|_| AtomicUsize::new(0)),
            jobs: std::array::from_fn(|_| AtomicUsize::new(0)),
        });
        let (outbound, mut frames) = mpsc::unbounded_channel::<(usize, Vec<u8>)>();
        let mut inbound = Vec::new();
        let mut owners = Vec::new();
        let mut scopes = Vec::new();
        let mut jobs = Vec::new();
        let mut enrollment_jobs = Vec::new();
        for peer in 0..PEERS {
            let (sender, receiver) = mpsc::unbounded_channel();
            inbound.push(sender);
            let interface = ControlledInterface {
                descriptor: descriptor(peer),
                peer,
                inbound: receiver,
                outbound: outbound.clone(),
            };
            let ingress = Arc::new(Mutex::new(None::<Ingress>));
            let callback = Arc::clone(&ingress);
            let enrollment_ingress = Arc::new(Mutex::new(None::<enrollment::Ingress>));
            let enrollment_callback = Arc::clone(&enrollment_ingress);
            let callback_state = Arc::clone(&state);
            let scope_events = Arc::new(Mutex::new(None::<NativeServiceScope>));
            let callback_scope = scope_events.clone();
            let mut sync_config = destination_config(limits()).unwrap();
            let mut enrollment_config =
                enrollment::destination_config(enrollment_limits()).unwrap();
            let secret = [peer as u8 + 0x60; 64];
            if scoped {
                sync_config.identity = prns_host::DestinationIdentityConfig::Dedicated(
                    IdentityConfig::Existing(prns_host::IdentitySecret::new(secret)),
                );
                enrollment_config.identity = prns_host::DestinationIdentityConfig::Dedicated(
                    IdentityConfig::Existing(prns_host::IdentitySecret::new(secret)),
                );
            }
            let owner = OwnedSession::open_with_embedding(
                HostConfig {
                    identity: IdentityConfig::GenerateEphemeral,
                    persistence: PersistenceConfig::Ephemeral,
                    role: HostRole::Endpoint,
                    destinations: vec![
                        DestinationConfig::Single(sync_config),
                        DestinationConfig::Single(enrollment_config),
                    ],
                    required_capabilities: vec![],
                    limits: PrnsLimits::balanced(),
                },
                NativeEmbedding {
                    on_event: Some(Box::new(move |event| {
                        if let Some(scope) = callback_scope.lock().unwrap().as_ref() {
                            scope.on_event_ref(&event);
                        }
                        if let Some(ingress) = callback.lock().unwrap().as_ref() {
                            if ingress.on_event_ref(&event)
                                == prns_groups_reticulum::IngressOutcome::Queued
                            {
                                callback_state.jobs[peer].fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        if let Some(ingress) = enrollment_callback.lock().unwrap().as_ref() {
                            ingress.on_event_ref(&event);
                        }
                    })),
                    application_events: ApplicationEventDispatch::NativeCallback,
                    prepare_interfaces: Some(Box::new(move |client| {
                        Ok(vec![NativePreparedAttachment::Interface {
                            attachment: client.protocols().attach(interface),
                            kind: prns_host::InterfaceKind::Pipe,
                        }])
                    })),
                    ..NativeEmbedding::default()
                },
            )
            .await
            .unwrap();
            let destination = owner.client().destination_hashes().unwrap()[0];
            let identity = if scoped {
                prns_host::IdentityHash::new(
                    *personal_rns::identity::PrivateIdentityMaterial::from(
                        personal_rns::identity::vault::IdentitySecretKey::new(secret),
                    )
                    .identity_hash()
                    .as_bytes(),
                )
            } else {
                owner.client().identity_hash().unwrap()
            };
            let scope = NativeServiceScope::new(
                owner.client(),
                identity,
                owner.client().destination_hashes().unwrap(),
                16,
                8,
            )
            .unwrap();
            *scope_events.lock().unwrap() = Some(scope.clone());
            scopes.push(scope);
            let (callback, receiver) = Ingress::new(destination, limits()).unwrap();
            *ingress.lock().unwrap() = Some(callback);
            let (callback, receiver_enrollment) = enrollment::Ingress::new(
                enrollment::destination_for(identity).unwrap(),
                enrollment_limits(),
            )
            .unwrap();
            *enrollment_ingress.lock().unwrap() = Some(callback);
            enrollment_jobs.push(Some(receiver_enrollment));
            owners.push(owner);
            jobs.push(Some(receiver));
        }
        let forward_state = Arc::clone(&state);
        let forwarder = tokio::spawn(async move {
            while let Some((origin, frame)) = frames.recv().await {
                if !forward_state.online[origin].load(Ordering::SeqCst) {
                    continue;
                }
                forward_state.frames[origin].fetch_add(1, Ordering::SeqCst);
                if let Ok((header, _)) = WirePacketHeader::parse(&frame) {
                    if header.packet_type == PacketType::Announce {
                        if header.context == WireContext::PathResponse {
                            forward_state.path_responses.fetch_add(1, Ordering::SeqCst);
                        } else {
                            forward_state
                                .unsolicited_announces
                                .fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    if header.context == WireContext::ResourceAdvertisement {
                        forward_state.resources.fetch_add(1, Ordering::SeqCst);
                    }
                    if header.context == WireContext::ResourceProof
                        && header.packet_type == PacketType::Proof
                    {
                        forward_state.resource_proofs.fetch_add(1, Ordering::SeqCst);
                    }
                }
                for (recipient, channel) in inbound.iter().enumerate() {
                    if recipient != origin && forward_state.online[recipient].load(Ordering::SeqCst)
                    {
                        let _ = channel.send(frame.clone());
                    }
                }
            }
        });
        let peers: Vec<_> = owners
            .iter()
            .enumerate()
            .map(|(index, owner)| Peer {
                endpoint: super::endpoint(index),
                identity: scopes[index].identity_hash(),
                destination: owner.client().destination_hashes().unwrap()[0],
            })
            .collect();
        let mut networks = Vec::new();
        let mut enrollment_networks = Vec::new();
        for (index, owner) in owners.iter().enumerate() {
            // The fourth endpoint may know the members, but members do not trust it.
            let network = if scoped {
                Network::scoped(
                    scopes[index].clone(),
                    peers[index].destination,
                    peers[..3].to_vec(),
                    limits(),
                )
            } else {
                Network::new(
                    owner.client(),
                    peers[index].destination,
                    peers[..3].to_vec(),
                    limits(),
                )
            }
            .unwrap();
            network.authorize_peers().await.unwrap();
            networks.push(Arc::new(network));
            enrollment_networks.push(Arc::new(
                if scoped {
                    enrollment::Network::scoped(
                        scopes[index].clone(),
                        enrollment::destination_for(scopes[index].identity_hash()).unwrap(),
                        enrollment_limits(),
                    )
                } else {
                    enrollment::Network::new(
                        owner.client(),
                        enrollment::destination_for(scopes[index].identity_hash()).unwrap(),
                        enrollment_limits(),
                    )
                }
                .unwrap(),
            ));
        }
        Self {
            _host_slots: host_slots,
            owners,
            scopes,
            networks,
            jobs,
            enrollment_networks,
            enrollment_jobs,
            state,
            forwarder,
        }
    }
    pub async fn stop(self) {
        for owner in self.owners {
            owner.stop().await.unwrap();
        }
        self.forwarder.abort();
    }
}
