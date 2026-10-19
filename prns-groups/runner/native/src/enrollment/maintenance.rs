use super::*;
use prns_groups::retry::{RequestToken, RetryPolicy};

pub const RETRY_HORIZON_SECS: u64 = 14 * 24 * 60 * 60;
pub const MAX_UNACCEPTED_INCOMING: usize = 4;
pub const MAX_INCOMING_PER_PEER: usize = 2;
pub const MAX_LOCAL_INVITATIONS: usize = MAX_INVITATIONS - MAX_UNACCEPTED_INCOMING;
pub(super) const RETRY_POLICY: RetryPolicy = RetryPolicy {
    horizon_secs: RETRY_HORIZON_SECS,
    future_skew_secs: 300,
};

/// Bounded catalog cleanup. These are logical record counts, not disk reclamation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Maintenance {
    pub retired_requests: usize,
    pub retired_invitations: usize,
    pub retired_outputs: usize,
    pub request_retired_before: u64,
}

impl<'db> Enrollment<'db> {
    /// Atomically retire API receipts and safe terminal invitation records with
    /// their replay fences. Group namespaces and MLS Welcome/control bytes stay.
    pub fn maintain(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        now: u64,
    ) -> Result<Maintenance, EnrollmentError> {
        if now < 3600 {
            return Err(EnrollmentError::Invalid);
        }
        let mut catalog = self.read()?;
        let before = Zeroizing::new(encode(&catalog)?);
        let mut result = self.maintenance_plan(owner, &mut catalog, now)?;
        let after = Zeroizing::new(encode(&catalog)?);
        if before.as_slice() != after.as_slice() {
            self.write(&catalog)?;
        }
        for entry in &catalog.entries {
            self.initialize(owner, entry)?;
            result.retired_outputs += self.retire_outputs(owner, entry)?;
        }
        Ok(result)
    }

    pub(super) fn maintenance_plan(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &mut Catalog,
        now: u64,
    ) -> Result<Maintenance, EnrollmentError> {
        let now = now.max(catalog.invitation_time_floor);
        let upgraded = *catalog.legacy_upgrade_at.get_or_insert(now);
        let legacy_grace_finished = upgraded
            .checked_add(RETRY_HORIZON_SECS)
            .is_some_and(|end| now >= end);
        let floor = RETRY_POLICY.retirement_floor(now, catalog.request_retired_before);
        let requests_before = catalog.requests.len();
        catalog
            .requests
            .retain(|request| match RequestToken::parse(request.id) {
                Ok(token) => token.issued_secs() > floor,
                Err(_) => !legacy_grace_finished,
            });
        let retired_requests = requests_before - catalog.requests.len();
        // The fence and deletion publish in the same redb catalog transaction.
        if retired_requests != 0 {
            catalog.request_retired_before = floor;
        }

        let mut committed_adds = Vec::new();
        for entry in &catalog.entries {
            if entry.owner.rns != self.local.into_bytes() {
                continue;
            }
            self.initialize(owner, entry)?;
            self.adapter(owner, entry, None, |_, snapshot| {
                committed_adds.extend(snapshot.added.iter().map(|added| added.invitation));
                Ok(())
            })?;
        }
        let invitations_before = catalog.invites.len();
        catalog.invites.retain_mut(|invite| {
            let outgoing = invite.offer.owner.rns == self.local.into_bytes();
            let added = committed_adds.contains(&invite.offer.id);
            // A consumed offer is still needed until the recipient confirms
            // its durable join. Its original seven-day deadline cannot undo Add.
            if (outgoing && added && invite.state != InvitationState::Joined)
                || (!outgoing && invite.state == InvitationState::JoinRequested)
            {
                return true;
            }
            if matches!(
                invite.state,
                InvitationState::Joined | InvitationState::Expired
            ) {
                let terminal = *invite.terminal_at.get_or_insert(now);
                return !terminal
                    .checked_add(RETRY_HORIZON_SECS)
                    .is_some_and(|end| now >= end);
            }
            // Never-accepted offers and declines no longer authorize anything
            // after expiry. The clock fence prevents redisplay after rollback.
            now < invite.offer.expires
        });
        let retired_invitations = invitations_before - catalog.invites.len();
        if retired_invitations != 0 {
            catalog.invitation_time_floor = now;
            catalog.request_retired_before = floor;
        }
        Ok(Maintenance {
            retired_requests,
            retired_invitations,
            retired_outputs: 0,
            request_retired_before: catalog.request_retired_before,
        })
    }

    // Native managed namespaces are consumed only by enrollment and the group
    // service. Service operations have an application fact under b'o' + id;
    // enrollment publishes each replay obligation in Snapshot in the same MLS
    // transaction. Verify that proof as well as the operation's event before
    // deleting its redundant output. Backend operation receipts remain intact.
    fn retire_outputs(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
    ) -> Result<usize, EnrollmentError> {
        self.adapter(owner, entry, None, |adapter, snapshot| {
            let mut completed = Vec::new();
            let pending = adapter
                .pending_operations()
                .map_err(|_| EnrollmentError::Crypto)?;
            let (receipts, oldest) = Self::finalize_prefix(adapter, entry, &pending)?;
            for operation in pending.iter().copied() {
                let mut fact = [0; 33];
                fact[0] = b'o';
                fact[1..].copy_from_slice(&operation.0);
                if adapter
                    .application_read(&fact)
                    .map_err(|_| EnrollmentError::Crypto)?
                    .is_some()
                {
                    continue;
                }
                let output = adapter
                    .retry(operation)
                    .map_err(|_| EnrollmentError::Crypto)?
                    .ok_or(EnrollmentError::Corrupt)?;
                if Self::output_replay_is_preserved(entry, &snapshot, &output)? {
                    completed.push(operation);
                    if completed.len() == 16 {
                        break;
                    }
                }
            }
            if !completed.is_empty() {
                let max_receipts = profile::native_limits().max_receipts;
                if !Self::cleanup_receipt_available(
                    receipts,
                    max_receipts,
                    oldest.is_some_and(|operation| completed.contains(&operation)),
                ) {
                    return Ok(0);
                }
                adapter
                    .maintain(Self::next_maintenance(adapter, entry)?, &completed, |_| {
                        Ok::<_, EnrollmentError>(())
                    })
                    .map_err(|_| EnrollmentError::Crypto)?;
                let remaining = adapter
                    .pending_operations()
                    .map_err(|_| EnrollmentError::Crypto)?;
                Self::finalize_prefix(adapter, entry, &remaining)?;
            }
            Ok(completed.len())
        })
    }

    pub(super) fn cleanup_receipt_available(
        receipts: usize,
        max_receipts: usize,
        retires_oldest: bool,
    ) -> bool {
        receipts < max_receipts && (receipts < max_receipts.saturating_sub(1) || retires_oldest)
    }

    // Only the native owner may attest this prefix: all enrollment and service
    // recovery obligations in these namespaces retain an MLS output until their
    // own durable proof permits deletion. Finalization itself consumes no slot.
    pub(super) fn finalize_prefix(
        adapter: &Adapter<NativeBackend<'_>>,
        entry: &Entry,
        pending: &[OperationId],
    ) -> Result<(usize, Option<OperationId>), EnrollmentError> {
        let head = adapter
            .coordinator()
            .head()
            .map_err(|_| EnrollmentError::Corrupt)?
            .0;
        let incarnation =
            StoreIncarnation::new(entry.incarnation).map_err(|_| EnrollmentError::Corrupt)?;
        let mut oldest: Option<(u64, OperationId)> = None;
        for operation in pending {
            let ticket = OperationTicket::from_operation_id(*operation)
                .map_err(|_| EnrollmentError::Corrupt)?;
            if ticket.incarnation() != incarnation || ticket.sequence() > head {
                return Err(EnrollmentError::Corrupt);
            }
            if oldest.is_none_or(|(sequence, _)| ticket.sequence() < sequence) {
                oldest = Some((ticket.sequence(), *operation));
            }
        }
        let through = oldest.map_or(head, |(sequence, _)| sequence - 1);
        if through != 0 {
            let ticket =
                OperationTicket::new(incarnation, through).map_err(|_| EnrollmentError::Corrupt)?;
            match adapter
                .coordinator()
                .maintain(prns_groups_store_native::NativeRequest::FinalizeThrough(
                    ticket,
                ))
                .map_err(|_| EnrollmentError::Corrupt)?
            {
                prns_groups_store::MaintenanceOutcome::Complete(_) => {}
                _ => return Err(EnrollmentError::Corrupt),
            }
        }
        Ok((
            usize::try_from(head - through).map_err(|_| EnrollmentError::Capacity)?,
            oldest.map(|(_, operation)| operation),
        ))
    }

    pub(super) fn output_replay_is_preserved(
        entry: &Entry,
        snapshot: &Snapshot,
        output: &prns_groups_mls::Output,
    ) -> Result<bool, EnrollmentError> {
        use prns_groups_mls::Event;
        if output.event == Event::KeyPackage {
            return Ok(output.messages.len() == 1
                && (snapshot.package.as_ref() == output.messages.first()
                    || snapshot.current.is_some()));
        }
        let (Some(current), Some(context)) = (&snapshot.current, output.context) else {
            return Ok(false);
        };
        if context.group.0 != entry.group || context.epoch > current.manifest.epoch {
            return Err(EnrollmentError::Corrupt);
        }
        match output.event {
            Event::Created | Event::Joined => Ok(output.messages.is_empty()),
            Event::Control => {
                let Some(control) = snapshot
                    .controls
                    .iter()
                    .find(|c| c.signed.manifest.epoch == context.epoch)
                else {
                    return Ok(false);
                };
                control
                    .signed
                    .verify(entry.group, &entry.owner, &entry.title)?;
                match output.messages.as_slice() {
                    [] => Ok(true),
                    [commit, welcome] => Ok(commit == &control.commit
                        && snapshot.added.iter().any(|added| {
                            added.signed == control.signed && &added.welcome == welcome
                        })),
                    _ => Ok(false),
                }
            }
            // Never infer ownership of service output, including a missing or
            // damaged application fact, merely from the absence of that fact.
            _ => Ok(false),
        }
    }

    pub(super) fn admission(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        fingerprint: [u8; 32],
        now: u64,
    ) -> Result<(Catalog, Option<[u8; 32]>), EnrollmentError> {
        let catalog = self.read()?;
        if let Some(result) = Self::replay(&catalog, request, fingerprint)? {
            return Ok((catalog, Some(result)));
        }
        self.maintain(owner, now)?;
        let catalog = self.read()?;
        RETRY_POLICY
            .validate_new(
                request,
                now.max(catalog.invitation_time_floor),
                catalog.request_retired_before,
            )
            .map_err(EnrollmentError::Retry)?;
        Ok((catalog, None))
    }

    pub(super) fn local_invitation_count(&self, catalog: &Catalog) -> usize {
        catalog
            .invites
            .iter()
            .filter(|invite| Self::is_local_work(self.local, invite))
            .count()
    }
    fn is_local_work(local: IdentityHash, invite: &Invite) -> bool {
        invite.offer.owner.rns == local.into_bytes()
            || matches!(
                invite.state,
                InvitationState::JoinRequested | InvitationState::Joined | InvitationState::Expired
            )
    }
    pub(super) fn admit_incoming(
        &self,
        catalog: &Catalog,
        peer: IdentityHash,
    ) -> Result<(), EnrollmentError> {
        let incoming = catalog
            .invites
            .iter()
            .filter(|invite| !Self::is_local_work(self.local, invite))
            .collect::<Vec<_>>();
        if catalog.invites.len() >= MAX_INVITATIONS
            || incoming.len() >= MAX_UNACCEPTED_INCOMING
            || incoming
                .iter()
                .filter(|invite| invite.offer.owner.rns == peer.into_bytes())
                .count()
                >= MAX_INCOMING_PER_PEER
        {
            return Err(EnrollmentError::Capacity);
        }
        Ok(())
    }

    /// Find the original committed Add independently of prunable catalog UI
    /// records. Signed, contiguous owner controls identify its unique recipient.
    pub(super) fn retained_add(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
        invitation: [u8; 32],
        requester: IdentityHash,
    ) -> Result<Option<(Entry, Snapshot, Added)>, EnrollmentError> {
        for entry in &catalog.entries {
            if entry.owner.rns != self.local.into_bytes() {
                continue;
            }
            let found = self.adapter(owner, entry, None, |_, snapshot| {
                let Some(added) = snapshot
                    .added
                    .iter()
                    .find(|a| a.invitation == invitation)
                    .cloned()
                else {
                    return Ok(None);
                };
                let prior = if added.signed.manifest.revision == 1 {
                    vec![entry.owner.clone()]
                } else {
                    snapshot
                        .controls
                        .iter()
                        .find(|control| {
                            control.signed.head().ok() == Some(added.signed.manifest.parent)
                        })
                        .ok_or(EnrollmentError::Corrupt)?
                        .signed
                        .manifest
                        .members
                        .clone()
                };
                let mut new_members = added
                    .signed
                    .manifest
                    .members
                    .iter()
                    .filter(|member| !prior.contains(member));
                let recipient = new_members.next().ok_or(EnrollmentError::Corrupt)?;
                if new_members.next().is_some() || recipient.rns != requester.into_bytes() {
                    return Err(EnrollmentError::Authentication);
                }
                Ok(Some((entry.clone(), snapshot, added)))
            })?;
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }
}
