//! An independent, bounded stream-health signal for foreground qualification.
use super::platform;
use std::io::Write;
use std::sync::Once;
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(5);
const LIMIT: u32 = 1440; // Two hours; no unbounded diagnostic background worker.
static START: Once = Once::new();

pub(super) fn start() {
    START.call_once(|| {
        let mut random = [0_u8; 8];
        if getrandom::getrandom(&mut random).is_err() {
            let _ = writeln!(platform::output(), "[prns.trial] stage=diagnostics category=entropy_unavailable");
            return;
        }
        let session = u64::from_be_bytes(random);
        if std::thread::Builder::new().name("prns-trial-health".into()).spawn(move || {
            let started = Instant::now();
            for sequence in 0..LIMIT {
                let groups_ready = super::DELIVERY_SINK_SEEN.load(std::sync::atomic::Ordering::Relaxed)
                    && log::log_enabled!(target: super::DELIVERY_TARGET, log::Level::Info);
                let bluetooth_ready = super::BLUETOOTH_SINK_SEEN.load(std::sync::atomic::Ordering::Relaxed)
                    && log::log_enabled!(target: super::BLUETOOTH_TARGET, log::Level::Info);
                let routes_ready = super::routes::enabled();
                let (schedule_sequence, schedule_healthy) = prns_groups_runner_native::metadata_schedule_snapshot();
                let schedule_ready = super::schedule::ready() && schedule_healthy;
                let (link_sequence, links_ready) = super::links::snapshot();
                let _ = writeln!(platform::output(), "[prns.trial] stage=heartbeat session={session:016x} sequence={sequence} elapsed_ms={} groups_ready={groups_ready} bluetooth_ready={bluetooth_ready} routes_ready={routes_ready} schedule_ready={schedule_ready} schedule_sequence={schedule_sequence} links_ready={links_ready} link_sequence={link_sequence}", started.elapsed().as_millis());
                std::thread::sleep(INTERVAL);
            }
            let _ = writeln!(platform::output(), "[prns.trial] stage=diagnostics category=heartbeat_limit session={session:016x} limit={LIMIT}");
        }).is_err() {
            let _ = writeln!(platform::output(), "[prns.trial] stage=diagnostics category=heartbeat_start_failed");
        }
    });
}
