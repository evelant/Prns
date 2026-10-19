//! Bounded route-phase observations. Raw destinations, wire nonces and interface
//! identifiers never leave this callback. Endpoint tags are process-local; the
//! already random public request nonce is hashed consistently across relays.
use super::{platform, take_budget};
use personal_rns::runtime::{Diagnostic, PrnsEvent};
use prns_core::engine::{PathDiscoveryEvent, PathDiscoveryPhase};
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    OnceLock,
};

const LIMIT: u32 = 2048;
static ENABLED: OnceLock<bool> = OnceLock::new();
static SALT: OnceLock<Option<[u8; 32]>> = OnceLock::new();
static LINES: AtomicU32 = AtomicU32::new(0);
static CAPPED: AtomicBool = AtomicBool::new(false);

fn tag(salt: &[u8; 32], domain: &[u8], bytes: &[u8]) -> u64 {
    let digest = prns_core::crypto::hmac_sha256_chunks(salt, &[domain, bytes]);
    let mut short = [0; 8];
    short.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(short)
}

fn request_tag(bytes: &[u8; 16]) -> u64 {
    let digest = prns_core::crypto::sha256_chunks(&[b"prns.path.request.v1", bytes]);
    let mut short = [0; 8];
    short.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(short)
}

pub(super) fn enabled() -> bool {
    *ENABLED.get_or_init(|| platform::opted_in("PRNS_ROUTE_DIAGNOSTICS", c"debug.prns.routes"))
}

pub(crate) fn on_event(event: &PrnsEvent<'_>) {
    let PrnsEvent::Diagnostic(Diagnostic::PathDiscovery(event)) = event else {
        return;
    };
    if !enabled() {
        return;
    }
    if !take_budget(&LINES, LIMIT) {
        if !CAPPED.swap(true, Ordering::Relaxed) {
            let _ = writeln!(
                platform::output(),
                "[prns.routes] stage=diagnostics category=line_limit limit={LIMIT}"
            );
        }
        return;
    }
    let Some(salt) = SALT.get_or_init(|| {
        let mut salt = [0; 32];
        getrandom::getrandom(&mut salt).ok().map(|()| salt)
    }) else {
        let _ = writeln!(
            platform::output(),
            "[prns.routes] stage=diagnostics category=entropy_unavailable"
        );
        return;
    };
    write_event(&mut platform::output(), event, salt);
}

fn write_event(output: &mut impl Write, event: &PathDiscoveryEvent, salt: &[u8; 32]) {
    let destination = event
        .destination
        .map_or(0, |v| tag(salt, b"destination", v.as_bytes()));
    let interface = event
        .interface
        .map_or(0, |v| tag(salt, b"interface", v.as_bytes()));
    let request = event.request_tag.as_ref().map_or(0, request_tag);
    let kind = event.interface.and_then(|v| v.kind());
    let _ = write!(
        output,
        "[prns.routes] stage=path destination={destination:016x} interface={interface:016x} request={request:016x} has_destination={} has_interface={} has_request={} kind={kind:?} ",
        event.destination.is_some(),
        event.interface.is_some(),
        event.request_tag.is_some()
    );
    match event.phase {
        PathDiscoveryPhase::LocalRequest { .. } => {
            let _ = writeln!(output, "phase=local_request");
        }
        PathDiscoveryPhase::RequestIngress {
            disposition,
            mode,
            recursive,
            cached_interface,
        } => {
            // These closed enums contain only classifications, never packet data.
            let cached = cached_interface.map_or(0, |v| tag(salt, b"interface", v.as_bytes()));
            let _ = writeln!(
                output,
                "phase=request_ingress disposition={disposition:?} mode={mode:?} recursive={recursive} cached_interface={cached:016x}"
            );
        }
        PathDiscoveryPhase::RequestEgress(disposition) => {
            let _ = writeln!(output, "phase=request_egress disposition={disposition:?}");
        }
        PathDiscoveryPhase::ResponseIngress(disposition) => {
            let _ = writeln!(output, "phase=response_ingress disposition={disposition:?}");
        }
        PathDiscoveryPhase::ResponseDispatched => {
            let _ = writeln!(output, "phase=response_dispatched");
        }
        PathDiscoveryPhase::LocalTimeout { .. } => {
            let _ = writeln!(output, "phase=local_timeout");
        }
        PathDiscoveryPhase::RecursiveTimeout => {
            let _ = writeln!(output, "phase=recursive_timeout");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prns_core::{engine::PathRequestEgress, interfaces::InterfaceId, wire::DestinationHash};
    #[test]
    fn route_trace_keeps_phase_but_replaces_all_identifiers() {
        let event = PathDiscoveryEvent {
            at: None,
            destination: Some(DestinationHash::new([0x11; 16])),
            interface: Some(InterfaceId::new([0x22; 8])),
            request_tag: Some([0x33; 16]),
            phase: PathDiscoveryPhase::RequestEgress(PathRequestEgress::Offline),
        };
        let mut output = Vec::new();
        write_event(&mut output, &event, &[1; 32]);
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("phase=request_egress disposition=Offline"));
        for raw in ["1111111111111111", "2222222222222222", "3333333333333333"] {
            assert!(!text.contains(raw));
        }
        assert_ne!(
            tag(&[1; 32], b"destination", &[17; 16]),
            tag(&[2; 32], b"destination", &[17; 16])
        );
        assert_ne!(
            tag(&[1; 32], b"destination", &[17; 16]),
            tag(&[1; 32], b"request", &[17; 16])
        );
    }
}
