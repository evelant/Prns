//! Native-only extension points. These reuse the existing public Rust protocol APIs;
//! they do not expose an owning node or introduce foreign SDK-specific semantics.

use personal_rns::manifold::tokio::TokioClock;
use personal_rns::remote_control::RemoteControlService;
use personal_rns::runtime::PrnsEvent;
use personal_rns::{AttachedInterface, AttachedSupervisor, PlanRuntimeContext, PrnsNodeHandle};
use prns_host::InterfaceKind;

use crate::NativeStartError;

pub type NativeEventCallback = Box<dyn for<'a> FnMut(PrnsEvent<'a>) + Send>;
/// A successfully authenticated announce projected by the shared native host.
/// The borrowed payload is valid only for the callback; services copy their own
/// bounded facts before returning and validate protocol-specific name association.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedAnnounce<'a> {
    pub destination: prns_host::DestinationHash,
    pub announced_identity: prns_host::IdentityHash,
    /// Interface and hop count recorded when this announce was accepted.
    pub source_interface: prns_host::InterfaceId,
    pub hops: u8,
    pub app_data: &'a [u8],
    pub arrived_at_millis: u64,
    pub is_path_response: bool,
}

pub type NativeAnnounceObserver = Box<dyn for<'a> FnMut(AuthenticatedAnnounce<'a>) + Send>;
pub type NativeInterfacePreparation = Box<
    dyn FnOnce(&NativeServiceClient) -> Result<Vec<NativePreparedAttachment>, NativeStartError>
        + Send,
>;

/// Reusable platform construction for every automatic LAN attachment, including
/// re-enable. The host still owns the supervisor and its acknowledged retirement.
pub type NativeAutoLanFactory = std::sync::Arc<
    dyn Fn(
            &PrnsNodeHandle,
            &prns_host::InterfaceConfig,
            Option<prns_host::InterfaceRoutingPolicy>,
        ) -> Result<NativePreparedAttachment, prns_host::CommandFailure>
        + Send
        + Sync,
>;

/// Repeatable cleanup retains platform ownership after an uncertain shutdown.
pub type NativeAttachmentCleanup = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationEventDispatch {
    /// The canonical bounded queue owns application messages.
    #[default]
    Queue,
    /// A native service dispatcher owns application messages. The session owner must
    /// retain its application lane claim, and the callback must enforce its own bounds.
    NativeCallback,
}

/// Native-only, opt-in diagnostic callbacks. No payloads or key material are
/// supplied; exporters must redact interface identifiers and bound their sink.
#[derive(Clone, Copy)]
pub struct NativeLinkDiagnostics {
    pub core: personal_rns::engine::LinkHandshakeObserver,
    pub egress: personal_rns::runtime::EgressHandshakeObserver,
}

pub struct NativeEmbedding {
    pub remote_control: RemoteControlService<'static>,
    pub remote_control_config: Option<crate::remote_control_service::NativeRemoteControlConfig>,
    pub plan_context: Option<PlanRuntimeContext>,
    pub on_event: Option<NativeEventCallback>,
    pub accepted_announces: Option<NativeAnnounceObserver>,
    pub link_diagnostics: Option<NativeLinkDiagnostics>,
    pub prepare_interfaces: Option<NativeInterfacePreparation>,
    pub auto_lan_factory: Option<NativeAutoLanFactory>,
    pub application_events: ApplicationEventDispatch,
}

impl Default for NativeEmbedding {
    fn default() -> Self {
        Self {
            remote_control: RemoteControlService::Unavailable,
            remote_control_config: None,
            plan_context: None,
            on_event: None,
            accepted_announces: None,
            link_diagnostics: None,
            prepare_interfaces: None,
            auto_lan_factory: None,
            application_events: ApplicationEventDispatch::Queue,
        }
    }
}

/// A borrowed native protocol client. The underlying public command handle has no
/// owning node, run loop, stop authority, or ability to replace host persistence.
/// Native services may reuse its Rust protocol traits without parallel wrappers.
/// Attach prepared interfaces through `NativeEmbedding` so inspection and teardown
/// remain tracked by the host; ordinary foreign consumers use canonical commands.
#[derive(Clone)]
pub struct NativeServiceClient {
    pub(crate) handle: PrnsNodeHandle,
    pub(crate) clock: TokioClock,
}

impl NativeServiceClient {
    pub fn protocols(&self) -> &PrnsNodeHandle {
        &self.handle
    }
    pub fn clock(&self) -> TokioClock {
        self.clock.clone()
    }
}

/// Prepared platform mechanics still belong to the host's canonical attachment map.
/// A native composition must return every attached interface/supervisor here.
pub enum NativePreparedAttachment {
    /// Platform attachment types that expose a registered interface id but keep their
    /// platform status/manager outside `AttachedInterface` (for example prepared BLE).
    Registered {
        interface: personal_rns::interfaces::InterfaceId,
        kind: InterfaceKind,
    },
    Interface {
        attachment: AttachedInterface,
        kind: InterfaceKind,
    },
    Supervisor {
        attachment: AttachedSupervisor,
        kind: InterfaceKind,
    },
    ManagedSupervisor {
        attachment: AttachedSupervisor,
        kind: InterfaceKind,
        cleanup: NativeAttachmentCleanup,
    },
}
