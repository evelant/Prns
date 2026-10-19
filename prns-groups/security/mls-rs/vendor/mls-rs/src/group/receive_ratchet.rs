// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Copyright by contributors to this project.
// SPDX-License-Identifier: (Apache-2.0 OR MIT)

/// Local receive policy. These values are not MLS wire fields or snapshot data.
///
/// Applications must install their policy after creating, joining or loading a
/// group. Limits apply to each epoch's secret tree, across application and
/// handshake ratchets. Bound retained epochs/groups separately in the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiveRatchetLimits {
    /// Maximum skipped generations derived while processing one message.
    pub max_forward_distance: u32,
    /// Maximum retained skipped keys for one sender and key type.
    pub max_skipped_keys_per_ratchet: usize,
    /// Maximum retained skipped keys across all senders and key types in an epoch.
    pub max_skipped_keys_per_epoch: usize,
}

/// Retained skipped-key counts; does not count chain secrets or tree nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiveRatchetUsage {
    pub skipped_keys: usize,
    pub max_skipped_keys_in_ratchet: usize,
}

/// A policy failure detected before advancing a receive ratchet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiveRatchetLimitError {
    ForwardDistance { requested: u32, limit: u32 },
    PerRatchetSkippedKeys { requested: usize, limit: usize },
    PerEpochSkippedKeys { requested: usize, limit: usize },
    GenerationExhausted,
    CounterOverflow,
}

impl ReceiveRatchetLimits {
    /// Validate forward work without advancing state. Historical generations have
    /// zero forward distance; availability still depends on retained keys.
    pub fn forward_distance(
        self,
        next_generation: u32,
        requested_generation: u32,
    ) -> Result<u32, ReceiveRatchetLimitError> {
        requested_generation
            .checked_add(1)
            .ok_or(ReceiveRatchetLimitError::GenerationExhausted)?;
        let distance = requested_generation.saturating_sub(next_generation);
        if distance > self.max_forward_distance {
            return Err(ReceiveRatchetLimitError::ForwardDistance {
                requested: distance,
                limit: self.max_forward_distance,
            });
        }
        Ok(distance)
    }
}

impl ReceiveRatchetUsage {
    pub(crate) fn validate(
        self,
        limits: ReceiveRatchetLimits,
    ) -> Result<(), ReceiveRatchetLimitError> {
        if self.max_skipped_keys_in_ratchet > limits.max_skipped_keys_per_ratchet {
            return Err(ReceiveRatchetLimitError::PerRatchetSkippedKeys {
                requested: self.max_skipped_keys_in_ratchet,
                limit: limits.max_skipped_keys_per_ratchet,
            });
        }
        if self.skipped_keys > limits.max_skipped_keys_per_epoch {
            return Err(ReceiveRatchetLimitError::PerEpochSkippedKeys {
                requested: self.skipped_keys,
                limit: limits.max_skipped_keys_per_epoch,
            });
        }
        Ok(())
    }
}

#[cfg(feature = "private_message")]
impl<C> super::Group<C>
where
    C: crate::client_config::ClientConfig + Clone,
{
    /// Install a local policy after creation/join/load, validating current and
    /// already loaded prior epochs. An error leaves the previous policy unchanged.
    ///
    /// Snapshots intentionally do not contain this policy. Lazy-loaded prior
    /// epochs are validated again before receiving a message or returning usage.
    /// Decoding a stored snapshot precedes this check; bound stored record bytes
    /// before calling the upstream decoder. This is not a fallible allocator API.
    pub fn set_receive_ratchet_limits(
        &mut self,
        limits: ReceiveRatchetLimits,
    ) -> Result<(), crate::client::MlsError> {
        self.receive_ratchet_usage()?
            .validate(limits)
            .map_err(crate::client::MlsError::ReceiveRatchetLimitExceeded)?;
        #[cfg(feature = "prior_epoch")]
        self.state_repo
            .validate_pending_receive_ratchet_limits(limits)?;
        self.receive_ratchet_limits = Some(limits);
        Ok(())
    }

    /// Installed local policy, or `None` for upstream-compatible unrestricted
    /// skipped-key retention. PRNS adapters must not process in the latter state.
    pub fn receive_ratchet_limits(&self) -> Option<ReceiveRatchetLimits> {
        self.receive_ratchet_limits
    }

    pub fn receive_ratchet_usage(&self) -> Result<ReceiveRatchetUsage, crate::client::MlsError> {
        self.epoch_secrets.secret_tree.receive_ratchet_usage()
    }

    /// Load and validate a retained epoch using the installed local policy.
    /// Returns `None` when the storage provider no longer retains that epoch.
    #[cfg(feature = "prior_epoch")]
    #[cfg_attr(not(mls_build_async), maybe_async::must_be_sync)]
    pub async fn prior_epoch_receive_ratchet_usage(
        &mut self,
        epoch_id: u64,
    ) -> Result<Option<ReceiveRatchetUsage>, crate::client::MlsError> {
        let limits = self.receive_ratchet_limits;
        let Some(epoch) = self.state_repo.get_epoch_mut(epoch_id).await? else {
            return Ok(None);
        };
        let usage = epoch.secrets.secret_tree.receive_ratchet_usage()?;
        if let Some(limits) = limits {
            usage
                .validate(limits)
                .map_err(crate::client::MlsError::ReceiveRatchetLimitExceeded)?;
        }
        Ok(Some(usage))
    }
}
