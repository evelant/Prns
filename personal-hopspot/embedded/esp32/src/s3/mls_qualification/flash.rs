use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};
use prns_groups_store::flash::{FlashBackend, FlashLimits, Partition};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{Error, Result};
use crate::flash::{EspRomFlash, EspRomFlashError};

pub(super) const SLOT_BYTES: u32 = 64 * 1024;
const PARTITION_BYTES: u32 = 4 * 1024 * 1024;
const FLASH_BYTES: usize = 16 * 1024 * 1024;
const TABLE_OFFSET: u32 = 0x8000;
const PARTITIONS: [(u32, &[u8]); 2] = [(0x600000, b"mls_alice"), (0xa00000, b"mls_bob")];
const CAMPAIGN_MAGIC: u32 = 0x4d4c5300;

#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut CAMPAIGN: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut EXPECTED_SLOT: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut EXPECTED_DIGEST: [u8; 32] = [0; 32];

#[derive(Clone, Copy)]
pub(super) enum Cut {
    BeforePublish,
    TornBody,
    AfterPublish,
}
static ARMED: portable_atomic::AtomicU32 = portable_atomic::AtomicU32::new(0);

fn campaign() -> u32 {
    // SAFETY: Only the serial qualification worker accesses these RTC words.
    let value = unsafe { core::ptr::addr_of!(CAMPAIGN).read_volatile() };
    if value & 0xffffff00 == CAMPAIGN_MAGIC {
        value & 0xff
    } else {
        0
    }
}

pub(super) fn arm(cut: Cut) {
    let index = match cut {
        Cut::BeforePublish => 1,
        Cut::TornBody => 2,
        Cut::AfterPublish => 3,
    };
    if campaign() + 1 == index {
        ARMED.store(index, portable_atomic::Ordering::Release);
    }
}

fn reset_after_cut(index: u32) -> ! {
    // SAFETY: The one worker publishes progress before resetting. No other reader/writer exists.
    unsafe { core::ptr::addr_of_mut!(CAMPAIGN).write_volatile(CAMPAIGN_MAGIC | index) };
    log::info!("MLS cut={} software-reset", index);
    esp_hal::system::software_reset()
}

pub(super) struct QualificationFlash(EspRomFlash);
pub(super) type Backend = FlashBackend<QualificationFlash>;
impl ErrorType for QualificationFlash {
    type Error = EspRomFlashError;
}
impl ReadNorFlash for QualificationFlash {
    const READ_SIZE: usize = 4;
    fn capacity(&self) -> usize {
        FLASH_BYTES
    }
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> core::result::Result<(), Self::Error> {
        let result = self.0.read(offset, bytes);
        esp_radio_rtos_driver::yield_task();
        result
    }
}
impl NorFlash for QualificationFlash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    fn erase(&mut self, from: u32, to: u32) -> core::result::Result<(), Self::Error> {
        self.0.erase(from, to)
    }
    fn write(&mut self, offset: u32, bytes: &[u8]) -> core::result::Result<(), Self::Error> {
        let publication = offset >= PARTITIONS[0].0
            && offset < PARTITIONS[1].0
            && offset % SLOT_BYTES == SLOT_BYTES - 256
            && bytes.len() == 256;
        let armed = ARMED.load(portable_atomic::Ordering::Acquire);
        let body_start = offset >= PARTITIONS[0].0
            && offset < PARTITIONS[1].0
            && offset % SLOT_BYTES == 0
            && bytes.len() >= 128;
        let cut = if publication || (armed == 2 && body_start) {
            ARMED.swap(0, portable_atomic::Ordering::AcqRel)
        } else {
            0
        };
        match cut {
            1 => reset_after_cut(cut),
            2 => {
                self.0.write(offset, &bytes[..128])?;
                reset_after_cut(cut)
            }
            _ => {}
        }
        self.0.write(offset, bytes)?;
        if cut == 3 {
            let start = offset + 256 - SLOT_BYTES;
            let digest = slot_digest(&mut self.0, start)?;
            // SAFETY: Sole worker writes the recovery expectation before CAMPAIGN publication/reset.
            unsafe {
                core::ptr::addr_of_mut!(EXPECTED_SLOT).write_volatile(start);
                core::ptr::addr_of_mut!(EXPECTED_DIGEST).write_volatile(digest);
            }
            reset_after_cut(cut)
        }
        esp_radio_rtos_driver::yield_task();
        Ok(())
    }
}

fn slot_digest(
    flash: &mut EspRomFlash,
    start: u32,
) -> core::result::Result<[u8; 32], EspRomFlashError> {
    let mut digest = Sha256::new();
    let mut buffer = Zeroizing::new([0; 256]);
    for offset in (start..start + SLOT_BYTES).step_by(buffer.len()) {
        flash.read(offset, &mut *buffer)?;
        digest.update(&*buffer);
    }
    Ok(digest.finalize().into())
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
    let mut flash = EspRomFlash::new(FLASH_BYTES);
    let mut found = [false; 2];
    let mut entry = [0; 32];
    for index in 0..95 {
        flash
            .read(TABLE_OFFSET + index * 32, &mut entry)
            .map_err(Error::FlashIo)?;
        if entry[..2] == [0xff, 0xff] {
            break;
        }
        if entry[..2] == [0xeb, 0xeb] {
            break;
        }
        if entry[..2] != [0xaa, 0x50] {
            return Err(Error::Partition);
        }
        let start = u32::from_le_bytes(entry[4..8].try_into().map_err(|_| Error::Partition)?);
        let length = u32::from_le_bytes(entry[8..12].try_into().map_err(|_| Error::Partition)?);
        let end = start.checked_add(length).ok_or(Error::Partition)?;
        for (peer, &(expected_start, label)) in PARTITIONS.iter().enumerate() {
            if start < expected_start + PARTITION_BYTES && end > expected_start {
                if found[peer]
                    || start != expected_start
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
    if !found.into_iter().all(|item| item) {
        return Err(Error::Partition);
    }
    Ok(())
}

pub(super) fn prepare_campaign(
    alice: prns_groups_store::Revision,
    bob: prns_groups_store::Revision,
) -> Result<()> {
    if alice.0 == 0 && bob.0 == 0 {
        // SAFETY: Both stores are empty, so an explicitly erased fixture starts a fresh campaign.
        unsafe { core::ptr::addr_of_mut!(CAMPAIGN).write_volatile(CAMPAIGN_MAGIC) };
        log::info!("MLS recovery campaign=fresh");
        return Ok(());
    }
    let mut flash = EspRomFlash::new(FLASH_BYTES);
    if campaign() == 3 {
        // SAFETY: Sole worker reads a fully published pre-reset expectation.
        let (slot, expected) = unsafe {
            (
                core::ptr::addr_of!(EXPECTED_SLOT).read_volatile(),
                core::ptr::addr_of!(EXPECTED_DIGEST).read_volatile(),
            )
        };
        if !(PARTITIONS[0].0..PARTITIONS[0].0 + PARTITION_BYTES).contains(&slot)
            || slot % SLOT_BYTES != 0
            || slot_digest(&mut flash, slot).map_err(Error::FlashIo)? != expected
        {
            return Err(Error::RecoveryDigest);
        }
        log::info!("MLS recovery published-slot-sha256=verified slot={slot:#x}");
        // SAFETY: Sole worker marks the already-verified recovery expectation consumed.
        unsafe { core::ptr::addr_of_mut!(CAMPAIGN).write_volatile(CAMPAIGN_MAGIC | 4) };
    }
    Ok(())
}

pub(super) fn mount(peer: usize) -> Result<Backend> {
    let backend = FlashBackend::mount(
        QualificationFlash(EspRomFlash::new(FLASH_BYTES)),
        Partition {
            offset: PARTITIONS[peer].0,
            length: PARTITION_BYTES,
        },
        FlashLimits {
            slot_bytes: SLOT_BYTES,
            max_changes: 48,
            max_key_bytes: 64,
            max_value_bytes: 32 * 1024,
        },
    )
    .map_err(Error::Flash)?;
    log::info!("MLS flash peer={peer} usage={:?}", backend.usage());
    Ok(backend)
}
