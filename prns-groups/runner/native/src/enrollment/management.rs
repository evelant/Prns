use super::*;

// A full wire control plus its independently signed terminal notice and replay
// record. Admission reserves this space for each currently removable member.
const TERMINAL_BYTES: usize = 8192;
const REMOVAL_RESERVE: usize = MAX_FRAME_BYTES + TERMINAL_BYTES + 1024;

impl<'db> Enrollment<'db> {
    pub fn cancel_invitation(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        invitation: [u8; 32],
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&("cancel", invitation))?);
        let (mut catalog, replay) = self.admission(owner, request, fingerprint, now)?;
        if replay.is_some() {
            return Ok(());
        }
        let index = catalog
            .invites
            .iter()
            .position(|i| i.offer.id == invitation)
            .ok_or(EnrollmentError::Missing)?;
        let invite = &catalog.invites[index];
        if invite.offer.owner.rns != self.local.into_bytes() {
            return Err(EnrollmentError::NotOwner);
        }
        let entry = Self::find(&catalog, invite.offer.group)?;
        self.adapter(owner, entry, None, |_, snapshot| {
            if snapshot.added.iter().any(|a| a.invitation == invitation) {
                return Err(EnrollmentError::AlreadyJoined);
            }
            Self::require_active(&snapshot)?;
            Ok(())
        })?;
        if invite.state != InvitationState::Pending && invite.state != InvitationState::Cancelled {
            return Err(EnrollmentError::Conflict);
        }
        let invite = &mut catalog.invites[index];
        invite.state = InvitationState::Cancelled;
        invite.acknowledged = false;
        invite.terminal_at = Some(now.max(catalog.invitation_time_floor));
        invite.deferred_join = None;
        Self::remember(&mut catalog, request, fingerprint, invitation)?;
        self.write(&catalog)
    }

    pub fn dismiss_invitation(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        invitation: [u8; 32],
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&("dismiss", invitation))?);
        let (mut catalog, replay) = self.admission(owner, request, fingerprint, now)?;
        if replay.is_some() {
            return Ok(());
        }
        let index = catalog
            .invites
            .iter()
            .position(|i| i.offer.id == invitation)
            .ok_or(EnrollmentError::Missing)?;
        let (state, _) = self.invitation_state(owner, &catalog, &catalog.invites[index], now)?;
        if !Self::dismissible(state) {
            return Err(EnrollmentError::Conflict);
        }
        catalog.invites[index].dismissed = true;
        Self::remember(&mut catalog, request, fingerprint, invitation)?;
        self.write(&catalog)
    }
    pub(super) fn dismissible(state: InvitationState) -> bool {
        matches!(
            state,
            InvitationState::Joined
                | InvitationState::Declined
                | InvitationState::Expired
                | InvitationState::Cancelled
        )
    }
    pub(super) fn require_active(snapshot: &Snapshot) -> Result<(), EnrollmentError> {
        match snapshot.lifecycle {
            GroupLifecycle::Active => Ok(()),
            GroupLifecycle::Closed => Err(EnrollmentError::GroupClosed),
            _ => Err(EnrollmentError::NotJoined),
        }
    }
    pub(super) fn terminal(
        entry: &Entry,
        signed: &Signed,
        kind: TerminalKind,
    ) -> Result<Terminal, EnrollmentError> {
        let statement = TerminalStatement {
            version: 2,
            group: entry.group,
            title: entry.title.clone(),
            owner: entry.owner.clone(),
            head: signed.head()?,
            revision: signed.manifest.revision,
            kind,
        };
        let signature = entry
            .identity()?
            .sign_enrollment(&encode(&statement)?)
            .map_err(|_| EnrollmentError::Crypto)?;
        let terminal = Terminal {
            statement,
            signature,
        };
        if encode(&terminal)?.len() > TERMINAL_BYTES {
            return Err(EnrollmentError::Capacity);
        }
        Ok(terminal)
    }
    pub(super) fn stage_reserved(
        tx: &prns_groups_mls::ApplicationTransaction<'_, NativeBackend<'_>>,
        snapshot: &Snapshot,
    ) -> Result<(), EnrollmentError> {
        let bytes = encode(snapshot)?;
        let remaining = if snapshot.lifecycle == GroupLifecycle::Active {
            snapshot
                .current
                .as_ref()
                .map_or(0, |s| s.manifest.members.len().saturating_sub(1))
        } else {
            0
        };
        let reserve = remaining
            .checked_mul(REMOVAL_RESERVE)
            .and_then(|n| n.checked_add(TERMINAL_BYTES + 1024))
            .ok_or(EnrollmentError::Capacity)?;
        if bytes
            .len()
            .checked_add(reserve)
            .is_none_or(|n| n > profile::native_limits().max_value_bytes)
        {
            return Err(EnrollmentError::Capacity);
        }
        Self::stage(tx, snapshot)
    }
    fn management_replay(
        snapshot: &Snapshot,
        request: [u8; 32],
        fingerprint: [u8; 32],
    ) -> Result<bool, EnrollmentError> {
        match snapshot.management.iter().find(|r| r.id == request) {
            Some(r) if r.fingerprint == fingerprint => Ok(true),
            Some(_) => Err(EnrollmentError::RequestConflict),
            None => Ok(false),
        }
    }
    pub fn remove_member(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        group: GroupId,
        member: Endpoint,
        expected_head: [u8; 32],
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&(
            "remove",
            group.0,
            member.id.0,
            member.incarnation.get(),
            expected_head,
        ))?);
        let (mut catalog, replay) = self.management_admission(owner, request, fingerprint, now)?;
        if replay.is_some() {
            return Ok(());
        }
        if catalog.requests.len() >= MAX_REQUESTS {
            return Err(EnrollmentError::Capacity);
        }
        let entry = Self::find(&catalog, group.0)?.clone();
        if entry.owner.rns != self.local.into_bytes() {
            return Err(EnrollmentError::NotOwner);
        }
        let already = self.adapter(owner, &entry, None, |_, snapshot| {
            Self::management_replay(&snapshot, request, fingerprint)
        })?;
        if !already {
            self.remove_exact(
                owner,
                &entry,
                member,
                Some(expected_head),
                Some((request, fingerprint)),
            )?;
        }
        Self::remember(&mut catalog, request, fingerprint, group.0)?;
        self.write(&catalog)
    }
    fn remove_exact(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
        target: Endpoint,
        expected_head: Option<[u8; 32]>,
        request: Option<([u8; 32], [u8; 32])>,
    ) -> Result<Terminal, EnrollmentError> {
        let (prior, member, roster) = self.adapter(owner, entry, None, |_, snapshot| {
            Self::require_active(&snapshot)?;
            let prior = snapshot.current.ok_or(EnrollmentError::NotJoined)?;
            if expected_head.is_some_and(|head| prior.head().ok() != Some(head)) {
                return Err(EnrollmentError::StaleMembership);
            }
            let member = prior
                .manifest
                .members
                .iter()
                .find(|m| m.endpoint().ok() == Some(target))
                .cloned()
                .ok_or(EnrollmentError::MemberNotFound)?;
            if member == entry.owner {
                return Err(EnrollmentError::OwnerMustClose);
            }
            let roster = prior
                .manifest
                .members
                .iter()
                .filter(|m| *m != &member)
                .cloned()
                .collect::<Vec<_>>();
            Ok((prior, member, roster))
        })?;
        let expected = roster.iter().map(Member::binding).collect::<Vec<_>>();
        self.management_adapter(
            owner,
            entry,
            Some(&prior.manifest.members),
            |adapter, mut snapshot| {
                let mut result = None;
                adapter
                    .compose_with_roster(
                        Self::next_maintenance(adapter, entry)?,
                        Command::RemoveMember {
                            group: GroupId(entry.group),
                            member: member.binding(),
                        },
                        Some(&expected),
                        |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                        |tx, output| {
                            if output.event != prns_groups_mls::Event::Control
                                || output.messages.len() != 1
                            {
                                return Err(EnrollmentError::Corrupt);
                            }
                            let commit = output.messages[0].clone();
                            let manifest = Manifest {
                                version: 2,
                                group: entry.group,
                                owner: entry.owner.rns,
                                title: entry.title.clone(),
                                parent: prior.head()?,
                                revision: prior
                                    .manifest
                                    .revision
                                    .checked_add(1)
                                    .ok_or(EnrollmentError::Capacity)?,
                                epoch: output.context.ok_or(EnrollmentError::Corrupt)?.epoch,
                                commit_digest: hash(&commit),
                                kind: ControlKind::Remove(member.clone()),
                                members: roster.clone(),
                            };
                            let signed = Signed {
                                signature: entry
                                    .identity()?
                                    .sign_enrollment(&encode(&manifest)?)
                                    .map_err(|_| EnrollmentError::Crypto)?,
                                manifest,
                            };
                            Self::transition(entry, &prior, &signed, &commit)?;
                            let terminal = Self::terminal(
                                entry,
                                &signed,
                                TerminalKind::Removed(member.clone()),
                            )?;
                            let control = Control {
                                signed: signed.clone(),
                                commit,
                            };
                            frame(&Reply::Control(Some(control.clone())))?;
                            snapshot.current = Some(signed);
                            snapshot.controls.push(control);
                            snapshot.terminals.push(terminal.clone());
                            if let Some((id, fingerprint)) = request {
                                snapshot.management.push(Request {
                                    id,
                                    fingerprint,
                                    result: entry.group,
                                });
                            }
                            Self::stage_reserved(tx, &snapshot)?;
                            result = Some(terminal);
                            Ok(())
                        },
                    )
                    .map_err(composition_error)?;
                result.ok_or(EnrollmentError::Corrupt)
            },
        )
    }

    pub fn leave(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        group: GroupId,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&("leave", group.0))?);
        let (mut catalog, replay) = self.management_admission(owner, request, fingerprint, now)?;
        if replay.is_some() {
            return self.recover_departures(owner, &catalog);
        }
        let entry = Self::find(&catalog, group.0)?;
        if entry.owner.rns == self.local.into_bytes() {
            return Err(EnrollmentError::OwnerMustClose);
        }
        let known_head = self.adapter(owner, entry, None, |_, snapshot| {
            if matches!(
                snapshot.lifecycle,
                GroupLifecycle::Removed | GroupLifecycle::Closed
            ) {
                return Err(EnrollmentError::NotJoined);
            }
            snapshot
                .current
                .as_ref()
                .ok_or(EnrollmentError::NotJoined)?
                .head()
        })?;
        if !catalog
            .departures
            .iter()
            .any(|d| d.group == group.0 && d.member == entry.local)
        {
            let departure = Departure {
                id: random()?,
                group: group.0,
                owner: entry.owner.clone(),
                member: entry.local.clone(),
                known_head,
            };
            catalog.departures.push(departure);
        }
        Self::remember(&mut catalog, request, fingerprint, group.0)?;
        // The durable outbox itself is the admission fence. A crash before the
        // per-namespace lifecycle update cannot resume content participation.
        self.write(&catalog)?;
        self.recover_departures(owner, &catalog)
    }
    pub(super) fn recover_departures(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
    ) -> Result<(), EnrollmentError> {
        for departure in &catalog.departures {
            let Some(entry) = catalog
                .entries
                .iter()
                .find(|e| e.group == departure.group && e.local == departure.member)
            else {
                continue;
            };
            self.management_adapter(owner, entry, None, |adapter, mut snapshot| {
                if snapshot.lifecycle != GroupLifecycle::Active {
                    return Ok(());
                }
                snapshot.lifecycle = GroupLifecycle::Leaving;
                adapter
                    .maintain(Self::next_maintenance(adapter, entry)?, &[], |tx| {
                        Self::stage(tx, &snapshot)
                    })
                    .map_err(composition_error)?;
                Ok(())
            })?;
        }
        Ok(())
    }
    pub(super) fn receive_leave(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        requester: IdentityHash,
        departure: Departure,
    ) -> Result<Vec<u8>, EnrollmentError> {
        if departure.member.rns != requester.into_bytes()
            || departure.owner.rns != self.local.into_bytes()
        {
            return Err(EnrollmentError::Authentication);
        }
        let catalog = self.read()?;
        if let Some(proof) =
            self.terminal_for(owner, &catalog, departure.group, &departure.member)?
        {
            return frame(&Reply::Terminal(proof));
        }
        let entry = Self::find(&catalog, departure.group)?;
        if entry.owner != departure.owner {
            return Err(EnrollmentError::Authentication);
        }
        self.adapter(owner, entry, None, |_, snapshot| {
            if !snapshot
                .current
                .as_ref()
                .is_some_and(|s| s.manifest.members.contains(&departure.member))
            {
                return Err(EnrollmentError::Authentication);
            }
            Ok(())
        })?;
        let proof = self.remove_exact(owner, entry, departure.member.endpoint()?, None, None)?;
        frame(&Reply::Terminal(proof))
    }
    pub fn close(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        group: GroupId,
        expected_head: [u8; 32],
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&("close", group.0, expected_head))?);
        let (mut catalog, replay) = self.management_admission(owner, request, fingerprint, now)?;
        if replay.is_some() {
            return Ok(());
        }
        if catalog.requests.len() >= MAX_REQUESTS {
            return Err(EnrollmentError::Capacity);
        }
        let entry = Self::find(&catalog, group.0)?.clone();
        if entry.owner.rns != self.local.into_bytes() {
            return Err(EnrollmentError::NotOwner);
        }
        self.management_adapter(owner, &entry, None, |adapter, mut snapshot| {
            if Self::management_replay(&snapshot, request, fingerprint)? {
                return Ok(());
            }
            Self::require_active(&snapshot)?;
            let signed = snapshot
                .current
                .as_ref()
                .ok_or(EnrollmentError::NotJoined)?;
            if signed.head()? != expected_head {
                return Err(EnrollmentError::StaleMembership);
            }
            let proof = Self::terminal(&entry, signed, TerminalKind::Closed)?;
            adapter
                .compose(
                    Self::next_maintenance(adapter, &entry)?,
                    Command::Retire { group },
                    |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                    |tx, _| {
                        snapshot.lifecycle = GroupLifecycle::Closed;
                        snapshot.terminals.push(proof);
                        snapshot.management.push(Request {
                            id: request,
                            fingerprint,
                            result: group.0,
                        });
                        Self::stage(tx, &snapshot)
                    },
                )
                .map_err(composition_error)?;
            Ok(())
        })?;
        let proof = self.adapter(owner, &entry, None, |_, snapshot| {
            snapshot
                .terminals
                .into_iter()
                .find(|p| matches!(p.statement.kind, TerminalKind::Closed))
                .ok_or(EnrollmentError::Corrupt)
        })?;
        if !catalog.closed_groups.contains(&proof) {
            catalog.closed_groups.push(proof);
        }
        Self::remember(&mut catalog, request, fingerprint, group.0)?;
        self.write(&catalog)
    }

    pub(super) fn terminal_for(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
        group: [u8; 32],
        member: &Member,
    ) -> Result<Option<Terminal>, EnrollmentError> {
        if let Some(retired) = catalog
            .retired
            .iter()
            .find(|r| r.group == group && r.closed.is_some() && r.audience.contains(&member.rns))
        {
            return Ok(retired.closed.clone());
        }
        let Some(entry) = catalog
            .entries
            .iter()
            .find(|e| e.group == group && e.owner.rns == self.local.into_bytes())
        else {
            return Ok(None);
        };
        self.adapter(owner, entry, None, |_, snapshot| {
            Ok(snapshot
                .terminals
                .iter()
                .rev()
                .find(|p| match &p.statement.kind {
                    TerminalKind::Closed => {
                        snapshot
                            .current
                            .as_ref()
                            .is_some_and(|c| c.manifest.members.contains(member))
                            || snapshot.added.iter().any(|a| a.member == *member)
                            || catalog.invites.iter().any(|i| {
                                i.offer.group == group
                                    && i.offer.recipient == member.rns
                                    && i.offer.owner == entry.owner
                            })
                    }
                    TerminalKind::Removed(target) => target == member,
                })
                .cloned())
        })
    }
    fn notices(
        entry: &Entry,
        snapshot: &Snapshot,
        catalog: &Catalog,
    ) -> Result<Vec<Notification>, EnrollmentError> {
        let mut notices = Vec::new();
        for proof in &snapshot.terminals {
            if snapshot.lifecycle == GroupLifecycle::Closed
                && !matches!(proof.statement.kind, TerminalKind::Closed)
            {
                continue;
            }
            let mut audience = match &proof.statement.kind {
                TerminalKind::Removed(member) => vec![member.rns],
                TerminalKind::Closed => {
                    let mut audience = snapshot
                        .current
                        .iter()
                        .flat_map(|c| c.manifest.members.iter().map(|m| m.rns))
                        .collect::<Vec<_>>();
                    audience.extend(snapshot.added.iter().map(|a| a.member.rns));
                    audience.extend(
                        catalog
                            .invites
                            .iter()
                            .filter(|i| i.offer.group == entry.group)
                            .map(|i| i.offer.recipient),
                    );
                    audience
                }
            };
            audience.sort();
            audience.dedup();
            for recipient in audience {
                if recipient == entry.owner.rns {
                    continue;
                }
                let notice = Notification {
                    proof: proof.clone(),
                    recipient,
                    acknowledged: false,
                };
                if !snapshot.notice_acks.contains(&Self::notice_id(&notice)?) {
                    notices.push(notice);
                }
            }
        }
        Ok(notices)
    }
    pub(super) fn pending_notices(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        catalog: &Catalog,
    ) -> Result<Vec<Notification>, EnrollmentError> {
        let mut notices = catalog
            .notifications
            .iter()
            .filter(|n| !n.acknowledged)
            .cloned()
            .collect::<Vec<_>>();
        for entry in &catalog.entries {
            if entry.owner.rns == self.local.into_bytes() {
                notices.extend(self.adapter(owner, entry, None, |_, snapshot| {
                    Self::notices(entry, &snapshot, catalog)
                })?);
            }
        }
        Ok(notices)
    }
    pub(super) fn acknowledge_notice(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        id: [u8; 32],
    ) -> Result<(), EnrollmentError> {
        let mut catalog = self.read()?;
        if let Some(notice) = catalog
            .notifications
            .iter_mut()
            .find(|n| Self::notice_id(n).ok() == Some(id))
        {
            notice.acknowledged = true;
            return self.write(&catalog);
        }
        for entry in &catalog.entries {
            if entry.owner.rns != self.local.into_bytes() {
                continue;
            }
            let found = self.adapter(owner, entry, None, |adapter, mut snapshot| {
                if snapshot.notice_acks.contains(&id) {
                    return Ok(true);
                }
                if !Self::notices(entry, &snapshot, &catalog)?
                    .iter()
                    .any(|notice| Self::notice_id(notice).ok() == Some(id))
                {
                    return Ok(false);
                }
                snapshot.notice_acks.push(id);
                adapter
                    .maintain(Self::next_maintenance(adapter, entry)?, &[], |tx| {
                        Self::stage(tx, &snapshot)
                    })
                    .map_err(composition_error)?;
                Ok(true)
            })?;
            if found {
                return Ok(());
            }
        }
        Ok(())
    }
    pub(super) fn notice_id(notice: &Notification) -> Result<[u8; 32], EnrollmentError> {
        Ok(hash(&encode(&(
            "terminal-notice",
            notice.proof.id()?,
            notice.recipient,
        ))?))
    }
    pub(super) fn accept_terminal(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        requester: IdentityHash,
        proof: Terminal,
    ) -> Result<(), EnrollmentError> {
        let mut catalog = self.read()?;
        let group = proof.statement.group;
        if requester.into_bytes() != proof.statement.owner.rns {
            return Err(EnrollmentError::Authentication);
        }
        let entry = catalog.entries.iter().find(|e| e.group == group).cloned();
        let known_owner = entry
            .as_ref()
            .map(|e| &e.owner)
            .or_else(|| {
                catalog
                    .retired
                    .iter()
                    .find(|r| r.group == group)
                    .map(|r| &r.owner)
            })
            .or_else(|| {
                catalog
                    .closed_groups
                    .iter()
                    .find(|p| p.statement.group == group)
                    .map(|p| &p.statement.owner)
            })
            .or_else(|| {
                catalog
                    .invites
                    .iter()
                    .find(|i| i.offer.group == group)
                    .map(|i| &i.offer.owner)
            })
            .ok_or(EnrollmentError::Missing)?;
        proof.verify(group, known_owner)?;
        if let Some(entry) = &entry {
            let applies = match &proof.statement.kind {
                TerminalKind::Closed => true,
                TerminalKind::Removed(member) => member == &entry.local,
            };
            if applies {
                self.management_adapter(owner, entry, None, |adapter, mut snapshot| {
                    if snapshot.terminals.contains(&proof) {
                        return Ok(());
                    }
                    if matches!(snapshot.lifecycle, GroupLifecycle::Closed) {
                        return Ok(());
                    }
                    if snapshot
                        .current
                        .as_ref()
                        .is_some_and(|s| s.manifest.revision > proof.statement.revision)
                    {
                        return Err(EnrollmentError::StaleMembership);
                    }
                    snapshot.lifecycle = match proof.statement.kind {
                        TerminalKind::Closed => GroupLifecycle::Closed,
                        _ => GroupLifecycle::Removed,
                    };
                    snapshot.terminals.push(proof.clone());
                    if snapshot.current.is_none()
                        || adapter
                            .is_retired(GroupId(group))
                            .map_err(|_| EnrollmentError::Crypto)?
                    {
                        adapter
                            .maintain(Self::next_maintenance(adapter, entry)?, &[], |tx| {
                                Self::stage(tx, &snapshot)
                            })
                            .map_err(composition_error)?;
                    } else {
                        adapter
                            .compose(
                                Self::next_maintenance(adapter, entry)?,
                                Command::Retire {
                                    group: GroupId(group),
                                },
                                |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                                |tx, _| Self::stage(tx, &snapshot),
                            )
                            .map_err(composition_error)?;
                    }
                    Ok(())
                })?;
            }
        }
        catalog.departures.retain(|d| {
            d.group != group
                || match &proof.statement.kind {
                    TerminalKind::Closed => false,
                    TerminalKind::Removed(member) => member != &d.member,
                }
        });
        for invite in &mut catalog.invites {
            let cancel_invite = matches!(proof.statement.kind, TerminalKind::Closed) || entry.as_ref().is_some_and(|e| e.invitation == Some(invite.offer.id) && matches!(&proof.statement.kind, TerminalKind::Removed(member) if member == &e.local));
            if cancel_invite
                && invite.offer.group == group
                && matches!(
                    invite.state,
                    InvitationState::Pending | InvitationState::JoinRequested
                )
            {
                invite.state = InvitationState::Cancelled;
                invite.acknowledged = true;
                invite.deferred_join = None;
            }
        }
        if matches!(proof.statement.kind, TerminalKind::Closed) {
            if !catalog.closed_groups.contains(&proof) {
                catalog.closed_groups.push(proof.clone());
            }
            for retired in catalog.retired.iter_mut().filter(|r| r.group == group) {
                retired.closed = Some(proof.clone());
            }
        }
        self.write(&catalog)
    }

    pub fn delete_local(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request: [u8; 32],
        group: GroupId,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&("delete-local", group.0))?);
        let (mut catalog, replay) = self.management_admission(owner, request, fingerprint, now)?;
        if replay.is_some() {
            return self.purge_retired(owner);
        }
        let entry = Self::find(&catalog, group.0)?.clone();
        let (lifecycle, closed, mut audience) =
            self.adapter(owner, &entry, None, |_, snapshot| {
                Ok((
                    snapshot.lifecycle,
                    snapshot
                        .terminals
                        .iter()
                        .find(|p| matches!(p.statement.kind, TerminalKind::Closed))
                        .cloned(),
                    snapshot
                        .current
                        .as_ref()
                        .map(|s| s.manifest.members.iter().map(|m| m.rns).collect::<Vec<_>>())
                        .unwrap_or_default(),
                ))
            })?;
        if entry.owner.rns == self.local.into_bytes() && lifecycle != GroupLifecycle::Closed {
            return Err(EnrollmentError::OwnerMustClose);
        }
        audience.extend(
            catalog
                .invites
                .iter()
                .filter(|i| i.offer.group == group.0)
                .map(|i| i.offer.recipient),
        );
        audience.sort();
        audience.dedup();
        let surviving_notices = self
            .pending_notices(owner, &catalog)?
            .into_iter()
            .filter(|n| n.proof.statement.group == group.0)
            .collect::<Vec<_>>();
        for notice in surviving_notices {
            if !catalog
                .notifications
                .iter()
                .any(|n| Self::notice_id(n).ok() == Self::notice_id(&notice).ok())
            {
                catalog.notifications.push(notice);
            }
        }
        let invitations = catalog
            .invites
            .iter()
            .filter(|i| i.offer.group == group.0)
            .map(|i| i.offer.id)
            .collect::<Vec<_>>();
        let mut removed = catalog
            .entries
            .iter()
            .chain(catalog.archives.iter())
            .filter(|e| e.group == group.0)
            .cloned()
            .collect::<Vec<_>>();
        for entry in removed.drain(..) {
            catalog.retired.push(Retired {
                group: entry.group,
                owner: entry.owner.clone(),
                local: entry.local.clone(),
                namespace: entry.namespace,
                incarnation: entry.incarnation,
                purged: false,
                closed: closed.clone(),
                invitations: invitations.clone(),
                audience: audience.clone(),
            });
        }
        catalog.entries.retain(|e| e.group != group.0);
        catalog.archives.retain(|e| e.group != group.0);
        for invite in catalog
            .invites
            .iter_mut()
            .filter(|i| i.offer.group == group.0)
        {
            invite.dismissed = true;
            invite.state = InvitationState::Cancelled;
            invite.acknowledged = true;
        }
        Self::remember(&mut catalog, request, fingerprint, group.0)?;
        // Tombstones and surviving departure/terminal outboxes commit before any
        // namespace can disappear. All admitted jobs now observe the fence.
        self.write(&catalog)?;
        self.purge_retired(owner)
    }
    pub(super) fn purge_retired(
        &self,
        owner: &mut NativeStoreOwner<'db>,
    ) -> Result<(), EnrollmentError> {
        let mut catalog = self.read()?;
        let mut changed = false;
        for retired in &mut catalog.retired {
            if retired.purged {
                continue;
            }
            owner.delete(
                StoreNamespace(retired.namespace),
                StoreIncarnation::new(retired.incarnation).map_err(|_| EnrollmentError::Corrupt)?,
                profile::native_limits(),
            )?;
            crate::session::delete_cursors(self.database, retired.namespace)
                .map_err(|_| EnrollmentError::Corrupt)?;
            retired.purged = true;
            changed = true;
        }
        if changed {
            self.write(&catalog)?;
        }
        Ok(())
    }
    pub fn archived_configurations(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        group: GroupId,
    ) -> Result<Vec<Provisioned>, EnrollmentError> {
        let catalog = self.read()?;
        catalog
            .archives
            .iter()
            .filter(|e| e.group == group.0)
            .map(|entry| self.provisioned_entry(owner, entry, &catalog))
            .collect()
    }
}
