/// Mandatory local limits. These reserve disjoint key budgets for every possible
/// group and retained epoch, including groups that are not currently loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_members: usize,
    pub max_groups: usize,
    pub max_wire_bytes: usize,
    pub max_plaintext_bytes: usize,
    pub max_snapshot_bytes: usize,
    pub max_key_packages: usize,
    pub max_retained_epochs: usize,
    pub max_forward_distance: u32,
    pub max_skipped_per_ratchet: usize,
    pub max_skipped_per_epoch: usize,
    pub max_device_skipped_keys: usize,
    pub max_pending_proposals: usize,
    pub max_pending_outputs: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Configuration,
    Members,
    Groups,
    WireBytes,
    PlaintextBytes,
    SnapshotBytes,
    KeyPackages,
    Proposals,
    PendingOutputs,
    Allocation,
}

impl Limits {
    pub fn validate(&self) -> Result<(), Limit> {
        let reserved = self
            .max_retained_epochs
            .checked_add(1)
            .and_then(|epochs| epochs.checked_mul(self.max_groups))
            .and_then(|trees| trees.checked_mul(self.max_skipped_per_epoch))
            .ok_or(Limit::Configuration)?;
        if self.max_members == 0
            || self.max_members >= u32::MAX as usize
            || self.max_groups == 0
            || self.max_key_packages == 0
            || self.max_wire_bytes == 0
            || self.max_plaintext_bytes == 0
            || self.max_snapshot_bytes == 0
            || self.max_pending_outputs == 0
            || self.max_pending_proposals == 0
            || self.max_forward_distance > 1024
            || self.max_plaintext_bytes > self.max_wire_bytes
            || self.max_skipped_per_ratchet > self.max_skipped_per_epoch
            || reserved > self.max_device_skipped_keys
            || self.fields().iter().any(|n| *n > u32::MAX as usize)
        {
            return Err(Limit::Configuration);
        }
        Ok(())
    }

    pub(crate) fn fields(&self) -> [usize; 13] {
        [
            self.max_members,
            self.max_groups,
            self.max_wire_bytes,
            self.max_plaintext_bytes,
            self.max_snapshot_bytes,
            self.max_key_packages,
            self.max_retained_epochs,
            self.max_forward_distance as usize,
            self.max_skipped_per_ratchet,
            self.max_skipped_per_epoch,
            self.max_device_skipped_keys,
            self.max_pending_proposals,
            self.max_pending_outputs,
        ]
    }
}
