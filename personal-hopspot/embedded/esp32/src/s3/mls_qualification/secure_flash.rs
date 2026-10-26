//! Qualification only: the wrapping key is deliberately stored in plaintext
//! outside the secure backend's partition. This tests storage mechanics, not
//! confidentiality against physical flash extraction or rollback resistance.

use core::convert::Infallible;
use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};
use portable_atomic::{AtomicU32, AtomicU64, Ordering};
use prns_groups_store::{
    flash::Partition,
    secure::{Entropy, KeyProvider, SecureBackend, SecureError, SecureLimits},
    StoreIncarnation,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{Error, Result};
use crate::flash::{EspRomFlash, EspRomFlashError};

pub(super) const SLOT_BYTES: u32 = 4096;
const PARTITION_BYTES: u32 = 4 * 1024 * 1024;
const HEADER_BYTES: u32 = 8192;
const CAPSULES: usize = 4;
const CAPSULE_BYTES: u32 = CAPSULES as u32 * SLOT_BYTES;
const FLASH_BYTES: usize = 16 * 1024 * 1024;
const PARTITIONS: [(u32, &[u8]); 2] = [(0x600000, b"mls_alice"), (0xa00000, b"mls_bob")];
const HEADER_MAGIC: &[u8; 8] = b"MLSQKEY1";
const HEADER_MARKER: [u8; 256] = [0x69; 256];
const CAMPAIGN_MAGIC: u32 = 0x53454300;

#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut CAMPAIGN: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut EXPECTED_OUTPUT: [u8; 32] = [0; 32];
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut EXPECTED_STORAGE: [u8; 32] = [0; 32];
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut EXPECTED_CAPSULE: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut VERIFIED_CUT: u32 = 0;

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum Cut {
    CompactionPublished = 1,
    OldArenaSectorErased = 2,
    SendPublished = 3,
    OldCapsuleErased = 4,
}
impl Cut {
    fn label(self) -> &'static str {
        match self {
            Self::CompactionPublished => "compaction-published-before-retirement",
            Self::OldArenaSectorErased => "old-arena-one-sector-erased",
            Self::SendPublished => "send-published-before-retirement",
            Self::OldCapsuleErased => "old-capsule-erased-before-verification",
        }
    }
}

static ARMED: AtomicU32 = AtomicU32::new(0);
static PUBLISHED: AtomicU32 = AtomicU32::new(0);

struct IoCounter {
    count: AtomicU32,
    bytes: AtomicU32,
    micros: AtomicU32,
    max_micros: AtomicU32,
}
impl IoCounter {
    const fn new() -> Self {
        Self {
            count: AtomicU32::new(0),
            bytes: AtomicU32::new(0),
            micros: AtomicU32::new(0),
            max_micros: AtomicU32::new(0),
        }
    }
    fn add(&self, bytes: usize, start: embassy_time::Instant) {
        let elapsed = (embassy_time::Instant::now() - start).as_micros() as u32;
        self.count.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes as u32, Ordering::Relaxed);
        self.micros.fetch_add(elapsed, Ordering::Relaxed);
        self.max_micros.fetch_max(elapsed, Ordering::Relaxed);
    }
    fn take(&self) -> [u32; 4] {
        [
            self.count.swap(0, Ordering::Relaxed),
            self.bytes.swap(0, Ordering::Relaxed),
            self.micros.swap(0, Ordering::Relaxed),
            self.max_micros.swap(0, Ordering::Relaxed),
        ]
    }
}
static READS: IoCounter = IoCounter::new();
static WRITES: IoCounter = IoCounter::new();
static ERASES: IoCounter = IoCounter::new();
static PHASE_START: AtomicU64 = AtomicU64::new(0);

pub(super) fn begin_phase() -> embassy_time::Instant {
    READS.take();
    WRITES.take();
    ERASES.take();
    let start = embassy_time::Instant::now();
    PHASE_START.store(start.as_micros(), Ordering::Relaxed);
    start
}

/// Wall duration includes scheduling. I/O times cover each ROM-driver call,
/// including its bounce buffers, retries and core parking, but exclude yields.
pub(super) fn end_phase(label: &str, start: embassy_time::Instant) {
    let r = READS.take();
    let w = WRITES.take();
    let e = ERASES.take();
    log::info!("MLS secure phase={label} duration_us={} reads={} read_bytes={} read_us={} read_max_us={} writes={} write_bytes={} write_us={} write_max_us={} erases={} erase_bytes={} erase_us={} erase_max_us={}",
        (embassy_time::Instant::now() - start).as_micros(),
        r[0], r[1], r[2], r[3], w[0], w[1], w[2], w[3], e[0], e[1], e[2], e[3]);
    super::telemetry::sample(label);
}

pub(super) fn campaign() -> u32 {
    // SAFETY: The qualification worker alone reads/writes these persistent RTC words.
    let value = unsafe { core::ptr::addr_of!(CAMPAIGN).read_volatile() };
    if value & 0xffffff00 == CAMPAIGN_MAGIC && value & 0xff <= 4 {
        value & 0xff
    } else {
        0
    }
}

pub(super) fn prepare_campaign(alice_empty: bool, bob_empty: bool) {
    if alice_empty && bob_empty {
        // SAFETY: A completely new pair of stores starts a new single-worker campaign.
        unsafe {
            core::ptr::addr_of_mut!(CAMPAIGN).write_volatile(CAMPAIGN_MAGIC);
            core::ptr::addr_of_mut!(VERIFIED_CUT).write_volatile(0);
        }
        log::info!("MLS secure recovery campaign=fresh");
    }
}

pub(super) fn recovery_pending(cut: Cut) -> bool {
    // SAFETY: Sole qualification worker owns the RTC recovery bookkeeping.
    campaign() == cut as u32
        && unsafe { core::ptr::addr_of!(VERIFIED_CUT).read_volatile() } < cut as u32
}

pub(super) fn recovery_verified(cut: Cut) {
    // SAFETY: Called only by the worker after checking recovery evidence.
    unsafe { core::ptr::addr_of_mut!(VERIFIED_CUT).write_volatile(cut as u32) };
    log::info!(
        "MLS secure recovery cut={} outcome=verified exact-outbox=verified",
        cut as u32
    );
}

pub(super) fn arm(cut: Cut, output_digest: Option<[u8; 32]>) -> Result<()> {
    if campaign() + 1 != cut as u32 {
        return Err(Error::RecoveryDigest);
    }
    if let Some(digest) = output_digest {
        // SAFETY: Sole worker stores the pending-output expectation before enabling its cut.
        unsafe { core::ptr::addr_of_mut!(EXPECTED_OUTPUT).write_volatile(digest) };
    }
    PUBLISHED.store(0, Ordering::Release);
    ARMED.store(cut as u32, Ordering::Release);
    Ok(())
}

pub(super) fn verify_output(digest: [u8; 32]) -> Result<()> {
    // SAFETY: Sole worker reads an expectation published before the corresponding reset.
    let expected = unsafe { core::ptr::addr_of!(EXPECTED_OUTPUT).read_volatile() };
    if digest != expected {
        return Err(Error::RecoveryDigest);
    }
    Ok(())
}

fn reset_after_cut(cut: Cut) -> ! {
    // SAFETY: The worker publishes its completed cut only after all preceding flash calls return.
    unsafe { core::ptr::addr_of_mut!(CAMPAIGN).write_volatile(CAMPAIGN_MAGIC | cut as u32) };
    end_phase(
        cut.label(),
        embassy_time::Instant::from_micros(PHASE_START.load(Ordering::Relaxed)),
    );
    log::info!(
        "MLS secure cut={} boundary={} software-reset",
        cut as u32,
        cut.label()
    );
    esp_hal::system::software_reset()
}

pub(super) struct FixtureKey(Zeroizing<[u8; 32]>);
impl KeyProvider for FixtureKey {
    type Error = Infallible;
    fn wrapping_key(&mut self, output: &mut [u8; 32]) -> core::result::Result<(), Self::Error> {
        output.copy_from_slice(&*self.0);
        Ok(())
    }
}
pub(super) struct RuntimeEntropy;
impl Entropy for RuntimeEntropy {
    type Error = getrandom::Error;
    fn fill(&mut self, output: &mut [u8]) -> core::result::Result<(), Self::Error> {
        super::super::entropy::mls_random(output)
    }
}
pub(super) type Backend = SecureBackend<QualificationFlash, FixtureKey, RuntimeEntropy>;
pub(super) type BackendError = SecureError<EspRomFlashError, Infallible, getrandom::Error>;

pub(super) struct QualificationFlash(EspRomFlash);
impl ErrorType for QualificationFlash {
    type Error = EspRomFlashError;
}
impl ReadNorFlash for QualificationFlash {
    const READ_SIZE: usize = 4;
    fn capacity(&self) -> usize {
        FLASH_BYTES
    }
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> core::result::Result<(), Self::Error> {
        let start = embassy_time::Instant::now();
        let result = self.0.read(offset, bytes);
        READS.add(bytes.len(), start);
        esp_radio_rtos_driver::yield_task();
        result
    }
}
impl NorFlash for QualificationFlash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = SLOT_BYTES as usize;
    fn erase(&mut self, from: u32, to: u32) -> core::result::Result<(), Self::Error> {
        embedded_storage::nor_flash::check_erase(self, from, to)
            .map_err(EspRomFlashError::Contract)?;
        for sector in (from..to).step_by(SLOT_BYTES as usize) {
            let start = embassy_time::Instant::now();
            let result = self.0.erase(sector, sector + SLOT_BYTES);
            ERASES.add(SLOT_BYTES as usize, start);
            result?;
            let armed = ARMED.load(Ordering::Acquire);
            let published = PUBLISHED.load(Ordering::Acquire);
            let alice = PARTITIONS[0].0 + HEADER_BYTES;
            if armed == Cut::OldArenaSectorErased as u32
                && published != 0
                && (alice + CAPSULE_BYTES..PARTITIONS[0].0 + PARTITION_BYTES).contains(&sector)
            {
                reset_after_cut(Cut::OldArenaSectorErased);
            }
            if armed == Cut::OldCapsuleErased as u32
                && published != 0
                && sector != published
                && (alice..alice + CAPSULE_BYTES).contains(&sector)
            {
                self.capture_storage(published)?;
                reset_after_cut(Cut::OldCapsuleErased);
            }
            esp_radio_rtos_driver::yield_task();
        }
        Ok(())
    }
    fn write(&mut self, offset: u32, bytes: &[u8]) -> core::result::Result<(), Self::Error> {
        let start = embassy_time::Instant::now();
        let result = self.0.write(offset, bytes);
        WRITES.add(bytes.len(), start);
        result?;
        let alice = PARTITIONS[0].0 + HEADER_BYTES;
        let publication = (alice..alice + CAPSULE_BYTES).contains(&offset)
            && offset % SLOT_BYTES == SLOT_BYTES - 256
            && bytes == [0x3c; 256];
        if publication {
            let capsule = offset + 256 - SLOT_BYTES;
            PUBLISHED.store(capsule, Ordering::Release);
            match ARMED.load(Ordering::Acquire) {
                1 => reset_after_cut(Cut::CompactionPublished),
                3 => {
                    self.capture_storage(capsule)?;
                    reset_after_cut(Cut::SendPublished);
                }
                _ => {}
            }
        }
        esp_radio_rtos_driver::yield_task();
        Ok(())
    }
}

impl QualificationFlash {
    fn capture_storage(&mut self, capsule: u32) -> core::result::Result<(), EspRomFlashError> {
        let digest = storage_digest(self, capsule)?;
        // SAFETY: Sole worker publishes a complete capsule/immutable-data expectation before reset.
        unsafe {
            core::ptr::addr_of_mut!(EXPECTED_CAPSULE).write_volatile(capsule);
            core::ptr::addr_of_mut!(EXPECTED_STORAGE).write_volatile(digest);
        }
        Ok(())
    }
}

fn digest_range(
    flash: &mut QualificationFlash,
    digest: &mut Sha256,
    from: u32,
    to: u32,
) -> core::result::Result<(), EspRomFlashError> {
    let mut bytes = Zeroizing::new([0; 256]);
    for offset in (from..to).step_by(bytes.len()) {
        flash.read(offset, &mut *bytes)?;
        digest.update(&*bytes);
    }
    Ok(())
}

// Excludes predecessor capsules that recovery is required to erase. All object
// and catalog ciphertext plus the newly published capsule must remain identical.
fn storage_digest(
    flash: &mut QualificationFlash,
    capsule: u32,
) -> core::result::Result<[u8; 32], EspRomFlashError> {
    let mut digest = Sha256::new();
    digest_range(flash, &mut digest, capsule, capsule + SLOT_BYTES)?;
    digest_range(
        flash,
        &mut digest,
        PARTITIONS[0].0 + HEADER_BYTES + CAPSULE_BYTES,
        PARTITIONS[0].0 + PARTITION_BYTES,
    )?;
    Ok(digest.finalize().into())
}

pub(super) fn verify_storage() -> Result<()> {
    if !recovery_pending(Cut::SendPublished) && !recovery_pending(Cut::OldCapsuleErased) {
        return Ok(());
    }
    // SAFETY: Sole worker reads expectations published together before the reset.
    let (capsule, expected) = unsafe {
        (
            core::ptr::addr_of!(EXPECTED_CAPSULE).read_volatile(),
            core::ptr::addr_of!(EXPECTED_STORAGE).read_volatile(),
        )
    };
    let alice = PARTITIONS[0].0 + HEADER_BYTES;
    if !(alice..alice + CAPSULE_BYTES).contains(&capsule) || capsule % SLOT_BYTES != 0 {
        return Err(Error::RecoveryDigest);
    }
    if storage_digest(
        &mut QualificationFlash(EspRomFlash::new(FLASH_BYTES)),
        capsule,
    )
    .map_err(Error::FlashIo)?
        != expected
    {
        return Err(Error::RecoveryDigest);
    }
    log::info!(
        "MLS secure recovery cut={} immutable-ciphertext=verified",
        campaign()
    );
    Ok(())
}

fn erased(flash: &mut QualificationFlash, start: u32, length: u32) -> Result<bool> {
    let mut bytes = Zeroizing::new([0; 256]);
    for offset in (start..start + length).step_by(bytes.len()) {
        flash.read(offset, &mut *bytes).map_err(Error::FlashIo)?;
        if bytes.iter().any(|byte| *byte != 0xff) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn verify_single_capsule(peer: usize) -> Result<()> {
    let start = PARTITIONS.get(peer).ok_or(Error::Partition)?.0 + HEADER_BYTES;
    let mut flash = QualificationFlash(EspRomFlash::new(FLASH_BYTES));
    let mut live = 0;
    for slot in 0..CAPSULES {
        let offset = start + slot as u32 * SLOT_BYTES;
        if !erased(&mut flash, offset, SLOT_BYTES)? {
            let mut marker = [0; 256];
            flash
                .read(offset + SLOT_BYTES - 256, &mut marker)
                .map_err(Error::FlashIo)?;
            if marker != [0x3c; 256] {
                return Err(Error::RecoveryDigest);
            }
            live += 1;
        }
    }
    if live != 1 {
        return Err(Error::RecoveryDigest);
    }
    log::info!("MLS secure capsules peer={peer} live=1 retired_capsules_erased=3");
    Ok(())
}

pub(super) fn verify_layout() -> Result<()> {
    for &(offset, label) in &PARTITIONS {
        let region = personal_hopspot_memory::HELTEC_E290_MLS_QUALIFICATION
            .regions
            .iter()
            .find(|region| region.id.0.as_bytes() == label)
            .ok_or(Error::Partition)?;
        if region.range.start() != u64::from(offset)
            || region.range.byte_len() != u64::from(PARTITION_BYTES)
        {
            return Err(Error::Partition);
        }
    }
    let mut flash = QualificationFlash(EspRomFlash::new(FLASH_BYTES));
    let mut found = [false; 2];
    let mut entry = [0; 32];
    for index in 0..95 {
        flash
            .read(0x8000 + index * 32, &mut entry)
            .map_err(Error::FlashIo)?;
        if entry[..2] == [0xff, 0xff] || entry[..2] == [0xeb, 0xeb] {
            break;
        }
        if entry[..2] != [0xaa, 0x50] {
            return Err(Error::Partition);
        }
        let start = u32::from_le_bytes(entry[4..8].try_into().map_err(|_| Error::Partition)?);
        let length = u32::from_le_bytes(entry[8..12].try_into().map_err(|_| Error::Partition)?);
        let end = start.checked_add(length).ok_or(Error::Partition)?;
        for (peer, &(expected, label)) in PARTITIONS.iter().enumerate() {
            if start < expected + PARTITION_BYTES && end > expected {
                if found[peer]
                    || start != expected
                    || length != PARTITION_BYTES
                    || entry[2] != 0x47
                    || entry[3] != peer as u8
                    || &entry[12..12 + label.len()] != label
                    || entry[12 + label.len()..28].iter().any(|b| *b != 0)
                    || entry[28..32] != [0; 4]
                {
                    return Err(Error::Partition);
                }
                found[peer] = true;
            }
        }
    }
    if found.into_iter().all(|present| present) {
        Ok(())
    } else {
        Err(Error::Partition)
    }
}

pub(super) fn mount(peer: usize) -> Result<(Backend, StoreIncarnation)> {
    let start = PARTITIONS.get(peer).ok_or(Error::Partition)?.0;
    let mut flash = QualificationFlash(EspRomFlash::new(FLASH_BYTES));
    let mut header = Zeroizing::new([0; 256]);
    let mut marker = [0; 256];
    flash.read(start, &mut *header).map_err(Error::FlashIo)?;
    flash
        .read(start + SLOT_BYTES - 256, &mut marker)
        .map_err(Error::FlashIo)?;
    let fresh = header.iter().all(|b| *b == 0xff) && marker.iter().all(|b| *b == 0xff);
    if fresh {
        // Explicit first provision requires the WHOLE peer region to be blank.
        // A valid old fixture header with an empty store never triggers format.
        if !erased(&mut flash, start, PARTITION_BYTES)? {
            return Err(Error::FixtureMetadata);
        }
        header.fill(0xff);
        header[..8].copy_from_slice(HEADER_MAGIC);
        super::super::entropy::mls_random(&mut header[8..64]).map_err(|_| Error::Entropy)?;
        let checksum: [u8; 32] = Sha256::digest(&header[..64]).into();
        header[64..96].copy_from_slice(&checksum);
        // Deliberately publish this header only AFTER format. Interrupted
        // bootstrap is ambiguous and requires an explicit fixture erase.
        flash.write(start, &*header).map_err(Error::FlashIo)?;
    } else if marker != HEADER_MARKER
        || &header[..8] != HEADER_MAGIC
        || header[64..96] != Sha256::digest(&header[..64])[..]
        || header[96..].iter().any(|b| *b != 0xff)
        || !erased(&mut flash, start + SLOT_BYTES, SLOT_BYTES)?
    {
        return Err(Error::FixtureMetadata);
    }
    let incarnation = StoreIncarnation::new(
        header[8..32]
            .try_into()
            .map_err(|_| Error::FixtureMetadata)?,
    )
    .map_err(|_| Error::FixtureMetadata)?;
    let key = FixtureKey(Zeroizing::new(
        header[32..64]
            .try_into()
            .map_err(|_| Error::FixtureMetadata)?,
    ));
    let partition = Partition {
        offset: start + HEADER_BYTES,
        length: PARTITION_BYTES - HEADER_BYTES,
    };
    let limits = SecureLimits {
        capsule_sectors: CAPSULES,
        max_live_objects: 96,
        max_catalog_bytes: 32 * 1024,
        max_key_bytes: 64,
        max_value_bytes: 32 * 1024,
        max_changes: 48,
        max_receipts: 32,
        reserve_bytes: 128 * 1024,
    };
    let backend = if fresh {
        let backend = Backend::format(flash, partition, limits, incarnation, key, RuntimeEntropy)
            .map_err(Error::Flash)?;
        let (mut flash, key, entropy) = backend.into_parts();
        flash
            .write(start + SLOT_BYTES - 256, &HEADER_MARKER)
            .map_err(Error::FlashIo)?;
        let mut observed = Zeroizing::new([0; 256]);
        flash.read(start, &mut *observed).map_err(Error::FlashIo)?;
        flash
            .read(start + SLOT_BYTES - 256, &mut marker)
            .map_err(Error::FlashIo)?;
        if *observed != *header || marker != HEADER_MARKER {
            return Err(Error::FixtureMetadata);
        }
        Backend::mount(flash, partition, limits, incarnation, key, entropy).map_err(Error::Flash)?
    } else {
        Backend::mount(flash, partition, limits, incarnation, key, RuntimeEntropy)
            .map_err(Error::Flash)?
    };
    log::info!("MLS secure fixture-key=plaintext-header confidentiality=unqualified peer={peer} fresh={fresh}");
    log::info!("MLS secure flash peer={peer} usage={:?}", backend.usage());
    Ok((backend, incarnation))
}
