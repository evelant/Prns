//! Opt-in, bounded queue-drop evidence. No frames, stable IDs, or heap storage.
//! A missing event is not evidence that no frame was sent or discarded.
use core::cell::RefCell;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use personal_rns::lora::{LoRaQueueDrop, LoRaQueueSource};
use personal_rns::manifold::outbound::OutboundQueueDropReason;

const LIMIT: u16 = 512;
static COUNT: Mutex<CriticalSectionRawMutex, RefCell<u16>> = Mutex::new(RefCell::new(0));

pub(super) fn observe(event: LoRaQueueDrop) {
    let sequence = COUNT.lock(|count| {
        let mut count = count.borrow_mut();
        if *count > LIMIT {
            return None;
        }
        *count += 1;
        Some(*count)
    });
    let Some(sequence) = sequence else { return };
    if sequence > LIMIT {
        log::info!("prns.lora.queue limit_reached={LIMIT}");
        return;
    }
    let source = match event.source {
        LoRaQueueSource::RuntimeLane => "runtime_lane",
        LoRaQueueSource::DriverQueue => "driver_queue",
        LoRaQueueSource::Active => "active",
    };
    let reason = match event.reason {
        OutboundQueueDropReason::Expired => "expired",
        OutboundQueueDropReason::Disabled => "disabled",
        OutboundQueueDropReason::PreResume => "pre_resume",
        OutboundQueueDropReason::MissingTime => "missing_time",
        OutboundQueueDropReason::ClockRollback => "clock_rollback",
    };
    log::info!(
        "prns.lora.queue seq={} observed_ms={} phase=drop source={} reason={} age_ms={:?}",
        sequence,
        embassy_time::Instant::now().as_millis(),
        source,
        reason,
        event.age_millis,
    );
}
