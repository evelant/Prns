//! Independent bounded sink for opt-in Link diagnostics. Public, ephemeral Link
//! tokens intentionally correlate one handshake across hops. Interface tags are
//! process-salted. Nothing here proves RF transmission or durable group delivery.
use super::platform;
use personal_rns::runtime::{EgressHandshakeEvent, EgressHandshakePhase};
use prns_core::engine::{LinkHandshakeEvent, LinkHandshakePhase};
use prns_host_native::NativeLinkDiagnostics;
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{sync_channel, SyncSender},
    Mutex, OnceLock,
};

const LIMIT: u64 = 8192;
const QUEUE_DEPTH: usize = 64;
static ENABLED: OnceLock<bool> = OnceLock::new();
static SINK: OnceLock<Option<Sink>> = OnceLock::new();
static HEALTHY: AtomicBool = AtomicBool::new(true);
static READY: AtomicBool = AtomicBool::new(false);
static CONFIGURED: AtomicBool = AtomicBool::new(false);
static PUBLISHED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Event {
    Core(LinkHandshakeEvent),
    Egress(EgressHandshakeEvent),
}
#[derive(Clone, Copy)]
struct Queued {
    sequence: u64,
    event: Event,
}
#[derive(Default)]
struct State {
    sequence: u64,
    core: u64,
    egress: u64,
}
struct Sink {
    sender: SyncSender<Queued>,
    state: Mutex<State>,
}

impl State {
    fn source(&mut self, event: Event) -> bool {
        let (previous, sequence, lost) = match event {
            Event::Core(e) => (
                &mut self.core,
                e.sequence,
                e.dropped_before != 0
                    || matches!(e.phase, LinkHandshakePhase::ObservationLimitReached { .. }),
            ),
            Event::Egress(e) => (
                &mut self.egress,
                e.sequence,
                e.snapshot.producer_dropped != 0
                    || e.snapshot.sink_dropped != 0
                    || e.phase == EgressHandshakePhase::LimitReached,
            ),
        };
        let continuous = sequence == previous.saturating_add(1) && !lost;
        *previous = sequence;
        continuous
    }
    fn enqueue(&mut self, sender: &SyncSender<Queued>, event: Event) -> bool {
        if self.sequence >= LIMIT {
            return false;
        }
        let sequence = self.sequence + 1;
        if sender.try_send(Queued { sequence, event }).is_err() {
            return false;
        }
        self.sequence = sequence;
        true
    }
}

fn enabled() -> bool {
    *ENABLED.get_or_init(|| platform::opted_in("PRNS_LINK_DIAGNOSTICS", c"debug.prns.links"))
}

pub(crate) fn observers() -> Option<NativeLinkDiagnostics> {
    if !enabled() {
        return None;
    }
    if CONFIGURED.swap(true, Ordering::AcqRel) {
        HEALTHY.store(false, Ordering::Release);
    }
    let sink = SINK.get_or_init(|| {
        let mut salt = [0; 32];
        if getrandom::getrandom(&mut salt).is_err() {
            return None;
        }
        let (sender, receiver) = sync_channel::<Queued>(QUEUE_DEPTH);
        // Only this native, explicitly enabled writer may wait on platform I/O.
        // Core/runtime callbacks use try_lock + try_send and never format/write.
        std::thread::Builder::new()
            .name("prns-link-trace".into())
            .spawn(move || {
                let _ = writeln!(
                    platform::output(),
                    "[prns.links] stage=probe category=installed"
                );
                READY.store(true, Ordering::Release);
                while let Ok(queued) = receiver.recv() {
                    let mut output = platform::output();
                    let result = match queued.event {
                        Event::Core(event) => {
                            write_core(&mut output, queued.sequence, &salt, &event)
                        }
                        Event::Egress(event) => {
                            write_egress(&mut output, queued.sequence, &salt, &event)
                        }
                    };
                    drop(output);
                    if result.is_err() {
                        HEALTHY.store(false, Ordering::Release);
                    }
                    PUBLISHED.store(queued.sequence, Ordering::Release);
                }
            })
            .ok()?;
        Some(Sink {
            sender,
            state: Mutex::new(State::default()),
        })
    });
    if sink.is_none() {
        HEALTHY.store(false, Ordering::Release);
        return None;
    }
    Some(NativeLinkDiagnostics {
        core: on_core,
        egress: on_egress,
    })
}

/// A pending writer queue is not complete evidence. Heartbeats expose only the
/// published watermark, and stay unready until every admitted event is written.
/// A cap, missed source event, sink rejection or producer reset fails closed.
pub(super) fn snapshot() -> (u64, bool) {
    let Some(Some(sink)) = SINK.get() else {
        return (0, false);
    };
    let Ok(state) = sink.state.try_lock() else {
        return (PUBLISHED.load(Ordering::Acquire), false);
    };
    let published = PUBLISHED.load(Ordering::Acquire);
    (
        published,
        state.sequence == published
            && READY.load(Ordering::Acquire)
            && HEALTHY.load(Ordering::Acquire),
    )
}

fn link_tag(bytes: &[u8]) -> u64 {
    let digest = prns_core::crypto::sha256_chunks(&[b"prns.link.handshake.v1", bytes]);
    let mut short = [0; 8];
    short.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(short)
}
fn interface_tag(salt: &[u8; 32], bytes: &[u8]) -> u64 {
    let digest = prns_core::crypto::hmac_sha256_chunks(salt, &[b"interface", bytes]);
    let mut short = [0; 8];
    short.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(short)
}
fn emit(event: Event) -> bool {
    let Some(Some(sink)) = SINK.get() else {
        return false;
    };
    let Ok(mut state) = sink.state.try_lock() else {
        HEALTHY.store(false, Ordering::Release);
        return false;
    };
    if !state.source(event) {
        HEALTHY.store(false, Ordering::Release);
    }
    if !state.enqueue(&sink.sender, event) {
        HEALTHY.store(false, Ordering::Release);
        return false;
    }
    true
}
fn on_core(event: LinkHandshakeEvent) -> bool {
    emit(Event::Core(event))
}
fn on_egress(event: EgressHandshakeEvent) -> bool {
    emit(Event::Egress(event))
}

fn write_core(
    out: &mut impl Write,
    sequence: u64,
    salt: &[u8; 32],
    event: &LinkHandshakeEvent,
) -> std::io::Result<()> {
    write!(out, "[prns.links] stage=handshake family=core sequence={sequence} source_sequence={} dropped_before={} link={:016x} has_link={} interface={:016x} has_interface={} command={} has_command={} at_ms={} has_at={} ",
        event.sequence, event.dropped_before, event.link_id.map_or(0, |v|link_tag(v.as_bytes())), event.link_id.is_some(),
        event.interface.map_or(0, |v|interface_tag(salt, v.as_bytes())), event.interface.is_some(), event.command.map_or(0, |v|v.0), event.command.is_some(), event.at.map_or(0, |v|v.0), event.at.is_some())?;
    match event.phase {
        LinkHandshakePhase::RequestPending { requested_at, timeout_at, hops, bitrate, first_hop_floor_ms, computed_timeout_ms, route_generation } => writeln!(out, "phase=request_pending requested_at_ms={} timeout_at_ms={} hops={hops} bitrate={} has_bitrate={} first_hop_floor_ms={} has_floor={} computed_timeout_ms={computed_timeout_ms} route_generation={route_generation}", requested_at.0, timeout_at.0, bitrate.map_or(0, |v|v.get()), bitrate.is_some(), first_hop_floor_ms.unwrap_or(0), first_hop_floor_ms.is_some()),
        LinkHandshakePhase::RequestRejected(reason) => writeln!(out, "phase=request_rejected detail={reason:?}"),
        LinkHandshakePhase::RequestIngress(reason) => writeln!(out, "phase=request_ingress detail={reason:?}"),
        LinkHandshakePhase::ProofIngress(reason) => writeln!(out, "phase=proof_ingress detail={reason:?}"),
        LinkHandshakePhase::ProofPrepared => writeln!(out, "phase=proof_prepared"),
        LinkHandshakePhase::ProofRejected(reason) => writeln!(out, "phase=proof_rejected detail={reason:?}"),
        LinkHandshakePhase::ProofCompletion(reason) => writeln!(out, "phase=proof_completion detail={reason:?}"),
        LinkHandshakePhase::InitiatorTimeout { requested_at, route_generation, route } => writeln!(out, "phase=initiator_timeout requested_at_ms={} route_generation={route_generation} detail={route:?}", requested_at.0),
        LinkHandshakePhase::RelayPending { timeout_at, remaining_hops, taken_hops, bitrate, route_generation } => writeln!(out, "phase=relay_pending timeout_at_ms={} remaining_hops={remaining_hops} taken_hops={taken_hops} bitrate={} has_bitrate={} route_generation={route_generation}", timeout_at.0, bitrate.map_or(0, |v|v.get()), bitrate.is_some()),
        LinkHandshakePhase::RelayTimeout { timeout_at, route_generation } => writeln!(out, "phase=relay_timeout timeout_at_ms={} route_generation={route_generation}", timeout_at.0),
        LinkHandshakePhase::ResponderTimeout => writeln!(out, "phase=responder_timeout"),
        LinkHandshakePhase::ObservationLimitReached { limit, dropped_at_least } => writeln!(out, "phase=producer_limit limit={limit} dropped_at_least={dropped_at_least}"),
    }
}
fn write_egress(
    out: &mut impl Write,
    sequence: u64,
    salt: &[u8; 32],
    event: &EgressHandshakeEvent,
) -> std::io::Result<()> {
    let packet = event.packet.map_or("none", |v| match v {
        personal_rns::runtime::EgressHandshakePacket::Request => "request",
        personal_rns::runtime::EgressHandshakePacket::Proof => "proof",
    });
    writeln!(out, "[prns.links] stage=handshake family=egress sequence={sequence} source_sequence={} link={:016x} has_link={} interface={:016x} has_interface={} packet={packet} phase={:?} queue_age_ms={} has_age={} attempted={} emitted={} producer_dropped={} sink_dropped={}",
        event.sequence, event.link_id.map_or(0, |v|link_tag(v.as_bytes())), event.link_id.is_some(), event.interface.map_or(0, |v|interface_tag(salt, v.as_bytes())), event.interface.is_some(), event.phase, event.queue_age_ms.unwrap_or(0), event.queue_age_ms.is_some(), event.snapshot.attempted, event.snapshot.emitted, event.snapshot.producer_dropped, event.snapshot.sink_dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prns_core::{
        engine::{CommandId, InstantMillis},
        interfaces::InterfaceId,
        routing::links::LinkId,
    };
    fn example_event(sequence: u64) -> Event {
        Event::Core(LinkHandshakeEvent {
            sequence,
            dropped_before: 0,
            at: None,
            link_id: None,
            command: None,
            interface: None,
            phase: LinkHandshakePhase::ResponderTimeout,
        })
    }
    #[test]
    fn blocked_writer_is_bounded_and_does_not_block_admission() {
        let (sender, receiver) = sync_channel(2);
        let mut state = State::default();
        assert!(state.enqueue(&sender, example_event(1)));
        assert!(state.enqueue(&sender, example_event(2)));
        assert!(!state.enqueue(&sender, example_event(3)));
        assert_eq!(state.sequence, 2);
        assert_eq!(receiver.try_recv().unwrap().sequence, 1);
        assert!(state.enqueue(&sender, example_event(4)));
        assert_eq!(receiver.try_recv().unwrap().sequence, 2);
        assert_eq!(receiver.try_recv().unwrap().sequence, 3);
        drop(receiver);
        assert!(!state.enqueue(&sender, example_event(5)));
        assert_eq!(state.sequence, 3);
    }
    #[test]
    fn source_gap_restart_and_sink_limit_are_detected() {
        let mut state = State::default();
        assert!(state.source(example_event(1)));
        assert!(!state.source(example_event(3)));
        assert!(!state.source(example_event(1)));
        let (sender, receiver) = sync_channel(1);
        state.sequence = LIMIT;
        assert!(!state.enqueue(&sender, example_event(2)));
        assert!(receiver.try_recv().is_err());
        assert_eq!(state.sequence, LIMIT);
    }
    #[test]
    fn core_format_redacts_identifiers_and_preserves_deadline_inputs() {
        let event = LinkHandshakeEvent {
            sequence: 1,
            dropped_before: 0,
            at: None,
            link_id: Some(LinkId::new([0x11; 16])),
            command: Some(CommandId(7)),
            interface: Some(InterfaceId::new([0x22; 8])),
            phase: LinkHandshakePhase::RequestPending {
                requested_at: InstantMillis(100),
                timeout_at: InstantMillis(24100),
                hops: 3,
                bitrate: None,
                first_hop_floor_ms: Some(900),
                computed_timeout_ms: 24000,
                route_generation: 6,
            },
        };
        let mut output = Vec::new();
        write_core(&mut output, 1, &[1; 32], &event).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("timeout_at_ms=24100 hops=3"));
        assert!(text.contains("has_at=false"));
        for raw in ["1111111111111111", "2222222222222222"] {
            assert!(!text.contains(raw));
        }
        assert_ne!(
            interface_tag(&[1; 32], &[2; 8]),
            interface_tag(&[2; 32], &[2; 8])
        );
        assert_eq!(link_tag(&[1; 16]), link_tag(&[1; 16]));
    }
    #[test]
    fn egress_format_preserves_loss_and_queue_age() {
        let event = EgressHandshakeEvent {
            sequence: 4,
            link_id: None,
            interface: None,
            packet: None,
            phase: EgressHandshakePhase::PendingDiscarded,
            queue_age_ms: Some(300),
            snapshot: personal_rns::runtime::EgressHandshakeSnapshot {
                attempted: 4,
                emitted: 2,
                producer_dropped: 0,
                sink_dropped: 1,
                limit: 2048,
            },
        };
        let mut output = Vec::new();
        write_egress(&mut output, 5, &[1; 32], &event).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("phase=PendingDiscarded queue_age_ms=300 has_age=true"));
        assert!(text.contains("sink_dropped=1"));
    }
}
