//! One native database owner for the enrollment catalog and every group namespace.
use crate::{
    client::{Command, Reply},
    enrollment::Enrollment,
    Error, Job, Provisioned,
};
use prns_groups::GroupId;
use prns_groups_store_native::NativeStoreOwner;
use prns_host::IdentityHash;
use redb::Database;

pub struct GroupManager<'db> {
    database: &'db Database,
    owner: NativeStoreOwner<'db>,
    enrollment: Enrollment<'db>,
    identity: IdentityHash,
    legacy: Option<Provisioned>,
}

/// Bounded local text activity, independent of transport and UI selection.
#[derive(Debug)]
pub struct LocalMessageActivity {
    pub group: GroupId,
    pub latest_sequence: u64,
    pub incoming_sequences: Vec<u64>,
}
#[derive(Debug)]
pub struct LocalActivity {
    pub actionable_invitations: usize,
    pub groups: Vec<LocalMessageActivity>,
}
impl<'db> GroupManager<'db> {
    /// Stable, read-only invitation scope for a caller's durable action journal.
    pub fn invitation_group(&self, invitation: [u8; 32]) -> Result<Option<GroupId>, Error> {
        self.enrollment
            .invitation_group(invitation)
            .map_err(enrollment_error)
    }
    /// Inspect retained text records on the existing storage lane. Does not
    /// notify a runner, announce, publish, or change a membership/read receipt.
    pub fn local_activity(&mut self, now: u64) -> Result<LocalActivity, Error> {
        let actionable_invitations = self
            .enrollment
            .actionable_invitations(now)
            .map_err(enrollment_error)?;
        let mut configs = self
            .enrollment
            .local_configurations(&mut self.owner)
            .map_err(enrollment_error)?;
        if let Some(legacy) = &self.legacy {
            configs.push(legacy.clone());
        }
        let mut groups = Vec::with_capacity(configs.len());
        for cfg in configs {
            // The profile bounds retained records; pagination never grows with
            // lifetime message count and positions are durable change cursors.
            let mut activity = LocalMessageActivity {
                group: cfg.group,
                latest_sequence: 0,
                incoming_sequences: Vec::new(),
            };
            let mut before = None;
            let mut visited = 0;
            loop {
                let Reply::Messages(page) = crate::session::execute_with_owner(
                    self.database,
                    &mut self.owner,
                    &cfg,
                    Command::Messages {
                        group: cfg.group,
                        before_sequence: before,
                        limit: 100,
                        clock: prns_groups_service::ClockReading::Unknown,
                    },
                )?
                else {
                    return Err(Error::Corrupt);
                };
                for message in page.messages {
                    visited += 1;
                    if visited > cfg.service_limits.max_records {
                        return Err(Error::Corrupt);
                    }
                    activity.latest_sequence = activity.latest_sequence.max(message.sequence);
                    if !message.outgoing {
                        activity.incoming_sequences.push(message.sequence);
                    }
                }
                match page.next_before_sequence {
                    Some(next) if before.is_none_or(|old| next < old) => before = Some(next),
                    Some(_) => return Err(Error::Corrupt),
                    None => break,
                }
            }
            groups.push(activity);
        }
        Ok(LocalActivity {
            actionable_invitations,
            groups,
        })
    }
    /// The optional legacy enrollment has already been imported before this
    /// owner is constructed. It stays isolated in its original namespace.
    pub fn open(
        database: &'db Database,
        identity: IdentityHash,
        legacy: Option<Provisioned>,
    ) -> Result<Self, Error> {
        if legacy
            .as_ref()
            .is_some_and(|cfg| cfg.local_reticulum_identity != identity)
        {
            return Err(Error::Configuration);
        }
        Ok(Self {
            database,
            owner: NativeStoreOwner::new(database),
            enrollment: Enrollment::open(database, identity).map_err(enrollment_error)?,
            identity,
            legacy,
        })
    }
    pub fn execute(&mut self, job: Job) {
        let result = self.dispatch(job.group, job.control, job.command);
        Job::notify_commit(&job.wake, &result);
        let _ = job.reply.send(result);
    }
    fn configurations(&mut self) -> Result<Vec<Provisioned>, Error> {
        let mut result = Vec::new();
        if let Some(legacy) = &self.legacy {
            result.push(legacy.clone());
        }
        for group in self
            .enrollment
            .groups(&mut self.owner)
            .map_err(enrollment_error)?
        {
            result.push(
                self.enrollment
                    .provisioned(&mut self.owner, group)
                    .map_err(enrollment_error)?,
            );
        }
        Ok(result)
    }
    fn provisioned(&mut self, group: GroupId) -> Result<Provisioned, Error> {
        if let Some(cfg) = &self.legacy {
            if cfg.group == group {
                return Ok(cfg.clone());
            }
        }
        self.enrollment
            .provisioned(&mut self.owner, group)
            .map_err(enrollment_error)
    }
    fn history_messages(
        &mut self,
        group: prns_groups::GroupId,
        before: Option<u64>,
        limit: usize,
        clock: prns_groups::ClockReading,
    ) -> Result<Reply, Error> {
        if limit == 0 || limit > 100 {
            return Err(Error::InvalidInput);
        }
        let current = self.provisioned(group)?;
        let mut configs = self
            .enrollment
            .archived_configurations(&mut self.owner, group)
            .map_err(enrollment_error)?;
        configs.push(current);
        let mut messages = Vec::new();
        let mut revision = 0u64;
        let mut source_more = false;
        for cfg in configs {
            let base = u64::from(cfg.history_generation) << 32;
            if before.is_some_and(|before| before <= base) {
                let Reply::Conversations(items) = crate::session::execute_with_owner(
                    self.database,
                    &mut self.owner,
                    &cfg,
                    Command::Conversations,
                )?
                else {
                    return Err(Error::Corrupt);
                };
                revision = revision
                    .checked_add(items.first().ok_or(Error::Corrupt)?.revision)
                    .ok_or(Error::Corrupt)?;
                continue;
            }
            let local_before = before.and_then(|before| {
                let local = before - base;
                (local < (1u64 << 32)).then_some(local)
            });
            let Reply::Messages(page) = crate::session::execute_with_owner(
                self.database,
                &mut self.owner,
                &cfg,
                Command::Messages {
                    group,
                    before_sequence: local_before,
                    limit: 100,
                    clock,
                },
            )?
            else {
                return Err(Error::Corrupt);
            };
            revision = revision.checked_add(page.revision).ok_or(Error::Corrupt)?;
            source_more |= page.next_before_sequence.is_some();
            for mut message in page.messages {
                if message.sequence >= (1u64 << 32) {
                    return Err(Error::Corrupt);
                }
                message.sequence += base;
                messages.push(message);
            }
        }
        messages.sort_by_key(|message| std::cmp::Reverse(message.sequence));
        let more = messages.len() > limit || source_more;
        messages.truncate(limit);
        let next_before_sequence = if more {
            messages.last().map(|m| m.sequence)
        } else {
            None
        };
        Ok(Reply::Messages(crate::MessagePage {
            messages,
            revision,
            next_before_sequence,
        }))
    }
    fn dispatch(
        &mut self,
        scope: Option<GroupId>,
        control: Option<prns_groups::ControlReference>,
        command: Command,
    ) -> Result<Reply, Error> {
        match command {
            Command::Maintain(clock) => {
                if let prns_groups_service::ClockReading::Trusted(interval) = clock {
                    self.enrollment
                        .maintain(&mut self.owner, interval.lower().0)
                        .map_err(enrollment_error)?;
                }
                for cfg in self
                    .configurations()?
                    .into_iter()
                    .filter(|cfg| cfg.lifecycle == crate::GroupLifecycle::Active)
                {
                    crate::session::execute_with_owner(
                        self.database,
                        &mut self.owner,
                        &cfg,
                        Command::Maintain(clock),
                    )?;
                }
                Ok(Reply::Done)
            }
            Command::Identity => Ok(Reply::Identity(self.identity)),
            Command::Enrollment(request) => {
                use crate::{EnrollmentRequest as Request, EnrollmentResult as Result};
                let result = match request {
                    Request::CancelInvitation {
                        request_id,
                        invitation,
                        now,
                    } => {
                        self.enrollment
                            .cancel_invitation(&mut self.owner, request_id, invitation, now)
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                    Request::DismissInvitation {
                        request_id,
                        invitation,
                        now,
                    } => {
                        self.enrollment
                            .dismiss_invitation(&mut self.owner, request_id, invitation, now)
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                    Request::RemoveMember {
                        request_id,
                        group,
                        member,
                        expected_head,
                        now,
                    } => {
                        self.enrollment
                            .remove_member(
                                &mut self.owner,
                                request_id,
                                group,
                                member,
                                expected_head,
                                now,
                            )
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                    Request::Leave {
                        request_id,
                        group,
                        now,
                    } => {
                        self.enrollment
                            .leave(&mut self.owner, request_id, group, now)
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                    Request::Close {
                        request_id,
                        group,
                        expected_head,
                        now,
                    } => {
                        self.enrollment
                            .close(&mut self.owner, request_id, group, expected_head, now)
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                    Request::DeleteLocal {
                        request_id,
                        group,
                        now,
                    } => {
                        self.enrollment
                            .delete_local(&mut self.owner, request_id, group, now)
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                    Request::Create {
                        request_id,
                        title,
                        now,
                    } => Result::Created(
                        self.enrollment
                            .create(&mut self.owner, request_id, &title, now)
                            .map_err(enrollment_error)?,
                    ),
                    Request::Invite {
                        request_id,
                        group,
                        recipient,
                        now,
                    } => Result::Invited(
                        self.enrollment
                            .invite(&mut self.owner, request_id, group, recipient, now)
                            .map_err(enrollment_error)?,
                    ),
                    Request::Respond {
                        request_id,
                        invitation,
                        accept,
                        now,
                    } => {
                        self.enrollment
                            .respond(&mut self.owner, request_id, invitation, accept, now)
                            .map_err(enrollment_error)?;
                        Result::Saved
                    }
                };
                Ok(Reply::Enrollment(result))
            }
            Command::Invitations(now) => Ok(Reply::Invitations(
                self.enrollment
                    .invitations(&mut self.owner, now)
                    .map_err(enrollment_error)?,
            )),
            Command::EnrollmentWork(now) => Ok(Reply::EnrollmentWork(
                self.enrollment
                    .pending(&mut self.owner, now)
                    .map_err(enrollment_error)?,
            )),
            Command::EnrollmentHandle {
                requester,
                frame,
                now,
            } => Ok(Reply::EnrollmentFrame(
                self.enrollment
                    .handle(&mut self.owner, requester, &frame, now)
                    .map_err(enrollment_error)?,
            )),
            Command::EnrollmentComplete { id, response, now } => {
                let activity = self
                    .enrollment
                    .complete(&mut self.owner, id, &response, now)
                    .map_err(enrollment_error)?;
                Ok(Reply::EnrollmentCompleted(activity))
            }
            Command::Configs => {
                let mut configs = Vec::new();
                for cfg in self
                    .configurations()?
                    .into_iter()
                    .filter(|cfg| cfg.lifecycle == crate::GroupLifecycle::Active)
                {
                    configs.push(crate::session::configuration(&cfg)?);
                }
                Ok(Reply::Configs(configs))
            }
            Command::Conversations if scope.is_none() => {
                let mut conversations = Vec::new();
                for cfg in self.configurations()? {
                    let Reply::Conversations(mut items) = crate::session::execute_with_owner(
                        self.database,
                        &mut self.owner,
                        &cfg,
                        Command::Conversations,
                    )?
                    else {
                        return Err(Error::Corrupt);
                    };
                    conversations.append(&mut items);
                }
                Ok(Reply::Conversations(conversations))
            }
            Command::Messages {
                group,
                before_sequence,
                limit,
                clock,
            } => {
                if scope.is_some_and(|scope| scope != group) {
                    return Err(Error::Configuration);
                }
                if control.is_some_and(|control| {
                    self.provisioned(group)
                        .is_ok_and(|cfg| cfg.control != control)
                }) {
                    return Err(Error::Configuration);
                }
                self.history_messages(group, before_sequence, limit, clock)
            }
            command => {
                let group = match &command {
                    Command::Messages { group, .. }
                    | Command::Records { group, .. }
                    | Command::Record { group, .. }
                    | Command::Progress { group, .. }
                    | Command::Changes { group, .. } => Some(*group),
                    Command::Publish(request) => Some(request.group),
                    Command::SendText(request) => Some(request.group),
                    _ => scope,
                };
                if scope.is_some() && group != scope {
                    return Err(Error::Configuration);
                }
                let cfg = if let Some(group) = group {
                    self.provisioned(group)?
                } else {
                    let mut configs = self.configurations()?;
                    if configs.len() != 1 {
                        return Err(Error::NotProvisioned);
                    }
                    configs.remove(0)
                };
                if matches!(&command, Command::Publish(_) | Command::SendText(_)) {
                    if let Some(reply) = crate::session::retained_publication_with_owner(
                        self.database,
                        &mut self.owner,
                        &cfg,
                        &command,
                    )? {
                        return Ok(reply);
                    }
                    for archived in self
                        .enrollment
                        .archived_configurations(&mut self.owner, cfg.group)
                        .map_err(enrollment_error)?
                    {
                        if let Some(reply) = crate::session::retained_publication_with_owner(
                            self.database,
                            &mut self.owner,
                            &archived,
                            &command,
                        )? {
                            return Ok(reply);
                        }
                    }
                }
                if cfg.lifecycle != crate::GroupLifecycle::Active
                    && !matches!(
                        &command,
                        Command::Conversations
                            | Command::Records { .. }
                            | Command::Record { .. }
                            | Command::Progress { .. }
                            | Command::Changes { .. }
                    )
                {
                    return Err(Error::Enrollment(
                        if cfg.lifecycle == crate::GroupLifecycle::Closed {
                            crate::enrollment::EnrollmentError::GroupClosed
                        } else {
                            crate::enrollment::EnrollmentError::NotJoined
                        },
                    ));
                }
                if control.is_some_and(|head| head != cfg.control) {
                    return Err(Error::Configuration);
                }
                crate::session::execute_with_owner(self.database, &mut self.owner, &cfg, command)
            }
        }
    }
}
fn enrollment_error(error: crate::enrollment::EnrollmentError) -> Error {
    Error::Enrollment(error)
}
