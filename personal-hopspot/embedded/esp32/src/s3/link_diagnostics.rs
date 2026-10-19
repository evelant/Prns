//! E290 qualification-only sink. Fixed queue in PSRAM, no formatting or serial
//! I/O in producer callbacks. CRC and sequence gaps expose serial interleaving.
//! Source counters are lower bounds after caps. Output is not proof of RF delivery.
use core::{cell::RefCell, fmt::Write};
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex, Mutex},
    channel::Channel,
    once_lock::OnceLock,
};
use embassy_time::{Duration, Instant, Timer};
use personal_rns::lora::{LoRaLinkEvent, LoRaLinkPhase, LoRaLinkTrace};
use prns_core::engine::{LinkHandshakeEvent, LinkHandshakePhase};

const QUEUE_DEPTH: usize = 32;
const HEALTH_LIMIT: u32 = 1800;
type TraceQueue = Channel<CriticalSectionRawMutex, Queued, QUEUE_DEPTH>;
static QUEUE: OnceLock<&'static TraceQueue> = OnceLock::new();
struct ClockAnchor {
    core_ms: u64,
    radio_before_ms: u64,
    radio_after_ms: u64,
}
static CLOCK: OnceLock<ClockAnchor> = OnceLock::new();
pub(super) static LORA: LoRaLinkTrace = LoRaLinkTrace::new(on_lora);
#[derive(Clone, Copy)]
enum Event {
    Core(LinkHandshakeEvent),
    LoRa(LoRaLinkEvent),
}
#[derive(Clone, Copy)]
struct Queued {
    event: Event,
}
#[derive(Clone, Copy)]
struct State {
    core_seen: u64,
    lora_seen: u32,
    core_admitted: u32,
    lora_admitted: u32,
    core_published: u32,
    lora_published: u32,
    sink_dropped: u32,
    core_capped: bool,
    lora_capped: bool,
    healthy: bool,
}
static STATE: Mutex<CriticalSectionRawMutex, RefCell<State>> = Mutex::new(RefCell::new(State {
    core_seen: 0,
    lora_seen: 0,
    core_admitted: 0,
    lora_admitted: 0,
    core_published: 0,
    lora_published: 0,
    sink_dropped: 0,
    core_capped: false,
    lora_capped: false,
    healthy: true,
}));

// Separate preparation bounds the caller's frame. Inspect its generated prologue
// before flashing; no large trace buffer is placed in the internal task pool.
#[inline(never)]
pub(super) fn prepare() -> core::pin::Pin<&'static mut dyn core::future::Future<Output = ()>> {
    let queue = crate::storage::allocate_psram_uninit::<TraceQueue>().write(Channel::new());
    assert!(QUEUE.init(queue).is_ok(), "Link trace installed once");
    let mut salt = [0; 32];
    super::entropy::runtime_entropy().fill_random(&mut salt);
    let boot = prefix(prns_core::crypto::hmac_sha256_chunks(
        &salt,
        &[b"board-link-boot"],
    ));
    let run = crate::storage::allocate_psram(writer(queue, salt, boot));
    // SAFETY: the board allocator leaks this allocation; it cannot move or be freed.
    unsafe { core::pin::Pin::new_unchecked(run) }
}

/// Called once after persistence has resumed the actual runtime host clock and
/// before its manifold starts. The writer never samples a stale copied origin.
pub(super) fn anchor_restored_clock(now: impl FnOnce() -> prns_core::engine::InstantMillis) {
    let radio_before_ms = Instant::now().as_millis();
    let core_ms = now().0;
    let radio_after_ms = Instant::now().as_millis();
    assert!(
        CLOCK
            .init(ClockAnchor {
                core_ms,
                radio_before_ms,
                radio_after_ms
            })
            .is_ok(),
        "Link trace clock installed once"
    );
}

pub(super) fn on_core(event: LinkHandshakeEvent) -> bool {
    enqueue(Event::Core(event))
}
fn on_lora(event: LoRaLinkEvent) -> bool {
    enqueue(Event::LoRa(event))
}
fn enqueue(event: Event) -> bool {
    let Some(queue) = QUEUE.try_get() else {
        return false;
    };
    STATE.lock(|state| {
        let mut s = state.borrow_mut();
        match event {
            Event::Core(e) => {
                let continuous =
                    e.sequence == s.core_seen.saturating_add(1) && e.dropped_before == 0;
                s.healthy &= continuous;
                s.core_seen = e.sequence;
                s.core_capped |=
                    matches!(e.phase, LinkHandshakePhase::ObservationLimitReached { .. });
            }
            Event::LoRa(e) => {
                let continuous = e.sequence == s.lora_seen.saturating_add(1);
                s.healthy &= continuous;
                s.lora_seen = e.sequence;
                s.lora_capped |= e.phase == LoRaLinkPhase::LimitReached;
            }
        }
        let uncapped = !s.core_capped && !s.lora_capped;
        s.healthy &= uncapped;
        if queue.try_send(Queued { event }).is_err() {
            s.sink_dropped = s.sink_dropped.saturating_add(1);
            s.healthy = false;
            return false;
        }
        match event {
            Event::Core(_) => s.core_admitted = s.core_admitted.saturating_add(1),
            Event::LoRa(_) => s.lora_admitted = s.lora_admitted.saturating_add(1),
        }
        true
    })
}

async fn writer(queue: &'static TraceQueue, salt: [u8; 32], boot: u64) {
    use embassy_futures::select::{select, Either};
    let mut sequence = 0u32;
    // Core time is RTC-aligned; radio time is Embassy monotonic. Bracket their
    // mapping explicitly so a cross-domain subtraction cannot invent queue delay.
    let ClockAnchor {
        core_ms,
        radio_before_ms,
        radio_after_ms,
    } = *CLOCK.get().await;
    publish(&mut sequence, boot, Instant::now().as_millis(), |line| {
        write!(line, "stage=probe category=installed core_ms={core_ms} radio_before_ms={radio_before_ms} radio_after_ms={radio_after_ms}")
    });
    let mut health_count = 0;
    let mut next_health = Instant::now() + Duration::from_secs(2);
    loop {
        match select(queue.receive(), Timer::at(next_health)).await {
            Either::First(queued) => {
                let ok =
                    publish(
                        &mut sequence,
                        boot,
                        Instant::now().as_millis(),
                        |line| match queued.event {
                            Event::Core(e) => write_core(line, &salt, &e),
                            Event::LoRa(e) => write_lora(line, &e),
                        },
                    );
                STATE.lock(|state| {
                    let mut s = state.borrow_mut();
                    s.healthy &= ok;
                    if ok {
                        match queued.event {
                            Event::Core(_) => s.core_published = s.core_published.saturating_add(1),
                            Event::LoRa(_) => s.lora_published = s.lora_published.saturating_add(1),
                        }
                    }
                });
            }
            Either::Second(()) => {
                health_count += 1;
                let s = STATE.lock(|s| *s.borrow());
                let healthy = s.healthy
                    && s.core_admitted == s.core_published
                    && s.lora_admitted == s.lora_published;
                let ok = publish(&mut sequence, boot, Instant::now().as_millis(), |line| {
                    write!(line,
                    "stage=health core_seen={} lora_seen={} core_admitted={} lora_admitted={} core_published={} lora_published={} sink_dropped={} core_capped={} lora_capped={} healthy={}",
                    s.core_seen,s.lora_seen,s.core_admitted,s.lora_admitted,s.core_published,s.lora_published,s.sink_dropped,s.core_capped,s.lora_capped,healthy)
                });
                if !ok {
                    STATE.lock(|s| s.borrow_mut().healthy = false);
                }
                if health_count == HEALTH_LIMIT {
                    // No further callback may be claimed covered after the health budget.
                    STATE.lock(|s| s.borrow_mut().healthy = false);
                    publish(&mut sequence, boot, Instant::now().as_millis(), |line| {
                        write!(line, "stage=limit category=health limit={HEALTH_LIMIT}")
                    });
                    break;
                }
                next_health = Instant::now() + Duration::from_secs(2);
            }
        }
    }
}

fn publish(
    sequence: &mut u32,
    boot: u64,
    at_ms: u64,
    format: impl FnOnce(&mut heapless::String<750>) -> core::fmt::Result,
) -> bool {
    *sequence = sequence.saturating_add(1);
    let mut line = heapless::String::<750>::new();
    let result = (|| {
        write!(
            line,
            "v=1 boot={boot:016x} seq={sequence} observed_ms={at_ms} "
        )?;
        format(&mut line)?;
        let checksum = crc32(line.as_bytes());
        write!(line, " checksum={checksum:08x}")
    })();
    if result.is_err() {
        return false;
    }
    // This feature enables the printer's whole-record cross-core lock. ROM/panic
    // output and host loss still require CRC/sequence checks; never repair lines.
    esp_println::println!("prns.link.trace {}", line);
    true
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32.wrapping_mul(crc & 1));
        }
    }
    !crc
}
fn prefix(bytes: [u8; 32]) -> u64 {
    u64::from_be_bytes(bytes[..8].try_into().expect("eight-byte prefix"))
}
fn link_tag(bytes: &[u8]) -> u64 {
    prefix(prns_core::crypto::sha256_chunks(&[
        b"prns.link.handshake.v1",
        bytes,
    ]))
}
fn interface_tag(salt: &[u8; 32], bytes: &[u8]) -> u64 {
    prefix(prns_core::crypto::hmac_sha256_chunks(
        salt,
        &[b"interface", bytes],
    ))
}
fn write_core(
    out: &mut impl Write,
    salt: &[u8; 32],
    event: &LinkHandshakeEvent,
) -> core::fmt::Result {
    write!(out, "stage=handshake family=core source_sequence={} dropped_before={} link={:016x} has_link={} interface={:016x} has_interface={} command={} has_command={} at_ms={} has_at={} ",
        event.sequence, event.dropped_before, event.link_id.map_or(0, |v|link_tag(v.as_bytes())), event.link_id.is_some(),
        event.interface.map_or(0, |v|interface_tag(salt, v.as_bytes())), event.interface.is_some(), event.command.map_or(0, |v|v.0), event.command.is_some(), event.at.map_or(0, |v|v.0), event.at.is_some())?;
    match event.phase {
        LinkHandshakePhase::RequestPending { requested_at, timeout_at, hops, bitrate, first_hop_floor_ms, computed_timeout_ms, route_generation } => write!(out, "phase=request_pending requested_at_ms={} timeout_at_ms={} hops={hops} bitrate={} has_bitrate={} first_hop_floor_ms={} has_floor={} computed_timeout_ms={computed_timeout_ms} route_generation={route_generation}", requested_at.0, timeout_at.0, bitrate.map_or(0, |v|v.get()), bitrate.is_some(), first_hop_floor_ms.unwrap_or(0), first_hop_floor_ms.is_some()),
        LinkHandshakePhase::RequestRejected(reason) => write!(out, "phase=request_rejected detail={reason:?}"),
        LinkHandshakePhase::RequestIngress(reason) => write!(out, "phase=request_ingress detail={reason:?}"),
        LinkHandshakePhase::ProofIngress(reason) => write!(out, "phase=proof_ingress detail={reason:?}"),
        LinkHandshakePhase::ProofPrepared => write!(out, "phase=proof_prepared"),
        LinkHandshakePhase::ProofRejected(reason) => write!(out, "phase=proof_rejected detail={reason:?}"),
        LinkHandshakePhase::ProofCompletion(reason) => write!(out, "phase=proof_completion detail={reason:?}"),
        LinkHandshakePhase::InitiatorTimeout { requested_at, route_generation, route } => write!(out, "phase=initiator_timeout requested_at_ms={} route_generation={route_generation} detail={route:?}", requested_at.0),
        LinkHandshakePhase::RelayPending { timeout_at, remaining_hops, taken_hops, bitrate, route_generation } => write!(out, "phase=relay_pending timeout_at_ms={} remaining_hops={remaining_hops} taken_hops={taken_hops} bitrate={} has_bitrate={} route_generation={route_generation}", timeout_at.0, bitrate.map_or(0, |v|v.get()), bitrate.is_some()),
        LinkHandshakePhase::RelayTimeout { timeout_at, route_generation } => write!(out, "phase=relay_timeout timeout_at_ms={} route_generation={route_generation}", timeout_at.0),
        LinkHandshakePhase::ResponderTimeout => write!(out, "phase=responder_timeout"),
        LinkHandshakePhase::ObservationLimitReached { limit, dropped_at_least } => write!(out, "phase=producer_limit limit={limit} dropped_at_least={dropped_at_least}"),
    }
}
fn write_lora(out: &mut impl Write, e: &LoRaLinkEvent) -> core::fmt::Result {
    // Snapshot at publication time, individually atomic; not an event-time transaction.
    let s = LORA.snapshot();
    write!(out,"stage=handshake family=lora source_sequence={} link={:016x} has_link={} packet={:?} phase={:?} at_ms={} has_at={} age_ms={} has_age={} queue_bytes={} active={} frame_index={} frame_count={} attempted={} emitted={} producer_dropped={} sink_dropped={} queue_accepted={} tx_started={} tx_complete={} tx_failed={} rx_frames={} rx_packets={} rx_crc_errors={} rx_header_errors={} rx_radio_errors={}",
        e.sequence,e.link_id.map_or(0,|v|link_tag(v.as_bytes())),e.link_id.is_some(),e.packet,e.phase,e.at_ms.unwrap_or(0),e.at_ms.is_some(),e.age_ms.unwrap_or(0),e.age_ms.is_some(),e.queue_bytes,e.active,e.frame_index,e.frame_count,
        s.attempted,s.emitted,s.producer_dropped,s.sink_dropped,s.queue_accepted,s.tx_started,s.tx_complete,s.tx_failed,s.rx_frames,s.rx_packets,s.rx_crc_errors,s.rx_header_errors,s.rx_radio_errors)
}
