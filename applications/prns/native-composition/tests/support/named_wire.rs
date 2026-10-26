//! Controlled carrier using the same interface seam as the Reticulum transport
//! tests. Bytes still cross independent real hosts, authenticated Links and
//! Resources; this is not a Bluetooth/LoRa or power-loss fixture.
#![allow(dead_code)]

use personal_rns::{
    interfaces::{
        AnnounceBandwidthCap, BitrateBps, EgressCapability, IngressCapability,
        InterfaceCapabilities, InterfaceCommonPolicy, InterfaceDescriptor, InterfaceGravity,
        InterfaceId, InterfaceKind, InterfaceMode, ReportsStatus, TransportCapability,
    },
    manifold::interface_seam::{Interface, InterfaceSeam},
    runtime::{Attachable, AttachedInterface},
    wire::BROADCAST_MTU,
    PrnsNodeHandle,
};
use prns_app::identity_services::MessagingEvents;
use prns_host::{HostConfig, HostRole, IdentityConfig, PersistenceConfig, PrnsLimits};
use prns_host_native::{
    owner::OwnedSession, ApplicationEventDispatch, NativeEmbedding, NativePreparedAttachment,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::mpsc;

type Frame = (usize, Vec<u8>);
type ReceiverSlot = Option<mpsc::UnboundedSender<Vec<u8>>>;
pub struct Wire {
    online: Arc<[AtomicBool; 2]>,
    inbound: Arc<Mutex<[ReceiverSlot; 2]>>,
    outbound: mpsc::UnboundedSender<Frame>,
    pub delivered: Arc<AtomicUsize>,
    forwarder: tokio::task::JoinHandle<()>,
}
impl Wire {
    pub fn new() -> Self {
        let online = Arc::new(std::array::from_fn(|_| AtomicBool::new(true)));
        let inbound = Arc::new(Mutex::new([None::<mpsc::UnboundedSender<Vec<u8>>>, None]));
        let (outbound, mut frames) = mpsc::unbounded_channel::<Frame>();
        let delivered = Arc::new(AtomicUsize::new(0));
        let (enabled, inputs, count) = (online.clone(), inbound.clone(), delivered.clone());
        let forwarder = tokio::spawn(async move {
            while let Some((origin, frame)) = frames.recv().await {
                if !enabled[origin].load(Ordering::SeqCst) {
                    continue;
                }
                let recipient = 1 - origin;
                if enabled[recipient].load(Ordering::SeqCst) {
                    if let Some(input) = inputs
                        .lock()
                        .unwrap()
                        .get(recipient)
                        .and_then(Option::as_ref)
                    {
                        if input.send(frame).is_ok() {
                            count.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            }
        });
        Self {
            online,
            inbound,
            outbound,
            delivered,
            forwarder,
        }
    }
    pub fn set_online(&self, peer: usize, online: bool) {
        self.online[peer].store(online, Ordering::SeqCst);
    }
    pub async fn host(&self, peer: usize, events: &MessagingEvents) -> Arc<OwnedSession> {
        let (input, inbound) = mpsc::unbounded_channel();
        self.inbound.lock().unwrap()[peer] = Some(input);
        let interface = Carrier {
            peer,
            inbound,
            outbound: self.outbound.clone(),
        };
        let e = events.clone();
        let a = events.clone();
        OwnedSession::open_with_embedding(
            HostConfig {
                identity: IdentityConfig::GenerateEphemeral,
                persistence: PersistenceConfig::Ephemeral,
                role: HostRole::Endpoint,
                destinations: vec![],
                required_capabilities: vec![],
                limits: PrnsLimits::balanced(),
            },
            NativeEmbedding {
                on_event: Some(Box::new(move |event| e.on_event(&event))),
                accepted_announces: Some(Box::new(move |announce| a.on_announce(&announce))),
                application_events: ApplicationEventDispatch::NativeCallback,
                prepare_interfaces: Some(Box::new(move |client| {
                    Ok(vec![NativePreparedAttachment::Interface {
                        attachment: client.protocols().attach(interface),
                        kind: prns_host::InterfaceKind::Pipe,
                    }])
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }
}
impl Drop for Wire {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}
struct Carrier {
    peer: usize,
    inbound: mpsc::UnboundedReceiver<Vec<u8>>,
    outbound: mpsc::UnboundedSender<Frame>,
}
impl Carrier {
    fn descriptor(&self) -> InterfaceDescriptor {
        InterfaceDescriptor {
            id: InterfaceId::new([0xc0 + self.peer as u8; 8]),
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
}
impl Interface for Carrier {
    const HW_MTU: usize = BROADCAST_MTU;
    const KIND: InterfaceKind = InterfaceKind::Loopback;
    fn descriptor(&self) -> InterfaceDescriptor {
        self.descriptor()
    }
    fn channel_tag(&self) -> &[u8] {
        b"named-recovery"
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
impl ReportsStatus for Carrier {}
impl Attachable for Carrier {
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
