//! Persisted native text-profile resource ceilings. These do not guarantee that
//! every independent quota can be filled simultaneously. Changing them requires
//! a migration for existing namespaces.
use prns_groups::Seconds;
use prns_groups_store_native::NativeLimits;
pub const MAX_MEMBERS: usize = 8;
pub const RETRY_POLICY: prns_groups::retry::RetryPolicy = prns_groups::retry::RetryPolicy {
    horizon_secs: 14 * 24 * 60 * 60,
    future_skew_secs: 300,
};
pub fn native_limits() -> NativeLimits {
    NativeLimits {
        max_changes: 128,
        max_key_bytes: 256,
        max_value_bytes: 262_144,
        max_staged_bytes: 2_097_152,
        max_live_objects: 8192,
        max_live_bytes: 16_777_216,
        max_receipts: 4096,
    }
}
pub fn mls_limits() -> prns_groups_mls::Limits {
    prns_groups_mls::Limits {
        max_members: MAX_MEMBERS,
        max_groups: 1,
        max_wire_bytes: 65_536,
        max_plaintext_bytes: 16_384,
        max_snapshot_bytes: 262_144,
        max_key_packages: 16,
        max_retained_epochs: 1,
        max_forward_distance: 128,
        max_skipped_per_ratchet: 128,
        max_skipped_per_epoch: 512,
        max_device_skipped_keys: 1024,
        max_pending_proposals: 16,
        max_pending_outputs: 1024,
    }
}
pub fn service_limits() -> prns_groups_service::Limits {
    prns_groups_service::Limits {
        max_records: 256,
        max_targets: MAX_MEMBERS,
        max_body_bytes: prns_group_text::MAX_ENCODED_BYTES,
        max_changes: 4096,
        max_envelopes: 4096,
        max_retained_wire_bytes: 8_388_608,
        maximum_window: Seconds(14 * 24 * 60 * 60),
        future_skew: Seconds(300),
    }
}
