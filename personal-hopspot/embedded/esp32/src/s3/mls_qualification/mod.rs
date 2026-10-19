mod crypto_probe;
#[cfg(not(feature = "group-mls-secure-qualification"))]
mod flash;
#[cfg(feature = "group-mls-secure-qualification")]
#[path = "secure_flash.rs"]
mod flash;
#[cfg(not(feature = "group-mls-secure-qualification"))]
mod scenario;
#[cfg(feature = "group-mls-secure-qualification")]
#[path = "secure_scenario.rs"]
mod scenario;
mod telemetry;

use allocator_api2::alloc::Allocator;
use core::{alloc::Layout, ffi::c_void};
use personal_rns::interfaces::InterfaceStatus;
use personal_rns::manifold::embassy::EmbassyInterfaceStatus;
use portable_atomic::{AtomicU32, Ordering};

const STACK_BYTES: usize = 40 * 1024;
const GUARD_BYTES: usize = 4096;
const INTERNAL_RESERVE_BYTES: usize = 12 * 1024;
static WORKER: AtomicU32 = AtomicU32::new(0);
static START_SIGNAL: AtomicU32 = AtomicU32::new(0);
#[cfg(not(feature = "group-mls-secure-qualification"))]
type FlashError = prns_groups_store::flash::FlashError<crate::flash::EspRomFlashError>;
#[cfg(feature = "group-mls-secure-qualification")]
type FlashError = flash::BackendError;
type GroupAdapter = prns_groups_mls::Adapter<flash::Backend>;
type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
enum Error {
    Flash(FlashError),
    FlashIo(crate::flash::EspRomFlashError),
    Store(prns_groups_store::StoreError<FlashError>),
    Mls(prns_groups_mls::Error<FlashError>),
    Policy(prns_groups_mls::PolicyError),
    Partition,
    IdentityRecord,
    Commit,
    RecoveryDigest,
    Application,
    #[cfg(feature = "group-mls-secure-qualification")]
    FixtureMetadata,
    #[cfg(feature = "group-mls-secure-qualification")]
    Entropy,
    #[cfg(feature = "group-mls-secure-qualification")]
    Maintenance,
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Flash(error) => write!(f, "flash {error:?}"),
            Self::FlashIo(error) => write!(f, "flash IO {error:?}"),
            Self::Store(error) => write!(f, "store {error:?}"),
            Self::Mls(error) => write!(f, "MLS {error:?}"),
            Self::Policy(error) => write!(f, "identity policy {error:?}"),
            Self::Partition => f.write_str("qualification partition table mismatch"),
            Self::IdentityRecord => f.write_str("invalid durable identity"),
            Self::Commit => f.write_str("commit not confirmed"),
            Self::RecoveryDigest => f.write_str("recovery bytes differ"),
            Self::Application => f.write_str("application payload differs"),
            #[cfg(feature = "group-mls-secure-qualification")]
            Self::FixtureMetadata => f.write_str("insecure fixture metadata missing, corrupt, or ambiguous; explicitly erase fixture to reprovision"),
            #[cfg(feature = "group-mls-secure-qualification")]
            Self::Entropy => f.write_str("runtime entropy unavailable"),
            #[cfg(feature = "group-mls-secure-qualification")]
            Self::Maintenance => f.write_str("maintenance or retirement fence not confirmed"),
        }
    }
}
impl From<prns_groups_store::StoreError<FlashError>> for Error {
    fn from(error: prns_groups_store::StoreError<FlashError>) -> Self {
        Self::Store(error)
    }
}
impl From<prns_groups_mls::Error<FlashError>> for Error {
    fn from(error: prns_groups_mls::Error<FlashError>) -> Self {
        Self::Mls(error)
    }
}

pub(super) fn reserve_worker() {
    // esp-rtos adds a guard word, the configured guard offset, and rounds up to the next 16-byte boundary.
    let reservation_bytes = ((STACK_BYTES + GUARD_BYTES + 4) & !15) + 16;
    let layout = Layout::from_size_align(reservation_bytes, 16).expect("fixed worker stack layout");
    let reserved = critical_section::with(|_| {
        let Ok(probe) = esp_alloc::InternalMemory.allocate(layout) else {
            return false;
        };
        // SAFETY: The probe is live and exclusively owned. Deallocation and task creation share
        // one cross-core critical section, so another allocation cannot consume this exact hole.
        // The callback owns no borrowed input; its null parameter is never dereferenced.
        unsafe {
            esp_alloc::InternalMemory.deallocate(probe.cast(), layout);
            esp_radio_rtos_driver::task_create(
                "mls-qualification",
                worker,
                core::ptr::null_mut(),
                0,
                Some(0),
                STACK_BYTES,
            );
        }
        true
    });
    WORKER.store(if reserved { 1 } else { 3 }, Ordering::Release);
    log::info!("MLS worker reservation contiguous_bytes={reservation_bytes} success={reserved}");
}

extern "C" fn worker(_: *mut c_void) {
    while START_SIGNAL.load(Ordering::Acquire) == 0 {
        esp_radio_rtos_driver::usleep(10_000);
    }
    if START_SIGNAL.load(Ordering::Acquire) != 1 {
        return;
    }
    telemetry::sample("worker-start");
    match scenario::run() {
        Ok(()) => WORKER.store(2, Ordering::Release),
        Err(error) => {
            log::error!("MLS qualification failed: {error}");
            WORKER.store(3, Ordering::Release);
        }
    }
    telemetry::sample("worker-end");
}

#[embassy_executor::task]
pub(super) async fn start(
    lora: &'static EmbassyInterfaceStatus,
    usb: &'static EmbassyInterfaceStatus,
    espnow: &'static EmbassyInterfaceStatus,
) {
    embassy_time::Timer::after_secs(12).await;
    telemetry::sample("radio-baseline");
    log::info!(
        "MLS wifi phase=baseline {}",
        esp_radio::wifi::data_path_diagnostics()
    );
    let internal_free = esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::Internal.into());
    if WORKER.load(Ordering::Acquire) != 1 || internal_free < INTERNAL_RESERVE_BYTES {
        START_SIGNAL.store(2, Ordering::Release);
        log::error!("MLS worker refused: internal_free={internal_free} reserve={INTERNAL_RESERVE_BYTES} state={}", WORKER.load(Ordering::Acquire));
        return;
    }
    log::info!("MLS qualification start stack_bytes={STACK_BYTES} stack_guard_bytes={GUARD_BYTES} flash_slot_bytes={} entropy=hardware-seeded-runtime fixture_peers=2 max_members=4", flash::SLOT_BYTES);
    telemetry::sampler_tick();
    START_SIGNAL.store(1, Ordering::Release);
    let mut samples = 0;
    loop {
        embassy_time::Timer::after_millis(10).await;
        telemetry::sampler_tick();
        samples += 1;
        if samples % 100 == 0 || WORKER.load(Ordering::Acquire) != 1 {
            telemetry::sample("radio-concurrent");
            log::info!(
                "MLS wifi phase=concurrent {}",
                esp_radio::wifi::data_path_diagnostics()
            );
            log::info!("MLS radio lora_rx={} lora_tx={} usb_rx={} usb_tx={} espnow_rx={} espnow_tx={} status={}",
                lora.rx_bytes(), lora.tx_bytes(), usb.rx_bytes(), usb.tx_bytes(), espnow.rx_bytes(), espnow.tx_bytes(), WORKER.load(Ordering::Acquire));
        }
        if WORKER.load(Ordering::Acquire) != 1 {
            break;
        }
    }
}
