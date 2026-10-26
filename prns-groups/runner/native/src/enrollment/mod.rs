//! Directed, authenticated owner-managed enrollment. All group crypto work and
//! its public control facts commit together on the caller's sole native store lane.
//! The caller authenticates `handle`'s requester using an identified Reticulum
//! Link and authenticates response peers against each Work's recipient.
mod maintenance;
mod management;
mod model;
pub use maintenance::{
    Maintenance, MAX_INCOMING_PER_PEER, MAX_LOCAL_INVITATIONS, MAX_UNACCEPTED_INCOMING,
    RETRY_HORIZON_SECS,
};
mod protocol;
mod storage;

/// Scheduling evidence from a correlated, validated completion. This reports
/// committed control work, not packet arrival or an untrusted reply tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionActivity {
    Idle,
    Progress,
}

use crate::{profile, GroupLifecycle, Provisioned};
use model::*;
pub use model::{
    Invitation, InvitationState, Work, INVITATION_LIFETIME_SECS, MAX_FRAME_BYTES, MAX_GROUPS,
    MAX_INVITATIONS,
};
use prns_groups::{Endpoint, EndpointId, EndpointIncarnation, GroupId, OperationId, Timestamp};
use prns_groups_mls::{Adapter, Admission, Command, LocalIdentity, MemberBinding, Policy};
use prns_groups_store::{Coordinator, OperationTicket, StoreIncarnation};
use prns_groups_store_native::{NativeBackend, NativeError, NativeStoreOwner, StoreNamespace};
use prns_host::IdentityHash;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug)]
pub enum EnrollmentError {
    Invalid,
    Retry(prns_groups::retry::RetryError),
    Authentication,
    Expired,
    Capacity,
    Missing,
    Conflict,
    /// The request identifier already belongs to different immutable input.
    RequestConflict,
    NotOwner,
    NotJoined,
    AlreadyJoined,
    GroupClosed,
    OwnerMustClose,
    MemberNotFound,
    StaleMembership,
    UnsupportedVersion,
    Corrupt,
    Entropy,
    Storage(NativeError),
    Database(redb::Error),
    Crypto,
}
impl std::fmt::Display for EnrollmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "group enrollment: {self:?}")
    }
}
impl std::error::Error for EnrollmentError {}
impl From<NativeError> for EnrollmentError {
    fn from(e: NativeError) -> Self {
        Self::Storage(e)
    }
}
fn db_error(e: impl Into<redb::Error>) -> EnrollmentError {
    EnrollmentError::Database(e.into())
}

#[derive(Clone, Copy)]
enum AdmissionClass {
    Ordinary,
    Management,
}

fn composition_error(
    error: prns_groups_mls::CompositionError<NativeError, EnrollmentError>,
) -> EnrollmentError {
    use prns_groups_mls::{CompositionError, Error as MlsError, ProviderError};
    use prns_groups_store::{OperationRejection, StoreError};
    match error {
        CompositionError::Application(error) => error,
        CompositionError::Mls(
            MlsError::Capacity(_)
            | MlsError::Provider(ProviderError::Capacity(_))
            | MlsError::Store(
                StoreError::Capacity(_)
                | StoreError::Backend(NativeError::Capacity)
                | StoreError::OperationRejected(
                    OperationRejection::RecoveryWindowFull | OperationRejection::ReservedCapacity,
                ),
            ),
        ) => EnrollmentError::Capacity,
        _ => EnrollmentError::Crypto,
    }
}

pub struct Enrollment<'db> {
    database: &'db Database,
    local: IdentityHash,
}
impl<'db> Enrollment<'db> {
    pub fn open(database: &'db Database, local: IdentityHash) -> Result<Self, EnrollmentError> {
        let engine = Self { database, local };
        let mut tx = database.begin_write().map_err(db_error)?;
        tx.set_durability(redb::Durability::Immediate)
            .map_err(db_error)?;
        {
            let mut table = tx.open_table(storage::CATALOG).map_err(db_error)?;
            let existing = table
                .get(0)
                .map_err(db_error)?
                .map(|v| Zeroizing::new(v.value().to_vec()));
            if let Some(bytes) = existing {
                require_version(&bytes)?;
                let catalog: Catalog =
                    serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::Corrupt)?;
                if catalog.version != 2 {
                    return Err(EnrollmentError::UnsupportedVersion);
                }
                if catalog.local != local.into_bytes() {
                    return Err(EnrollmentError::Authentication);
                }
            } else {
                let catalog = Catalog {
                    version: 2,
                    local: local.into_bytes(),
                    entries: Vec::new(),
                    archives: Vec::new(),
                    retired: Vec::new(),
                    departures: Vec::new(),
                    notifications: Vec::new(),
                    closed_groups: Vec::new(),
                    cancellations: Vec::new(),
                    invites: Vec::new(),
                    requests: Vec::new(),
                    request_retired_before: 0,
                    legacy_upgrade_at: None,
                    invitation_time_floor: 0,
                };
                let bytes = Zeroizing::new(encode(&catalog)?);
                table.insert(0, bytes.as_slice()).map_err(db_error)?;
            }
        }
        tx.commit().map_err(db_error)?;
        Ok(engine)
    }
    pub fn create(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request_id: [u8; 32],
        title: &str,
        now: u64,
    ) -> Result<GroupId, EnrollmentError> {
        if title.trim().is_empty() || title.len() > 128 || now < 3600 {
            return Err(EnrollmentError::Invalid);
        }
        let fingerprint = hash(&encode(&("create", title))?);
        let (mut catalog, replay) = self.admission(owner, request_id, fingerprint, now)?;
        let group = if let Some(id) = replay {
            id
        } else {
            if catalog.entries.len() >= MAX_GROUPS {
                return Err(EnrollmentError::Capacity);
            }
            let group = random()?;
            let entry = self.entry(group, title, None, now)?;
            catalog.entries.push(entry);
            Self::remember(&mut catalog, request_id, fingerprint, group)?;
            self.write(&catalog)?;
            group
        };
        if catalog.retired.iter().any(|r| r.group == group) {
            return Ok(GroupId(group));
        }
        self.ensure_created(owner, Self::find(&catalog, group)?)?;
        Ok(GroupId(group))
    }
    fn ensure_created(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
    ) -> Result<(), EnrollmentError> {
        self.initialize(owner, entry)?;
        self.adapter(owner, entry, None, |adapter, snapshot| {
            if snapshot.current.is_some() {
                return Ok(());
            }
            let expected = [entry.local.binding()];
            let identity = entry.identity()?;
            adapter
                .compose_with_roster(
                    Self::next(adapter, entry)?,
                    Command::Create {
                        group: GroupId(entry.group),
                    },
                    Some(&expected),
                    |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                    |tx, output| {
                        let manifest = Manifest {
                            version: 2,
                            group: entry.group,
                            owner: self.local.into_bytes(),
                            title: entry.title.clone(),
                            parent: [0; 32],
                            revision: 0,
                            epoch: output.context.ok_or(EnrollmentError::Corrupt)?.epoch,
                            commit_digest: [0; 32],
                            kind: ControlKind::Create,
                            members: vec![entry.local.clone()],
                        };
                        let signed = Signed {
                            signature: identity
                                .sign_enrollment(&encode(&manifest)?)
                                .map_err(|_| EnrollmentError::Crypto)?,
                            manifest,
                        };
                        Self::stage(
                            tx,
                            &Snapshot {
                                current: Some(signed),
                                ..Snapshot::default()
                            },
                        )
                    },
                )
                .map_err(composition_error)?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn invite(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request_id: [u8; 32],
        group: GroupId,
        recipient: IdentityHash,
        now: u64,
    ) -> Result<[u8; 32], EnrollmentError> {
        if recipient == self.local || now < 3600 {
            return Err(EnrollmentError::Invalid);
        }
        let fingerprint = hash(&encode(&("invite", group.0, recipient.into_bytes()))?);
        let (mut catalog, replay) = self.admission(owner, request_id, fingerprint, now)?;
        if let Some(id) = replay {
            return Ok(id);
        }
        let now = now.max(catalog.invitation_time_floor);
        if catalog.invites.len() >= MAX_INVITATIONS
            || self.local_invitation_count(&catalog) >= MAX_LOCAL_INVITATIONS
        {
            return Err(EnrollmentError::Capacity);
        }
        let entry = Self::find(&catalog, group.0)?;
        if entry.owner.rns != self.local.into_bytes() {
            return Err(EnrollmentError::NotOwner);
        }
        self.adapter(owner, entry, None, |_, snapshot| {
            Self::require_active(&snapshot)?;
            let manifest = &snapshot.current.ok_or(EnrollmentError::NotJoined)?.manifest;
            let pending = catalog
                .invites
                .iter()
                .filter(|invite| {
                    invite.offer.group == group.0
                        && invite.offer.owner.rns == self.local.into_bytes()
                        && !invite.dismissed
                        && invite.state == InvitationState::Pending
                        && now < invite.offer.expires
                        && !snapshot
                            .added
                            .iter()
                            .any(|added| added.invitation == invite.offer.id)
                })
                .collect::<Vec<_>>();
            if manifest
                .members
                .iter()
                .any(|member| member.rns == recipient.into_bytes())
                || pending
                    .iter()
                    .any(|invite| invite.offer.recipient == recipient.into_bytes())
            {
                return Err(EnrollmentError::Conflict);
            }
            // Invitations reserve the eventual membership slot before any peer
            // accepts, so independently accepted invites cannot overbook MLS.
            if manifest.members.len() + pending.len() >= 8 {
                return Err(EnrollmentError::Capacity);
            }
            Ok(())
        })?;
        let id = random()?;
        let offer = Offer {
            id,
            group: group.0,
            title: entry.title.clone(),
            owner: entry.owner.clone(),
            recipient: recipient.into_bytes(),
            created: now,
            expires: now
                .checked_add(INVITATION_LIFETIME_SECS)
                .ok_or(EnrollmentError::Invalid)?,
        };
        catalog.invites.push(Invite {
            offer,
            state: InvitationState::Pending,
            acknowledged: false,
            terminal_at: None,
            deferred_join: None,
            dismissed: false,
        });
        Self::remember(&mut catalog, request_id, fingerprint, id)?;
        self.write(&catalog)?;
        Ok(id)
    }
    /// Resolve an invitation's stable group without maintenance or other writes.
    /// Dismissed and locally deleted invitations retain this association.
    pub fn invitation_group(
        &self,
        invitation: [u8; 32],
    ) -> Result<Option<GroupId>, EnrollmentError> {
        let catalog = self.read()?;
        let candidates = catalog
            .invites
            .iter()
            .filter(|invite| invite.offer.id == invitation)
            .map(|invite| invite.offer.group)
            .chain(
                catalog
                    .entries
                    .iter()
                    .chain(&catalog.archives)
                    .filter(|entry| entry.invitation == Some(invitation))
                    .map(|entry| entry.group),
            )
            .chain(
                catalog
                    .retired
                    .iter()
                    .filter(|entry| entry.invitations.contains(&invitation))
                    .map(|entry| entry.group),
            )
            .chain(
                catalog
                    .cancellations
                    .iter()
                    .filter(|entry| entry.invitation == invitation)
                    .map(|entry| entry.group),
            );
        let mut result = None;
        for group in candidates {
            if result.is_some_and(|existing| existing != group) {
                return Err(EnrollmentError::Corrupt);
            }
            result = Some(group);
        }
        Ok(result.map(GroupId))
    }
    pub fn invitations(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        now: u64,
    ) -> Result<Vec<Invitation>, EnrollmentError> {
        self.maintain(owner, now)?;
        let catalog = self.read()?;
        catalog
            .invites
            .iter()
            .filter(|invite| !invite.dismissed)
            .map(|invite| {
                let (state, waiting_for_delivery) =
                    self.invitation_state(owner, &catalog, invite, now)?;
                let can_cancel = if invite.offer.owner.rns == self.local.into_bytes()
                    && state == InvitationState::Pending
                {
                    if let Ok(entry) = Self::find(&catalog, invite.offer.group) {
                        self.adapter(owner, entry, None, |_, snapshot| {
                            Ok(snapshot.lifecycle == GroupLifecycle::Active
                                && !snapshot
                                    .added
                                    .iter()
                                    .any(|a| a.invitation == invite.offer.id))
                        })?
                    } else {
                        false
                    }
                } else {
                    false
                };
                Ok(Invitation {
                    id: invite.offer.id,
                    group: GroupId(invite.offer.group),
                    title: invite.offer.title.clone(),
                    inviter: IdentityHash::new(invite.offer.owner.rns),
                    recipient: IdentityHash::new(invite.offer.recipient),
                    expires_at_secs: invite.offer.expires,
                    outgoing: invite.offer.owner.rns == self.local.into_bytes(),
                    state,
                    waiting_for_delivery,
                    can_cancel,
                    can_dismiss: Self::dismissible(state),
                })
            })
            .collect()
    }
    pub fn respond(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        request_id: [u8; 32],
        id: [u8; 32],
        accept: bool,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let fingerprint = hash(&encode(&("respond", id, accept))?);
        let (mut catalog, replay) = self.admission(owner, request_id, fingerprint, now)?;
        if replay.is_some() {
            self.ensure_packages(owner, &catalog)?;
            return Ok(());
        }
        let index = catalog
            .invites
            .iter()
            .position(|i| i.offer.id == id)
            .ok_or(EnrollmentError::Missing)?;
        let invite = &catalog.invites[index];
        if invite.offer.recipient != self.local.into_bytes()
            || invite.state != InvitationState::Pending
        {
            return Err(EnrollmentError::Conflict);
        }
        require_time(&invite.offer, now.max(catalog.invitation_time_floor))?;
        if accept {
            if (catalog.entries.len() >= MAX_GROUPS
                && !catalog
                    .entries
                    .iter()
                    .any(|e| e.group == invite.offer.group))
                || self.local_invitation_count(&catalog) >= MAX_LOCAL_INVITATIONS
            {
                return Err(EnrollmentError::Capacity);
            }
            if let Some(prior) = catalog
                .entries
                .iter()
                .find(|e| e.group == invite.offer.group)
            {
                let lifecycle =
                    self.adapter(owner, prior, None, |_, snapshot| Ok(snapshot.lifecycle))?;
                let terminal_unjoined = self.adapter(owner, prior, None, |_, snapshot| {
                    Ok(snapshot.current.is_none())
                })? && catalog.invites.iter().any(|i| {
                    i.offer.group == prior.group
                        && i.offer.recipient == self.local.into_bytes()
                        && matches!(
                            i.state,
                            InvitationState::Cancelled | InvitationState::Expired
                        )
                });
                if lifecycle != GroupLifecycle::Removed && !terminal_unjoined {
                    return Err(EnrollmentError::Conflict);
                }
            }
            let mut entry = self.entry(
                invite.offer.group,
                &invite.offer.title,
                Some(invite.offer.owner.clone()),
                now,
            )?;
            entry.invitation = Some(id);
            if let Some(prior) = catalog.entries.iter().position(|e| e.group == entry.group) {
                let old = catalog.entries.remove(prior);
                entry.generation = old
                    .generation
                    .checked_add(1)
                    .ok_or(EnrollmentError::Capacity)?;
                if self.adapter(owner, &old, None, |_, snapshot| {
                    Ok(snapshot.current.is_some())
                })? {
                    catalog.archives.push(old);
                } else {
                    catalog.retired.push(Retired {
                        group: old.group,
                        owner: old.owner.clone(),
                        local: old.local.clone(),
                        namespace: old.namespace,
                        incarnation: old.incarnation,
                        purged: false,
                        closed: None,
                        invitations: catalog
                            .invites
                            .iter()
                            .filter(|i| i.offer.group == old.group && i.offer.id != id)
                            .map(|i| i.offer.id)
                            .collect(),
                        audience: Vec::new(),
                    });
                }
            } else {
                entry.generation = catalog
                    .archives
                    .iter()
                    .filter(|e| e.group == entry.group)
                    .map(|e| e.generation)
                    .max()
                    .map_or(Ok(0), |generation| {
                        generation.checked_add(1).ok_or(EnrollmentError::Capacity)
                    })?;
            }
            catalog.entries.push(entry);
        }
        catalog.invites[index].state = if accept {
            InvitationState::JoinRequested
        } else {
            InvitationState::Declined
        };
        catalog.invites[index].acknowledged = false;
        Self::remember(&mut catalog, request_id, fingerprint, id)?;
        self.write(&catalog)?;
        self.ensure_packages(owner, &catalog)
    }
    pub fn groups(
        &self,
        owner: &mut NativeStoreOwner<'db>,
    ) -> Result<Vec<GroupId>, EnrollmentError> {
        let catalog = self.read()?;
        self.recover_creates(owner, &catalog)?;
        let mut groups = Vec::new();
        for e in &catalog.entries {
            self.initialize(owner, e)?;
            if self.adapter(owner, e, None, |_, s| {
                Ok(s.current.is_some() && (s.lifecycle != GroupLifecycle::Active || s.ready()?))
            })? {
                groups.push(GroupId(e.group));
            }
        }
        Ok(groups)
    }
    /// Local projection only: do not run invitation maintenance or finish
    /// enrollment work merely because a consumer refreshes an activity badge.
    pub(crate) fn actionable_invitations(&self, now: u64) -> Result<usize, EnrollmentError> {
        let catalog = self.read()?;
        let now = now.max(catalog.invitation_time_floor);
        Ok(catalog
            .invites
            .iter()
            .filter(|invite| {
                invite.offer.recipient == self.local.into_bytes()
                    && invite.offer.owner.rns != self.local.into_bytes()
                    && invite.state == InvitationState::Pending
                    && now < invite.offer.expires
            })
            .count())
    }
    pub(crate) fn local_configurations(
        &self,
        owner: &mut NativeStoreOwner<'db>,
    ) -> Result<Vec<Provisioned>, EnrollmentError> {
        let catalog = self.read()?;
        let mut result = Vec::with_capacity(catalog.entries.len());
        for entry in &catalog.entries {
            match self.provisioned(owner, GroupId(entry.group)) {
                Ok(cfg) => result.push(cfg),
                Err(EnrollmentError::NotJoined) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }
    pub fn provisioned(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        group: GroupId,
    ) -> Result<Provisioned, EnrollmentError> {
        let catalog = self.read()?;
        let entry = Self::find(&catalog, group.0)?;
        self.provisioned_entry(owner, entry, &catalog)
    }
    fn provisioned_entry(
        &self,
        owner: &mut NativeStoreOwner<'db>,
        entry: &Entry,
        catalog: &Catalog,
    ) -> Result<Provisioned, EnrollmentError> {
        let group = GroupId(entry.group);
        self.adapter(owner, entry, None, |_, snapshot| {
            if snapshot.lifecycle == GroupLifecycle::Active && !snapshot.ready()? {
                return Err(EnrollmentError::NotJoined);
            }
            let signed = snapshot
                .current
                .as_ref()
                .ok_or(EnrollmentError::NotJoined)?;
            signed.verify(entry.group, &entry.owner, &entry.title)?;
            let readers = signed
                .manifest
                .members
                .iter()
                .map(Member::endpoint)
                .collect::<Result<Vec<_>, _>>()?;
            let bindings = signed
                .manifest
                .members
                .iter()
                .map(Member::binding)
                .collect();
            let head = prns_groups::ControlId(signed.head()?);
            let peers = signed
                .manifest
                .members
                .iter()
                .filter(|m| m.rns != self.local.into_bytes())
                .map(|member| {
                    Ok(prns_groups_reticulum::Peer {
                        endpoint: member.endpoint()?,
                        identity: IdentityHash::new(member.rns),
                        destination: prns_groups_reticulum::destination_for(IdentityHash::new(
                            member.rns,
                        ))
                        .map_err(|_| EnrollmentError::Invalid)?,
                    })
                })
                .collect::<Result<Vec<_>, EnrollmentError>>()?;
            let mut history_peers = Vec::new();
            for member in snapshot
                .controls
                .iter()
                .flat_map(|c| c.signed.manifest.members.iter())
                .chain(signed.manifest.members.iter())
            {
                let endpoint = member.endpoint()?;
                if member.rns == self.local.into_bytes()
                    || history_peers
                        .iter()
                        .any(|peer: &prns_groups_reticulum::Peer| peer.endpoint == endpoint)
                {
                    continue;
                }
                history_peers.push(prns_groups_reticulum::Peer {
                    endpoint,
                    identity: IdentityHash::new(member.rns),
                    destination: prns_groups_reticulum::destination_for(IdentityHash::new(
                        member.rns,
                    ))
                    .map_err(|_| EnrollmentError::Invalid)?,
                });
            }
            let lifecycle = if catalog
                .departures
                .iter()
                .any(|d| d.group == entry.group && d.member == entry.local)
                && snapshot.lifecycle == GroupLifecycle::Active
            {
                GroupLifecycle::Leaving
            } else {
                snapshot.lifecycle
            };
            Ok(Provisioned {
                lifecycle,
                history_generation: entry.generation,
                history_peers,
                owner_identity: Some(IdentityHash::new(entry.owner.rns)),
                group,
                control: prns_groups_service::ControlReference {
                    head,
                    era: prns_groups::AuthorizationEra(head),
                },
                readers,
                identity: entry.identity()?,
                policy: Policy::owner_managed(entry.owner.binding(), bindings)
                    .map_err(|_| EnrollmentError::Corrupt)?,
                namespace: StoreNamespace(entry.namespace),
                incarnation: StoreIncarnation::new(entry.incarnation)
                    .map_err(|_| EnrollmentError::Corrupt)?,
                native_limits: profile::native_limits(),
                mls_limits: profile::mls_limits(),
                service_limits: profile::service_limits(),
                title: entry.title.clone(),
                peers,
                local_reticulum_identity: self.local,
            })
        })
    }
}

#[cfg(test)]
mod tests;
