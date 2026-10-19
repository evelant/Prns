//! Independent output reserve for opt-in scheduling evidence.
use super::{platform, take_budget};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub(super) const TARGET: &str = "prns.groups.schedule";
// One probe, 1,024 producer events, and its explicit exhaustion marker.
const LIMIT: u32 = 1026;
static LINES: AtomicU32 = AtomicU32::new(0);
static REPORTED: AtomicBool = AtomicBool::new(false);
static SEEN: AtomicBool = AtomicBool::new(false);

pub(super) fn ready() -> bool {
    SEEN.load(Ordering::Relaxed) && log::log_enabled!(target: TARGET, log::Level::Info)
}

pub(super) fn write(message: &std::fmt::Arguments<'_>) {
    SEEN.store(true, Ordering::Relaxed);
    write_bounded(&mut platform::output(), message, &LINES, &REPORTED, LIMIT);
}

fn write_bounded(
    output: &mut impl Write,
    message: &std::fmt::Arguments<'_>,
    lines: &AtomicU32,
    reported: &AtomicBool,
    limit: u32,
) {
    if take_budget(lines, limit) {
        let _ = writeln!(output, "[prns.schedule] {message}");
    } else if !reported.swap(true, Ordering::Relaxed) {
        let _ = writeln!(
            output,
            "[prns.schedule] stage=diagnostics category=schedule_line_limit family=schedule limit={limit} dropped_at_least=1"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_reserve_is_independent_and_does_not_format_after_its_cap() {
        struct MustNotFormat;
        impl std::fmt::Display for MustNotFormat {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("capped schedule record must not be formatted");
            }
        }
        let unrelated = AtomicU32::new(super::super::LINE_LIMIT);
        assert!(!take_budget(&unrelated, super::super::LINE_LIMIT));
        let lines = AtomicU32::new(0);
        let reported = AtomicBool::new(false);
        let mut output = Vec::new();
        write_bounded(
            &mut output,
            &format_args!("stage=schedule category=admitted sequence=1"),
            &lines,
            &reported,
            1,
        );
        for _ in 0..2 {
            write_bounded(
                &mut output,
                &format_args!("{MustNotFormat}"),
                &lines,
                &reported,
                1,
            );
        }
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "[prns.schedule] stage=schedule category=admitted sequence=1\n[prns.schedule] stage=diagnostics category=schedule_line_limit family=schedule limit=1 dropped_at_least=1\n"
        );
        assert_eq!(lines.load(Ordering::Relaxed), 1);
    }
}
