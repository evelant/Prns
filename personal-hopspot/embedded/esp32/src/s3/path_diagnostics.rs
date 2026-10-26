//! Temporary qualification trace. No packets, secrets, raw endpoint IDs, or heap
//! storage. A reboot creates a new redaction key and resets the hard output cap.
use core::cell::RefCell;
use core::fmt;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use personal_rns::runtime::{Diagnostic, PrnsEvent};
use prns_core::crypto::{hmac_sha256_chunks, sha256_chunks};
use prns_core::engine::{PathDiscoveryEvent, PathDiscoveryPhase};

const LIMIT: u16 = 2048;
struct State {
    salt: Option<[u8; 32]>,
    count: u16,
}
static STATE: Mutex<CriticalSectionRawMutex, RefCell<State>> = Mutex::new(RefCell::new(State {
    salt: None,
    count: 0,
}));

pub(super) fn observe(event: &PrnsEvent<'_>) {
    let PrnsEvent::Diagnostic(Diagnostic::PathDiscovery(event)) = event else {
        return;
    };
    let permit = STATE.lock(|state| {
        let mut state = state.borrow_mut();
        if state.count > LIMIT {
            return None;
        }
        state.count += 1;
        let sequence = state.count;
        let salt = *state.salt.get_or_insert_with(|| {
            let mut salt = [0; 32];
            super::entropy::runtime_entropy().fill_random(&mut salt);
            salt
        });
        Some((sequence, salt))
    });
    let Some((sequence, salt)) = permit else {
        return;
    };
    if sequence > LIMIT {
        log::info!("prns.path.discovery limit_reached={LIMIT}");
        return;
    }
    let destination = event
        .destination
        .map_or(0, |value| token(&salt, b"destination", value.as_bytes()));
    let interface = event
        .interface
        .map_or(0, |value| token(&salt, b"interface", value.as_bytes()));
    // The wire nonce is fresh and public. A domain-separated hash deliberately
    // correlates this one request across devices; it never exposes a stable ID.
    let request = event.request_tag.map_or(0, |tag| {
        prefix(sha256_chunks(&[b"prns.path.request.v1", &tag]))
    });
    let base = TraceLine {
        sequence,
        destination,
        interface,
        request,
        event,
    };
    match event.phase {
        PathDiscoveryPhase::RequestIngress {
            disposition,
            mode,
            recursive,
            cached_interface,
        } => {
            let cached =
                cached_interface.map_or(0, |value| token(&salt, b"interface", value.as_bytes()));
            base.emit(format_args!("disposition={disposition:?} mode={mode:?} recursive={recursive} cached_interface={cached:016x}"));
        }
        PathDiscoveryPhase::RequestEgress(outcome) => {
            base.emit(format_args!("disposition={outcome:?}"))
        }
        PathDiscoveryPhase::ResponseIngress(outcome) => {
            base.emit(format_args!("disposition={outcome:?}"))
        }
        PathDiscoveryPhase::LocalRequest { command }
        | PathDiscoveryPhase::LocalTimeout { command } => {
            base.emit(format_args!("command={}", command.0))
        }
        PathDiscoveryPhase::ResponseDispatched | PathDiscoveryPhase::RecursiveTimeout => {
            base.emit(format_args!(""))
        }
    }
}

struct TraceLine<'a> {
    sequence: u16,
    destination: u64,
    interface: u64,
    request: u64,
    event: &'a PathDiscoveryEvent,
}

impl TraceLine<'_> {
    fn emit(&self, extra: fmt::Arguments<'_>) {
        log::info!("prns.path.discovery seq={} observed_ms={} core_ms={:?} phase={} destination={:016x} interface={:016x} request={:016x} {}",
            self.sequence, embassy_time::Instant::now().as_millis(), self.event.at.map(|at| at.0),
            self.event.phase.name(), self.destination, self.interface, self.request, extra);
    }
}

fn prefix(bytes: [u8; 32]) -> u64 {
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
fn token(salt: &[u8; 32], domain: &[u8], value: &[u8]) -> u64 {
    prefix(hmac_sha256_chunks(salt, &[domain, value]))
}
