//! Opt-in metadata-only diagnostics for a controlled foreground phone trial.
//! This never replaces an embedding's logger or prints arbitrary error values.
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Once, OnceLock,
};

mod heartbeat;
pub(crate) mod links;
// Platform diagnostics are the app's only handwritten unsafe boundary.
#[allow(unsafe_code)]
mod platform;
mod routes;
mod schedule;
pub(crate) use routes::on_event;

const TARGET: &str = "prns.groups.diagnostics";
const DELIVERY_TARGET: &str = "prns.groups.delivery";
const BLUETOOTH_TARGET: &str = "prns.bluetooth.diagnostics";
const LINE_LIMIT: u32 = 2048;
// Covers both lines of each bounded push trace and all receipt transitions.
// Routine setup/inventory events cannot consume this independent reserve.
const DELIVERY_LINE_LIMIT: u32 = 1024;
const BLUETOOTH_LINE_LIMIT: u32 = 512;
static ENABLED: OnceLock<bool> = OnceLock::new();
static BLUETOOTH_ENABLED: OnceLock<bool> = OnceLock::new();
static TRIAL_ENABLED: OnceLock<bool> = OnceLock::new();
static INSTALL: Once = Once::new();
static LOGGER: GroupLogger = GroupLogger;
static LINES: AtomicU32 = AtomicU32::new(0);
static INGRESS_LINES: AtomicU32 = AtomicU32::new(0);
static LINE_LIMIT_REPORTED: AtomicBool = AtomicBool::new(false);
static DELIVERY_LINES: AtomicU32 = AtomicU32::new(0);
static DELIVERY_LIMIT_REPORTED: AtomicBool = AtomicBool::new(false);
static INGRESS_LIMIT_REPORTED: AtomicBool = AtomicBool::new(false);
static BLUETOOTH_LINES: AtomicU32 = AtomicU32::new(0);
static BLUETOOTH_LIMIT_REPORTED: AtomicBool = AtomicBool::new(false);
static DELIVERY_SINK_SEEN: AtomicBool = AtomicBool::new(false);
static BLUETOOTH_SINK_SEEN: AtomicBool = AtomicBool::new(false);

fn enabled() -> bool {
    *ENABLED.get_or_init(|| platform::opted_in("PRNS_GROUP_DIAGNOSTICS", c"debug.prns.groups"))
}
fn opted_in(value: Option<&str>) -> bool {
    value == Some("1")
}

fn bluetooth_enabled() -> bool {
    *BLUETOOTH_ENABLED
        .get_or_init(|| platform::opted_in("PRNS_BLUETOOTH_DIAGNOSTICS", c"debug.prns.bluetooth"))
}

pub(crate) fn install() {
    if enabled() {
        prns_groups_runner_native::enable_metadata_diagnostics();
    }
    if enabled() || bluetooth_enabled() {
        INSTALL.call_once(|| {
            // An existing logger remains authoritative; the restoration probe
            // explicitly forwards these safe targets when their flags are enabled.
            if log::set_logger(&LOGGER).is_ok() {
                log::set_max_level(LevelFilter::Info);
                if enabled() {
                    log::info!(target: TARGET, "stage=logger category=installed frame_bytes=0");
                }
                if bluetooth_enabled() {
                    log::info!(target: BLUETOOTH_TARGET, "stage=logger category=installed");
                }
            }
            // Probe the exact sinks even when another logger owns the facade.
            // A heartbeat alone must not imply that delivery logs are visible.
            if enabled() {
                log::info!(target: DELIVERY_TARGET, "stage=logger category=delivery_probe");
                log::info!(target: schedule::TARGET, "stage=logger category=schedule_probe");
            }
            if bluetooth_enabled() {
                log::info!(target: BLUETOOTH_TARGET, "stage=logger category=bluetooth_probe");
            }
        });
    }
    if *TRIAL_ENABLED
        .get_or_init(|| platform::opted_in("PRNS_TRIAL_DIAGNOSTICS", c"debug.prns.trial"))
    {
        heartbeat::start();
    }
}
struct GroupLogger;
impl Log for GroupLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        accepts(metadata)
    }
    fn log(&self, record: &Record<'_>) {
        let _ = write_record(record);
    }
    fn flush(&self) {}
}

pub(crate) fn accepts(metadata: &Metadata<'_>) -> bool {
    accepts_target(
        metadata.target(),
        metadata.level(),
        enabled(),
        bluetooth_enabled(),
    )
}

fn accepts_target(target: &str, level: Level, groups: bool, bluetooth: bool) -> bool {
    level <= Level::Info
        && ((groups && matches!(target, TARGET | DELIVERY_TARGET | schedule::TARGET))
            || (bluetooth && target == BLUETOOTH_TARGET))
}

/// Allow an existing restoration-probe logger to forward only the exact safe targets.
/// Producers pass classifications and counts; group events also use process-salted tags,
/// never payloads, raw identifiers or cryptographic material.
pub(crate) fn write_record(record: &Record<'_>) -> bool {
    if !accepts(record.metadata()) {
        return false;
    }
    if record.target() == schedule::TARGET {
        schedule::write(record.args());
        return true;
    }
    if record.target() == BLUETOOTH_TARGET {
        BLUETOOTH_SINK_SEEN.store(true, Ordering::Relaxed);
        write_bounded_bluetooth(
            &mut platform::output(),
            record.args(),
            &BLUETOOTH_LINES,
            &BLUETOOTH_LIMIT_REPORTED,
            BLUETOOTH_LINE_LIMIT,
        );
        return true;
    }
    if record.target() == DELIVERY_TARGET {
        DELIVERY_SINK_SEEN.store(true, Ordering::Relaxed);
        write_bounded_delivery(
            &mut platform::output(),
            record.args(),
            &DELIVERY_LINES,
            &DELIVERY_LIMIT_REPORTED,
            DELIVERY_LINE_LIMIT,
        );
        return true;
    }
    if take_budget(&LINES, LINE_LIMIT) {
        let _ = writeln!(platform::output(), "[prns.groups] {}", record.args());
    } else if !LINE_LIMIT_REPORTED.swap(true, Ordering::Relaxed) {
        let _ = writeln!(
            platform::output(),
            "[prns.groups] stage=diagnostics category=line_limit limit={LINE_LIMIT} dropped_at_least=1"
        );
    }
    true
}

fn write_bounded_delivery(
    output: &mut impl Write,
    message: &std::fmt::Arguments<'_>,
    lines: &AtomicU32,
    reported: &AtomicBool,
    limit: u32,
) {
    if take_budget(lines, limit) {
        let _ = writeln!(output, "[prns.groups] {message}");
    } else if !reported.swap(true, Ordering::Relaxed) {
        let _ = writeln!(
            output,
            "[prns.groups] stage=diagnostics category=delivery_line_limit limit={limit} dropped_at_least=1"
        );
    }
}

fn write_bounded_bluetooth(
    output: &mut impl Write,
    message: &std::fmt::Arguments<'_>,
    lines: &AtomicU32,
    reported: &AtomicBool,
    limit: u32,
) {
    if take_budget(lines, limit) {
        let _ = writeln!(output, "[prns.bluetooth] {message}");
    } else if !reported.swap(true, Ordering::Relaxed) {
        let _ = writeln!(
            output,
            "[prns.bluetooth] stage=diagnostics category=line_limit limit={limit} dropped_at_least=1"
        );
    }
}

pub(crate) fn ingress(
    stage: &'static str,
    outcome: prns_groups_reticulum::IngressOutcome,
    bytes: usize,
    identified: bool,
) {
    use prns_groups_reticulum::IngressOutcome;
    if !enabled() || outcome == IngressOutcome::Ignored {
        return;
    }
    if !take_budget(&INGRESS_LINES, 64) {
        if !INGRESS_LIMIT_REPORTED.swap(true, Ordering::Relaxed) {
            log::info!(target: TARGET, "stage=diagnostics category=ingress_limit limit=64 dropped_at_least=1");
        }
        return;
    }
    let category = match outcome {
        IngressOutcome::Ignored => return,
        IngressOutcome::Queued => "queued",
        IngressOutcome::Rejected if !identified => "unidentified",
        IngressOutcome::Rejected => "rejected",
        IngressOutcome::Full => "queue_full",
    };
    log::info!(target: TARGET, "stage={} category={} frame_bytes={}", stage, category, bytes);
}

fn take_budget(counter: &AtomicU32, limit: u32) -> bool {
    {
        let mut previous = counter.load(Ordering::Relaxed);
        loop {
            let Some(next) = (previous < limit).then(|| previous + 1) else {
                break Err(previous);
            };
            match counter.compare_exchange_weak(
                previous,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(previous) => break Ok(previous),
                Err(current) => previous = current,
            }
        }
    }
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_opt_in_requires_exact_explicit_value() {
        assert!(opted_in(Some("1")));
        for value in [None, Some(""), Some("0"), Some("true"), Some(" 1")] {
            assert!(!opted_in(value));
        }
    }
    #[test]
    fn diagnostic_stderr_and_ingress_are_bounded() {
        let count = AtomicU32::new(0);
        for _ in 0..LINE_LIMIT {
            assert!(take_budget(&count, LINE_LIMIT));
        }
        assert!(!take_budget(&count, LINE_LIMIT));
        assert_eq!(count.load(Ordering::Relaxed), LINE_LIMIT);
        count.store(u32::MAX, Ordering::Relaxed);
        assert!(!take_budget(&count, LINE_LIMIT));
    }

    #[test]
    fn bluetooth_and_group_opt_ins_select_only_their_exact_safe_targets() {
        for groups in [false, true] {
            for bluetooth in [false, true] {
                assert_eq!(
                    accepts_target(TARGET, Level::Info, groups, bluetooth),
                    groups
                );
                assert_eq!(
                    accepts_target(DELIVERY_TARGET, Level::Info, groups, bluetooth),
                    groups
                );
                assert_eq!(
                    accepts_target(schedule::TARGET, Level::Info, groups, bluetooth),
                    groups
                );
                assert_eq!(
                    accepts_target(BLUETOOTH_TARGET, Level::Info, groups, bluetooth),
                    bluetooth
                );
                for target in [
                    "prns_ffi::bluetooth_auto::macos::central",
                    "prns.bluetooth.diagnostics.extra",
                    "prns.groups.delivery.extra",
                    "prns.groups.schedule.extra",
                    "other",
                ] {
                    assert!(!accepts_target(target, Level::Info, groups, bluetooth));
                }
                assert!(!accepts_target(
                    BLUETOOTH_TARGET,
                    Level::Debug,
                    groups,
                    bluetooth
                ));
            }
        }
    }

    #[test]
    fn delivery_reserve_survives_exhausted_setup_budget_and_caps_explicitly() {
        let setup = AtomicU32::new(LINE_LIMIT);
        assert!(!take_budget(&setup, LINE_LIMIT));
        let delivery = AtomicU32::new(0);
        let reported = AtomicBool::new(false);
        let mut output = Vec::new();
        for _ in 0..3 {
            write_bounded_delivery(
                &mut output,
                &format_args!("stage=push phase=finish category=accepted"),
                &delivery,
                &reported,
                1,
            );
        }
        assert_eq!(
            String::from_utf8(output).expect("ASCII output"),
            "[prns.groups] stage=push phase=finish category=accepted\n[prns.groups] stage=diagnostics category=delivery_line_limit limit=1 dropped_at_least=1\n"
        );
        assert_eq!(setup.load(Ordering::Relaxed), LINE_LIMIT);
        assert_eq!(delivery.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn bluetooth_sink_emits_one_explicit_cap_without_formatting_later_records() {
        struct MustNotFormat;
        impl std::fmt::Display for MustNotFormat {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("capped message must not be formatted");
            }
        }
        let lines = AtomicU32::new(0);
        let reported = AtomicBool::new(false);
        let mut output = Vec::new();
        write_bounded_bluetooth(
            &mut output,
            &format_args!("stage=scan category=request enabled=true"),
            &lines,
            &reported,
            1,
        );
        write_bounded_bluetooth(
            &mut output,
            &format_args!("{}", MustNotFormat),
            &lines,
            &reported,
            1,
        );
        write_bounded_bluetooth(
            &mut output,
            &format_args!("{}", MustNotFormat),
            &lines,
            &reported,
            1,
        );
        assert_eq!(
            String::from_utf8(output).expect("ASCII output"),
            "[prns.bluetooth] stage=scan category=request enabled=true\n[prns.bluetooth] stage=diagnostics category=line_limit limit=1 dropped_at_least=1\n"
        );
    }
}
