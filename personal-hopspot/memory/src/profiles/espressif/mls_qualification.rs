//! Experimental ownership contract, deliberately outside the shipping profile catalog.

use super::*;

const REGIONS: [MemoryRegion; 14] = [
    ESP_16_MIB_REGIONS[0],
    ESP_16_MIB_REGIONS[1],
    ESP_16_MIB_REGIONS[2],
    ESP_16_MIB_REGIONS[3],
    ESP_16_MIB_REGIONS[4],
    ESP_16_MIB_REGIONS[5],
    ESP_16_MIB_REGIONS[6],
    region(
        "firmware",
        FLASH,
        0x10000,
        0x600000,
        RegionOwner::FirmwareImage,
        RegionRetention::ReplaceWithFirmware,
        RegionRole::FirmwareImage,
    ),
    region(
        "mls_alice",
        FLASH,
        0x600000,
        0xa00000,
        RegionOwner::Platform,
        RegionRetention::PreserveAcrossFirmwareUpdate,
        RegionRole::GroupState,
    ),
    region(
        "mls_bob",
        FLASH,
        0xa00000,
        0xe00000,
        RegionOwner::Platform,
        RegionRetention::PreserveAcrossFirmwareUpdate,
        RegionRole::GroupState,
    ),
    ESP_16_MIB_REGIONS[8],
    ESP_16_MIB_REGIONS[9],
    ESP_16_MIB_REGIONS[10],
    ESP_16_MIB_REGIONS[11],
];

/// E290 with two independent MLS test peers. This is not a release layout.
pub const HELTEC_E290_MLS_QUALIFICATION: MemoryProfile = MemoryProfile {
    id: MemoryProfileId("heltec-e290-mls-qualification"),
    regions: &REGIONS,
    firmware: firmware_placement(0x10000, 0x600000, 0x600000),
    ..HELTEC_E290
};

const PARTITIONS: [EspPartitionBinding; 12] = [
    ESP_16_MIB_PARTITIONS[0],
    ESP_16_MIB_PARTITIONS[1],
    ESP_16_MIB_PARTITIONS[2],
    ESP_16_MIB_PARTITIONS[3],
    ESP_16_MIB_PARTITIONS[4],
    ESP_16_MIB_PARTITIONS[5],
    esp_binding(
        "mls_alice",
        "mls_alice",
        EspPartitionKind::Custom {
            partition_type: 0x47,
            subtype: 0,
        },
    ),
    esp_binding(
        "mls_bob",
        "mls_bob",
        EspPartitionKind::Custom {
            partition_type: 0x47,
            subtype: 1,
        },
    ),
    ESP_16_MIB_PARTITIONS[6],
    ESP_16_MIB_PARTITIONS[7],
    ESP_16_MIB_PARTITIONS[8],
    ESP_16_MIB_PARTITIONS[9],
];

pub const HELTEC_E290_MLS_PARTITION_TABLE: EspPartitionTable = EspPartitionTable {
    profiles: &[HELTEC_E290_MLS_QUALIFICATION.id],
    partitions: &PARTITIONS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;

    #[test]
    fn qualification_layout_preserves_radio_state_and_matches_csv() {
        let profile = &HELTEC_E290_MLS_QUALIFICATION;
        assert_eq!(profile.validate(), Ok(()));
        assert_eq!(HELTEC_E290_MLS_PARTITION_TABLE.validate(profile), Ok(()));
        for region in HELTEC_E290
            .regions
            .iter()
            .filter(|region| region.retention == RegionRetention::PreserveAcrossFirmwareUpdate)
        {
            assert_eq!(profile.region(region.id), Some(region));
        }
        let mut generated = String::new();
        HELTEC_E290_MLS_PARTITION_TABLE
            .write_csv(profile, &mut generated)
            .unwrap();
        let csv = include_str!("../../../../embedded/esp32/partitions-hopspot-16mb-mls.csv");
        let data: String = csv
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| std::format!("{line}\n"))
            .collect();
        assert_eq!(data, generated);
        assert!(
            crate::memory_profile(profile.id).is_none(),
            "qualification is not a shipping profile"
        );
    }
}
