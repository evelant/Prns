use super::*;
pub(super) const CATALOG: TableDefinition<u8, &[u8]> =
    TableDefinition::new("prns_group_enrollment_v1");
impl<'db> Enrollment<'db> {
    pub(super) fn read(&self) -> Result<Catalog, EnrollmentError> {
        let tx = self.database.begin_read().map_err(db_error)?;
        let table = tx.open_table(CATALOG).map_err(db_error)?;
        let bytes = table
            .get(0)
            .map_err(db_error)?
            .ok_or(EnrollmentError::Corrupt)?;
        if bytes.value().len() > MAX_CATALOG_BYTES {
            return Err(EnrollmentError::Capacity);
        }
        require_version(bytes.value())?;
        let catalog: Catalog =
            serde_json::from_slice(bytes.value()).map_err(|_| EnrollmentError::Corrupt)?;
        if catalog.version != 2 {
            return Err(EnrollmentError::UnsupportedVersion);
        }
        if catalog.local != self.local.into_bytes()
            || catalog.entries.len() > MAX_GROUPS
            || catalog.invites.len() > MAX_INVITATIONS
            || catalog.requests.len() > MAX_REQUESTS
        {
            return Err(EnrollmentError::Corrupt);
        }
        Ok(catalog)
    }
    pub(super) fn write(&self, catalog: &Catalog) -> Result<(), EnrollmentError> {
        let bytes = Zeroizing::new(encode(catalog)?);
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(EnrollmentError::Capacity);
        }
        let mut tx = self.database.begin_write().map_err(db_error)?;
        tx.set_durability(redb::Durability::Immediate)
            .map_err(db_error)?;
        {
            let mut table = tx.open_table(CATALOG).map_err(db_error)?;
            table.insert(0, bytes.as_slice()).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)
    }
    pub(super) fn find(catalog: &Catalog, group: [u8; 32]) -> Result<&Entry, EnrollmentError> {
        catalog
            .entries
            .iter()
            .find(|e| e.group == group)
            .ok_or(EnrollmentError::Missing)
    }
    pub(super) fn replay(
        catalog: &Catalog,
        id: [u8; 32],
        fingerprint: [u8; 32],
    ) -> Result<Option<[u8; 32]>, EnrollmentError> {
        if id == [0; 32] {
            return Err(EnrollmentError::Invalid);
        }
        match catalog.requests.iter().find(|r| r.id == id) {
            Some(r) if r.fingerprint == fingerprint => Ok(Some(r.result)),
            Some(_) => Err(EnrollmentError::RequestConflict),
            None => Ok(None),
        }
    }
    pub(super) fn remember(
        catalog: &mut Catalog,
        id: [u8; 32],
        fingerprint: [u8; 32],
        result: [u8; 32],
    ) -> Result<(), EnrollmentError> {
        if catalog.requests.len() >= MAX_REQUESTS {
            return Err(EnrollmentError::Capacity);
        }
        catalog.requests.push(Request {
            id,
            fingerprint,
            result,
        });
        Ok(())
    }
    pub(super) fn entry(
        &self,
        group: [u8; 32],
        title: &str,
        owner: Option<Member>,
        now: u64,
    ) -> Result<Entry, EnrollmentError> {
        let identity =
            LocalIdentity::generate(random()?, 1).map_err(|_| EnrollmentError::Entropy)?;
        let (binding, secret) = identity.into_parts();
        let local = Member::new(self.local, binding);
        Ok(Entry {
            group,
            title: title.to_owned(),
            owner: owner.unwrap_or_else(|| local.clone()),
            local,
            secret: secret.as_ref().to_vec(),
            namespace: random()?,
            incarnation: random()?,
            created: now,
            generation: 0,
            invitation: None,
        })
    }
    pub(super) fn initialize(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
    ) -> Result<(), EnrollmentError> {
        let ns = StoreNamespace(entry.namespace);
        let inc = StoreIncarnation::new(entry.incarnation).map_err(|_| EnrollmentError::Corrupt)?;
        match owner.open(ns, inc, profile::native_limits()) {
            Ok(_) => Ok(()),
            Err(NativeError::MissingNamespace) => {
                owner.initialize(ns, inc, profile::native_limits())?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
    pub(super) fn adapter<T>(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
        roster: Option<&[Member]>,
        action: impl FnOnce(&Adapter<NativeBackend<'_>>, Snapshot) -> Result<T, EnrollmentError>,
    ) -> Result<T, EnrollmentError> {
        self.owned_adapter(owner, entry, roster, |adapter, snapshot| {
            action(&adapter, snapshot)
        })
    }
    pub(super) fn owned_adapter<T>(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
        roster: Option<&[Member]>,
        action: impl FnOnce(Adapter<NativeBackend<'_>>, Snapshot) -> Result<T, EnrollmentError>,
    ) -> Result<T, EnrollmentError> {
        self.owned_adapter_class(owner, entry, roster, AdmissionClass::Ordinary, action)
    }
    pub(super) fn management_adapter<T>(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
        roster: Option<&[Member]>,
        action: impl FnOnce(&Adapter<NativeBackend<'_>>, Snapshot) -> Result<T, EnrollmentError>,
    ) -> Result<T, EnrollmentError> {
        self.owned_adapter_class(
            owner,
            entry,
            roster,
            AdmissionClass::Management,
            |adapter, snapshot| action(&adapter, snapshot),
        )
    }
    fn owned_adapter_class<T>(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
        roster: Option<&[Member]>,
        class: AdmissionClass,
        action: impl FnOnce(Adapter<NativeBackend<'_>>, Snapshot) -> Result<T, EnrollmentError>,
    ) -> Result<T, EnrollmentError> {
        let backend = owner.open(
            StoreNamespace(entry.namespace),
            StoreIncarnation::new(entry.incarnation).map_err(|_| EnrollmentError::Corrupt)?,
            profile::native_limits(),
        )?;
        let backend = match class {
            AdmissionClass::Ordinary => backend.reserve_capacity(profile::management_reserve())?,
            AdmissionClass::Management => backend,
        };
        let l = profile::native_limits();
        let coordinator = Coordinator::new(
            backend,
            prns_groups_store::Limits {
                max_changes: l.max_changes,
                max_key_bytes: l.max_key_bytes,
                max_value_bytes: l.max_value_bytes,
                max_staged_bytes: l.max_staged_bytes,
            },
        )
        .map_err(|_| EnrollmentError::Corrupt)?;
        let mut key = vec![0];
        key.extend_from_slice(STATE_KEY);
        let snapshot: Snapshot = match coordinator
            .read(&key)
            .map_err(|_| EnrollmentError::Corrupt)?
        {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::Corrupt)?,
            None => Snapshot::default(),
        };
        let bindings = if let Some(roster) = roster {
            roster.iter().map(Member::binding).collect()
        } else if let Some(signed) = &snapshot.current {
            signed.verify(entry.group, &entry.owner, &entry.title)?;
            signed
                .manifest
                .members
                .iter()
                .map(Member::binding)
                .collect()
        } else if entry.owner == entry.local {
            vec![entry.local.binding()]
        } else {
            vec![entry.owner.binding(), entry.local.binding()]
        };
        let policy = Policy::owner_managed(entry.owner.binding(), bindings)
            .map_err(|_| EnrollmentError::Authentication)?;
        let adapter = Adapter::new(
            coordinator,
            entry.identity()?,
            policy,
            profile::mls_limits(),
        )
        .map_err(|_| EnrollmentError::Crypto)?;
        action(adapter, snapshot)
    }
    pub(super) fn next(
        adapter: &Adapter<NativeBackend<'_>>,
        entry: &Entry,
    ) -> Result<OperationId, EnrollmentError> {
        // Leave two backend receipt slots for bounded cleanup while an older
        // service output still pins the contiguous retirement prefix.
        let pending = adapter
            .pending_operations()
            .map_err(|_| EnrollmentError::Corrupt)?;
        if pending.len()
            >= profile::mls_limits()
                .max_pending_outputs
                .saturating_sub(profile::MANAGEMENT_OUTPUT_RESERVE)
        {
            return Err(EnrollmentError::Capacity);
        }
        let (receipts, _) = Self::finalize_prefix(adapter, entry, &pending)?;
        if !Self::ordinary_receipt_available(receipts, profile::native_limits().max_receipts) {
            return Err(EnrollmentError::Capacity);
        }
        Self::next_maintenance(adapter, entry)
    }
    pub(super) fn ordinary_receipt_available(receipts: usize, max_receipts: usize) -> bool {
        receipts < max_receipts.saturating_sub(2)
    }
    pub(super) fn next_maintenance(
        adapter: &Adapter<NativeBackend<'_>>,
        entry: &Entry,
    ) -> Result<OperationId, EnrollmentError> {
        let head = adapter
            .coordinator()
            .head()
            .map_err(|_| EnrollmentError::Corrupt)?;
        Ok(OperationTicket::new(
            StoreIncarnation::new(entry.incarnation).map_err(|_| EnrollmentError::Corrupt)?,
            head.0.checked_add(1).ok_or(EnrollmentError::Capacity)?,
        )
        .map_err(|_| EnrollmentError::Corrupt)?
        .operation_id())
    }
    pub(super) fn stage(
        tx: &prns_groups_mls::ApplicationTransaction<'_, NativeBackend<'_>>,
        snapshot: &Snapshot,
    ) -> Result<(), EnrollmentError> {
        let bytes = encode(snapshot)?;
        if bytes.len() > profile::native_limits().max_value_bytes {
            return Err(EnrollmentError::Capacity);
        }
        tx.put(STATE_KEY, &bytes)
            .map_err(|_| EnrollmentError::Corrupt)
    }
    pub(super) fn recover_creates(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
    ) -> Result<(), EnrollmentError> {
        for entry in &catalog.entries {
            if entry.owner.rns != self.local.into_bytes() {
                continue;
            }
            self.ensure_created(owner, entry)?;
        }
        Ok(())
    }
    pub(super) fn ensure_packages(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
    ) -> Result<(), EnrollmentError> {
        for invite in &catalog.invites {
            if invite.offer.recipient != self.local.into_bytes()
                || invite.state != InvitationState::JoinRequested
            {
                continue;
            }
            let entry = Self::find(catalog, invite.offer.group)?;
            self.initialize(owner, entry)?;
            self.adapter(owner, entry, None, |adapter, mut snapshot| {
                if snapshot.package.is_some() || snapshot.current.is_some() {
                    return Ok(());
                }
                adapter
                    .compose(
                        Self::next(adapter, entry)?,
                        Command::KeyPackage {
                            now_secs: entry.created,
                        },
                        |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                        |tx, output| {
                            snapshot.package = Some(
                                output
                                    .messages
                                    .first()
                                    .ok_or(EnrollmentError::Corrupt)?
                                    .clone(),
                            );
                            Self::stage(tx, &snapshot)
                        },
                    )
                    .map_err(composition_error)?;
                Ok(())
            })?;
        }
        Ok(())
    }
    pub(super) fn invitation_state(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
        invite: &Invite,
        now: u64,
    ) -> Result<(InvitationState, bool), EnrollmentError> {
        if matches!(
            invite.state,
            InvitationState::Cancelled | InvitationState::Expired | InvitationState::Declined
        ) {
            return Ok((invite.state, false));
        }
        if let Some(entry) = catalog
            .entries
            .iter()
            .find(|e| e.group == invite.offer.group)
        {
            self.initialize(owner, entry)?;
            let state = self.adapter(owner, entry, None, |_, snapshot| {
                if invite.offer.owner.rns == self.local.into_bytes() {
                    if snapshot
                        .added
                        .iter()
                        .any(|a| a.invitation == invite.offer.id)
                    {
                        return Ok(Some((
                            if invite.state == InvitationState::Joined {
                                InvitationState::Joined
                            } else {
                                InvitationState::JoinRequested
                            },
                            false,
                        )));
                    }
                } else if entry.invitation == Some(invite.offer.id) && snapshot.current.is_some() {
                    return Ok(Some((
                        if snapshot.ready()? {
                            InvitationState::Joined
                        } else {
                            invite.state
                        },
                        false,
                    )));
                }
                Ok(None)
            })?;
            if let Some(state) = state {
                return Ok(state);
            }
        }
        if invite.state == InvitationState::Pending && now >= invite.offer.expires {
            Ok((InvitationState::Expired, false))
        } else {
            Ok((
                invite.state,
                matches!(
                    invite.state,
                    InvitationState::Pending | InvitationState::JoinRequested
                ) && invite.deferred_join.is_some()
                    && now < invite.offer.expires,
            ))
        }
    }
}
impl EnrollmentError {
    pub fn is_invalid_input(&self) -> bool {
        matches!(
            self,
            Self::Invalid
                | Self::Authentication
                | Self::Expired
                | Self::Missing
                | Self::Conflict
                | Self::RequestConflict
                | Self::NotOwner
                | Self::NotJoined
                | Self::AlreadyJoined
                | Self::GroupClosed
                | Self::OwnerMustClose
                | Self::MemberNotFound
                | Self::StaleMembership
                | Self::UnsupportedVersion
                | Self::Retry(
                    prns_groups::retry::RetryError::InvalidToken
                        | prns_groups::retry::RetryError::Expired
                )
        )
    }
}
