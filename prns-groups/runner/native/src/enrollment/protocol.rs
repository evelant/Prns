use super::*;
impl<'db> Enrollment<'db> {
    pub fn handle(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        requester: IdentityHash,
        bytes: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, EnrollmentError> {
        let request: Frame = decode(bytes)?;
        self.maintain(owner, now)?;
        match request {
            Frame::Offer(offer) => self.receive_offer(requester, offer, now),
            Frame::Join {
                invitation,
                member,
                package,
            } => self.receive_join(owner, requester, invitation, member, package, now),
            Frame::Joined { invitation, head } => {
                let mut catalog = self.read()?;
                let (_, snapshot, added) = self
                    .retained_add(owner, &catalog, invitation, requester)?
                    .ok_or(EnrollmentError::Missing)?;
                let signed = snapshot
                    .current
                    .iter()
                    .chain(snapshot.controls.iter().map(|c| &c.signed))
                    .find(|signed| signed.head().ok() == Some(head))
                    .ok_or(EnrollmentError::Authentication)?;
                if signed.manifest.epoch < added.signed.manifest.epoch {
                    return Err(EnrollmentError::Authentication);
                }
                if let Some(invite) = catalog
                    .invites
                    .iter_mut()
                    .find(|i| i.offer.id == invitation)
                {
                    invite.state = InvitationState::Joined;
                    invite.acknowledged = true;
                    invite
                        .terminal_at
                        .get_or_insert(now.max(catalog.invitation_time_floor));
                    self.write(&catalog)?;
                }
                frame(&Reply::Ack)
            }
            Frame::Decline { invitation } => {
                let mut catalog = self.read()?;
                let Some(invite) = catalog
                    .invites
                    .iter_mut()
                    .find(|i| i.offer.id == invitation)
                else {
                    return frame(&Reply::Ack);
                };
                if invite.offer.owner.rns != self.local.into_bytes()
                    || invite.offer.recipient != requester.into_bytes()
                {
                    return Err(EnrollmentError::Authentication);
                }
                // A committed Add is irreversible in this add-only profile.
                let group = invite.offer.group;
                let entry = Self::find(&catalog, group)?;
                if self.adapter(owner, entry, None, |_, s| {
                    Ok(s.added.iter().any(|a| a.invitation == invitation))
                })? {
                    return Err(EnrollmentError::Conflict);
                }
                let invite = catalog
                    .invites
                    .iter_mut()
                    .find(|i| i.offer.id == invitation)
                    .ok_or(EnrollmentError::Missing)?;
                invite.state = InvitationState::Declined;
                invite.acknowledged = true;
                self.write(&catalog)?;
                frame(&Reply::Ack)
            }
            Frame::Sync { group, head } => {
                let catalog = self.read()?;
                let entry = Self::find(&catalog, group)?;
                self.adapter(owner, entry, None, |_, snapshot| {
                    let current = snapshot
                        .current
                        .as_ref()
                        .ok_or(EnrollmentError::NotJoined)?;
                    if !current
                        .manifest
                        .members
                        .iter()
                        .any(|m| m.rns == requester.into_bytes())
                    {
                        return Err(EnrollmentError::Authentication);
                    }
                    if head == current.head()? {
                        return frame(&Reply::Control(None));
                    }
                    let next = snapshot
                        .controls
                        .iter()
                        .find(|c| c.signed.manifest.parent == head)
                        .ok_or(EnrollmentError::Conflict)?;
                    frame(&Reply::Control(Some(next.clone())))
                })
            }
        }
    }
    fn receive_offer(
        &self,
        requester: IdentityHash,
        offer: Offer,
        now: u64,
    ) -> Result<Vec<u8>, EnrollmentError> {
        if requester.into_bytes() != offer.owner.rns
            || offer.recipient != self.local.into_bytes()
            || requester == self.local
            || offer.owner.incarnation == 0
            || offer.title.trim().is_empty()
            || offer.title.len() > 128
        {
            return Err(EnrollmentError::Authentication);
        }
        let mut catalog = self.read()?;
        if let Some(prior) = catalog.invites.iter().find(|i| i.offer.id == offer.id) {
            return if prior.offer == offer {
                frame(&Reply::Ack)
            } else {
                Err(EnrollmentError::Conflict)
            };
        }
        require_time(&offer, now.max(catalog.invitation_time_floor))?;
        self.admit_incoming(&catalog, requester)?;
        // A group address has a single immutable owner binding and title. Never
        // rebind an existing namespace through a second invite or a new peer.
        if catalog.entries.iter().any(|e| e.group == offer.group)
            || catalog.invites.iter().any(|i| {
                i.offer.group == offer.group
                    && (i.offer.owner != offer.owner || i.offer.title != offer.title)
            })
        {
            return Err(EnrollmentError::Conflict);
        }
        catalog.invites.push(Invite {
            offer,
            state: InvitationState::Pending,
            acknowledged: true,
            terminal_at: None,
            deferred_join: None,
        });
        self.write(&catalog)?;
        frame(&Reply::Ack)
    }
    fn receive_join(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        requester: IdentityHash,
        invitation: [u8; 32],
        member: Member,
        package: Vec<u8>,
        now: u64,
    ) -> Result<Vec<u8>, EnrollmentError> {
        if member.rns != requester.into_bytes()
            || member.incarnation == 0
            || package.len() > profile::mls_limits().max_wire_bytes
        {
            return Err(EnrollmentError::Authentication);
        }
        let catalog = self.read()?;
        let request_hash = hash(&encode(&Frame::Join {
            invitation,
            member: member.clone(),
            package: package.clone(),
        })?);
        if let Some((_, snapshot, added)) =
            self.retained_add(owner, &catalog, invitation, requester)?
        {
            if added.request_hash != request_hash {
                return Err(EnrollmentError::Conflict);
            }
            return frame(&Reply::Welcome {
                added,
                tip: Box::new(snapshot.current.ok_or(EnrollmentError::Corrupt)?),
            });
        }
        let Some(invite) = catalog.invites.iter().find(|i| i.offer.id == invitation) else {
            return frame(&Reply::Expired);
        };
        if invite.offer.owner.rns != self.local.into_bytes()
            || invite.offer.recipient != requester.into_bytes()
        {
            return Err(EnrollmentError::Authentication);
        }
        let entry = Self::find(&catalog, invite.offer.group)?;
        let (prior, mut roster) = self.adapter(owner, entry, None, |_, snapshot| {
            let prior = snapshot.current.ok_or(EnrollmentError::NotJoined)?;
            Ok((prior.clone(), prior.manifest.members.clone()))
        })?;
        let now = now.max(catalog.invitation_time_floor);
        if now >= invite.offer.expires {
            return frame(&Reply::Expired);
        }
        require_time(&invite.offer, now)?;
        if invite.state != InvitationState::Pending {
            return Err(EnrollmentError::Conflict);
        }
        if roster.len() >= 8 {
            return Err(EnrollmentError::Capacity);
        }
        if roster
            .iter()
            .any(|m| m.endpoint == member.endpoint || m.rns == member.rns || m.key == member.key)
        {
            return Err(EnrollmentError::Conflict);
        }
        if invite
            .deferred_join
            .is_some_and(|prior| prior != request_hash)
        {
            return Err(EnrollmentError::Conflict);
        }
        // This lane exclusively owns both service publications/receipts and Add.
        // Check at the actual transition, including sends admitted after Offer.
        // Never weaken the old-era fence to make a delayed join succeed.
        let waiting = self.owned_adapter(owner, entry, None, |adapter, snapshot| {
            if snapshot.current.as_ref() != Some(&prior) {
                return Err(EnrollmentError::Conflict);
            }
            if !adapter
                .key_package_matches(&package, &member.binding())
                .map_err(|_| EnrollmentError::Authentication)?
            {
                return Err(EnrollmentError::Authentication);
            }
            let readers = prior
                .manifest
                .members
                .iter()
                .map(Member::endpoint)
                .collect::<Result<Vec<_>, _>>()?;
            let head = prns_groups::ControlId(prior.head()?);
            let service = prns_groups_service::GroupService::new_managed(
                adapter,
                prns_groups_service::Descriptor {
                    group: GroupId(entry.group),
                    control: prns_groups_service::ControlReference {
                        head,
                        era: prns_groups::AuthorizationEra(head),
                    },
                    readers: &readers,
                },
                profile::service_limits(),
            )
            .map_err(|_| EnrollmentError::Corrupt)?;
            let clock = prns_groups::ClockReading::Trusted(
                prns_groups::ClockInterval::new(Timestamp(now), Timestamp(now))
                    .map_err(|_| EnrollmentError::Invalid)?,
            );
            service
                .has_pending_authored_deliveries(clock)
                .map_err(|_| EnrollmentError::Corrupt)
        })?;
        if waiting {
            self.defer_join(invitation, request_hash)?;
            return frame(&Reply::AwaitingDelivery);
        }
        roster.push(member);
        roster.sort_by_key(|m| m.endpoint);
        let expected = roster.iter().map(Member::binding).collect::<Vec<_>>();
        let added = self.adapter(owner, entry, Some(&roster), |adapter, mut snapshot| {
            if snapshot.current.as_ref() != Some(&prior) {
                return Err(EnrollmentError::Conflict);
            }
            let candidate = roster
                .iter()
                .find(|m| m.rns == requester.into_bytes())
                .ok_or(EnrollmentError::Authentication)?;
            if !adapter
                .key_package_matches(&package, &candidate.binding())
                .map_err(|_| EnrollmentError::Authentication)?
            {
                return Err(EnrollmentError::Authentication);
            }
            let identity = entry.identity()?;
            let mut added = None;
            adapter
                .compose_with_roster(
                    Self::next(adapter, entry)?,
                    Command::Add {
                        group: GroupId(entry.group),
                        packages: &[&package],
                        now_secs: now,
                    },
                    Some(&expected),
                    |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                    |tx, output| {
                        if output.messages.len() != 2 {
                            return Err(EnrollmentError::Corrupt);
                        }
                        let commit = output.messages[0].clone();
                        let welcome = output.messages[1].clone();
                        let manifest = Manifest {
                            version: 1,
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
                            members: roster.clone(),
                        };
                        let signed = Signed {
                            signature: identity
                                .sign_enrollment(&encode(&manifest)?)
                                .map_err(|_| EnrollmentError::Crypto)?,
                            manifest,
                        };
                        Self::transition(entry, &prior, &signed, &commit)?;
                        let result = Added {
                            invitation,
                            request_hash,
                            signed: signed.clone(),
                            welcome,
                        };
                        frame(&Reply::Welcome {
                            tip: Box::new(signed.clone()),
                            added: result.clone(),
                        })?;
                        // Every previously admitted member must still be able to
                        // recover its exact Welcome with this newer signed tip.
                        for previous in &snapshot.added {
                            frame(&Reply::Welcome {
                                tip: Box::new(signed.clone()),
                                added: previous.clone(),
                            })?;
                        }
                        let control = Control {
                            signed: signed.clone(),
                            commit,
                        };
                        frame(&Reply::Control(Some(control.clone())))?;
                        snapshot.current = Some(signed);
                        snapshot.controls.push(control);
                        snapshot.added.push(result.clone());
                        Self::stage(tx, &snapshot)?;
                        added = Some(result);
                        Ok(())
                    },
                )
                .map_err(|_| EnrollmentError::Crypto)?;
            added.ok_or(EnrollmentError::Corrupt)
        })?;
        frame(&Reply::Welcome {
            tip: Box::new(added.signed.clone()),
            added,
        })
    }
    pub fn pending(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        now: u64,
    ) -> Result<Vec<Work>, EnrollmentError> {
        self.maintain(owner, now)?;
        let catalog = self.read()?;
        self.recover_creates(owner, &catalog)?;
        self.ensure_packages(owner, &catalog)?;
        let mut work = Vec::new();
        for invite in &catalog.invites {
            let offer = &invite.offer;
            if offer.owner.rns == self.local.into_bytes() {
                if !invite.acknowledged
                    && invite.state == InvitationState::Pending
                    && now < offer.expires
                {
                    work.push(Work {
                        id: work_id(b"offer", &offer.id),
                        recipient: IdentityHash::new(offer.recipient),
                        frame: frame(&Frame::Offer(offer.clone()))?,
                    });
                }
            } else if invite.state == InvitationState::JoinRequested {
                let entry = Self::find(&catalog, offer.group)?;
                self.adapter(owner, entry, None, |_, snapshot| {
                    if snapshot.ready()? && !invite.acknowledged {
                        work.push(Work {
                            id: work_id(b"joined", &offer.id),
                            recipient: IdentityHash::new(offer.owner.rns),
                            frame: frame(&Frame::Joined {
                                invitation: offer.id,
                                head: snapshot
                                    .current
                                    .as_ref()
                                    .ok_or(EnrollmentError::Corrupt)?
                                    .head()?,
                            })?,
                        });
                    } else if snapshot.current.is_none() {
                        let package = snapshot.package.ok_or(EnrollmentError::Corrupt)?;
                        work.push(Work {
                            id: work_id(b"join", &offer.id),
                            recipient: IdentityHash::new(offer.owner.rns),
                            frame: frame(&Frame::Join {
                                invitation: offer.id,
                                member: entry.local.clone(),
                                package,
                            })?,
                        });
                    }
                    Ok(())
                })?;
            } else if invite.state == InvitationState::Declined && !invite.acknowledged {
                work.push(Work {
                    id: work_id(b"decline", &offer.id),
                    recipient: IdentityHash::new(offer.owner.rns),
                    frame: frame(&Frame::Decline {
                        invitation: offer.id,
                    })?,
                });
            }
        }
        for entry in &catalog.entries {
            if entry.owner.rns == self.local.into_bytes() {
                continue;
            }
            self.initialize(owner, entry)?;
            self.adapter(owner, entry, None, |_, snapshot| {
                if let Some(current) = snapshot.current {
                    let head = current.head()?;
                    work.push(Work {
                        id: work_id(b"sync", &head),
                        recipient: IdentityHash::new(entry.owner.rns),
                        frame: frame(&Frame::Sync {
                            group: entry.group,
                            head,
                        })?,
                    });
                }
                Ok(())
            })?;
        }
        Ok(work)
    }
    /// Complete only a response obtained from the Work's authenticated recipient.
    /// A dropped response can always be retried against the exact original work.
    pub fn complete(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        id: [u8; 32],
        response: &[u8],
        now: u64,
    ) -> Result<CompletionActivity, EnrollmentError> {
        let reply: Reply = decode(response)?;
        let work = self.pending(owner, now)?.into_iter().find(|w| w.id == id);
        let Some(work) = work else {
            // Duplicate completion after commitment must be a no-op, without
            // accepting fresh state or interpreting an uncorrelated response.
            return Ok(CompletionActivity::Idle);
        };
        match (decode::<Frame>(&work.frame)?, reply) {
            (Frame::Offer(offer), Reply::Ack) => self
                .acknowledge(offer.id)
                .map(|()| CompletionActivity::Progress),
            (Frame::Decline { invitation }, Reply::Ack) => self
                .acknowledge(invitation)
                .map(|()| CompletionActivity::Progress),
            (Frame::Joined { invitation, .. }, Reply::Ack) => {
                let mut catalog = self.read()?;
                let invite = catalog
                    .invites
                    .iter_mut()
                    .find(|i| i.offer.id == invitation)
                    .ok_or(EnrollmentError::Missing)?;
                invite.state = InvitationState::Joined;
                invite.acknowledged = true;
                invite
                    .terminal_at
                    .get_or_insert(now.max(catalog.invitation_time_floor));
                self.write(&catalog).map(|()| CompletionActivity::Progress)
            }
            (join @ Frame::Join { .. }, Reply::AwaitingDelivery) => {
                let Frame::Join { invitation, .. } = &join else {
                    return Err(EnrollmentError::Invalid);
                };
                self.defer_join(*invitation, hash(&encode(&join)?))?;
                // A deferred Join keeps the exact durable KeyPackage and normal
                // retry schedule; it is not fresh membership progress.
                Ok(CompletionActivity::Idle)
            }
            (
                Frame::Join {
                    invitation,
                    member,
                    package,
                },
                Reply::Welcome { added, tip },
            ) => {
                if added.invitation != invitation
                    || added.request_hash
                        != hash(&encode(&Frame::Join {
                            invitation,
                            member,
                            package,
                        })?)
                {
                    return Err(EnrollmentError::Authentication);
                }
                let catalog = self.read()?;
                let invite = catalog
                    .invites
                    .iter()
                    .find(|i| i.offer.id == invitation)
                    .ok_or(EnrollmentError::Missing)?;
                let entry = Self::find(&catalog, invite.offer.group)?;
                added
                    .signed
                    .verify(entry.group, &entry.owner, &entry.title)?;
                tip.verify(entry.group, &entry.owner, &entry.title)?;
                if tip.manifest.epoch < added.signed.manifest.epoch
                    || added
                        .signed
                        .manifest
                        .members
                        .iter()
                        .any(|member| !tip.manifest.members.contains(member))
                    || (tip.manifest.epoch == added.signed.manifest.epoch && *tip != added.signed)
                {
                    return Err(EnrollmentError::Authentication);
                }
                if !added.signed.manifest.members.contains(&entry.local) {
                    return Err(EnrollmentError::Authentication);
                }
                let roster = &added.signed.manifest.members;
                let expected = roster.iter().map(Member::binding).collect::<Vec<_>>();
                self.adapter(owner, entry, Some(roster), |adapter, mut snapshot| {
                    if snapshot.current.is_some() {
                        return Ok(CompletionActivity::Idle);
                    }
                    adapter
                        .compose_with_roster(
                            Self::next(adapter, entry)?,
                            Command::Join {
                                group: GroupId(entry.group),
                                welcome: &added.welcome,
                                now_secs: now,
                            },
                            Some(&expected),
                            |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                            |tx, output| {
                                if output.context.ok_or(EnrollmentError::Corrupt)?.epoch
                                    != added.signed.manifest.epoch
                                {
                                    return Err(EnrollmentError::Authentication);
                                }
                                snapshot.required_tip = Some((*tip).clone());
                                snapshot.current = Some(added.signed.clone());
                                snapshot.package = None;
                                Self::stage(tx, &snapshot)
                            },
                        )
                        .map_err(|_| EnrollmentError::Crypto)?;
                    Ok(CompletionActivity::Progress)
                })
            }
            (Frame::Join { invitation, .. }, Reply::Expired) => {
                let mut catalog = self.read()?;
                let invite = catalog
                    .invites
                    .iter_mut()
                    .find(|i| i.offer.id == invitation)
                    .ok_or(EnrollmentError::Missing)?;
                invite.state = InvitationState::Expired;
                invite.acknowledged = true;
                invite
                    .terminal_at
                    .get_or_insert(now.max(catalog.invitation_time_floor));
                self.write(&catalog).map(|()| CompletionActivity::Progress)
            }
            (Frame::Sync { group, head }, Reply::Control(control)) => {
                let Some(control) = control else {
                    return Ok(CompletionActivity::Idle);
                };
                let catalog = self.read()?;
                let entry = Self::find(&catalog, group)?;
                let prior = self.adapter(owner, entry, None, |_, s| {
                    s.current.ok_or(EnrollmentError::NotJoined)
                })?;
                if prior.head()? != head {
                    return Err(EnrollmentError::Conflict);
                }
                Self::transition(entry, &prior, &control.signed, &control.commit)?;
                let roster = &control.signed.manifest.members;
                let expected = roster.iter().map(Member::binding).collect::<Vec<_>>();
                self.adapter(owner, entry, Some(roster), |adapter, mut snapshot| {
                    adapter
                        .compose_with_roster(
                            Self::next(adapter, entry)?,
                            Command::Receive {
                                group: GroupId(group),
                                bytes: &control.commit,
                            },
                            Some(&expected),
                            |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                            |tx, output| {
                                if output.event != prns_groups_mls::Event::Control
                                    || output.context.ok_or(EnrollmentError::Corrupt)?.epoch
                                        != control.signed.manifest.epoch
                                {
                                    return Err(EnrollmentError::Authentication);
                                }
                                snapshot.current = Some(control.signed.clone());
                                snapshot.controls.push(control.clone());
                                // Crossing the required join checkpoint must match
                                // the signed tip exactly before publication.
                                snapshot.ready()?;
                                Self::stage(tx, &snapshot)
                            },
                        )
                        .map_err(|_| EnrollmentError::Crypto)?;
                    Ok(CompletionActivity::Progress)
                })
            }
            _ => Err(EnrollmentError::Invalid),
        }
    }
    fn defer_join(
        &self,
        invitation: [u8; 32],
        request_hash: [u8; 32],
    ) -> Result<(), EnrollmentError> {
        let mut catalog = self.read()?;
        let invite = catalog
            .invites
            .iter_mut()
            .find(|i| i.offer.id == invitation)
            .ok_or(EnrollmentError::Missing)?;
        if let Some(prior) = invite.deferred_join {
            return if prior == request_hash {
                Ok(())
            } else {
                Err(EnrollmentError::Conflict)
            };
        }
        invite.deferred_join = Some(request_hash);
        self.write(&catalog)
    }
    fn acknowledge(&self, id: [u8; 32]) -> Result<(), EnrollmentError> {
        let mut catalog = self.read()?;
        let invite = catalog
            .invites
            .iter_mut()
            .find(|i| i.offer.id == id)
            .ok_or(EnrollmentError::Missing)?;
        invite.acknowledged = true;
        self.write(&catalog)
    }
    fn transition(
        entry: &Entry,
        prior: &Signed,
        next: &Signed,
        commit: &[u8],
    ) -> Result<(), EnrollmentError> {
        next.verify(entry.group, &entry.owner, &entry.title)?;
        if next.manifest.revision != prior.manifest.revision + 1
            || next.manifest.commit_digest != hash(commit)
            || prior
                .manifest
                .members
                .iter()
                .any(|m| !next.manifest.members.contains(m))
        {
            return Err(EnrollmentError::Authentication);
        }
        let old = prior
            .manifest
            .members
            .iter()
            .map(Member::endpoint)
            .collect::<Result<Vec<_>, _>>()?;
        let new = next
            .manifest
            .members
            .iter()
            .map(Member::endpoint)
            .collect::<Result<Vec<_>, _>>()?;
        prns_groups::enrollment::validate_add_transition(
            GroupId(entry.group),
            prior.head()?,
            prior.manifest.epoch,
            &old,
            GroupId(next.manifest.group),
            next.manifest.parent,
            next.manifest.epoch,
            &new,
            8,
        )
        .map_err(|_| EnrollmentError::Authentication)?;
        Ok(())
    }
}
