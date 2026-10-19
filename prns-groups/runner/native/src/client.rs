use crate::{
    ChangeToken, Conversation, Error, MessagePage, NetworkConfig, PublishRecord, RecordChanges,
    RecordDeliveryProgress, RecordPage, SendText, StoreSession, StoredRecord,
};
use prns_groups_service::{ClockReading, GroupId, RecordId};
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};

/// Admission belongs to the existing database owner. Return Busy when its
/// bounded lane is full and Unavailable when that owner has stopped.
pub trait Submit: Send + Sync {
    fn submit(&self, job: Job) -> Result<(), Error>;
}

pub struct Job {
    pub(crate) command: Command,
    pub(crate) group: Option<GroupId>,
    pub(crate) control: Option<prns_groups::ControlReference>,
    pub(crate) reply: oneshot::Sender<Result<Reply, Error>>,
    pub(crate) wake: Arc<Notify>,
}
impl Job {
    /// Narrow lifecycle-owner inspection. Configurations/conversations may
    /// finish recovery of already admitted durable work; this is deliberately
    /// not a promise of read-only storage access. Only the exclusive lifecycle
    /// owner may use this path after fencing ordinary client admissions.
    pub fn is_lifecycle_inspection(&self) -> bool {
        matches!(
            &self.command,
            Command::Identity
                | Command::Configs
                | Command::Conversations
                | Command::Invitations(_)
                | Command::EnrollmentWork(_)
        )
    }

    /// An admitted mutation commits even when its caller has dropped the reply.
    pub fn execute(self, session: &mut StoreSession<'_>) {
        let result = if self.group.is_some_and(|group| group != session.group())
            || self
                .control
                .is_some_and(|control| control != session.control())
        {
            Err(Error::Configuration)
        } else {
            session.execute(self.command)
        };
        Self::notify_commit(&self.wake, &result);
        let _ = self.reply.send(result);
    }
    pub(crate) fn notify_commit(wake: &Notify, result: &Result<Reply, Error>) {
        // The storage owner, rather than the reply waiter, owns this signal.
        // Cancellation after admission cannot strand an already committed send.
        if matches!(result, Ok(Reply::Sent(_))) {
            wake.notify_one();
        }
    }
    pub fn fail(self, error: Error) {
        let _ = self.reply.send(Err(error));
    }
}

#[derive(Clone)]
pub struct Client {
    submitter: Arc<dyn Submit>,
    pub(crate) wake: Arc<Notify>,
    group: Option<GroupId>,
    control: Option<prns_groups::ControlReference>,
}
impl Client {
    pub fn new(submitter: Arc<dyn Submit>) -> Self {
        Self {
            submitter,
            wake: Arc::new(Notify::new()),
            group: None,
            control: None,
        }
    }
    pub fn for_group(&self, group: GroupId) -> Self {
        Self {
            group: Some(group),
            control: None,
            ..self.clone()
        }
    }
    /// Fence asynchronous sync work to the membership loaded at its start.
    pub fn at_control(&self, control: prns_groups::ControlReference) -> Self {
        Self {
            control: Some(control),
            ..self.clone()
        }
    }
    pub fn wake(&self) {
        self.wake.notify_one();
    }
    pub(crate) async fn call(&self, command: Command) -> Result<Reply, Error> {
        let (reply, receiver) = oneshot::channel();
        self.submitter.submit(Job {
            command,
            reply,
            group: self.group,
            control: self.control,
            wake: Arc::clone(&self.wake),
        })?;
        receiver.await.map_err(|_| Error::Unavailable)?
    }
    pub async fn network_config(&self) -> Result<NetworkConfig, Error> {
        match self.call(Command::Config).await? {
            Reply::Config(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn conversations(&self) -> Result<Vec<Conversation>, Error> {
        match self.call(Command::Conversations).await? {
            Reply::Conversations(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn messages(
        &self,
        group: GroupId,
        before_sequence: Option<u64>,
        limit: usize,
        clock: ClockReading,
    ) -> Result<MessagePage, Error> {
        if limit == 0 || limit > 100 {
            return Err(Error::InvalidInput);
        }
        match self
            .call(Command::Messages {
                group,
                before_sequence,
                limit,
                clock,
            })
            .await?
        {
            Reply::Messages(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    /// Submit synchronously while the application still holds its lifecycle
    /// admission guard. The returned waiter does not borrow that guard, and
    /// dropping it never cancels a mutation already admitted to the lane.
    pub fn admit_send_text(
        &self,
        request: SendText,
    ) -> Result<impl std::future::Future<Output = Result<RecordId, Error>> + Send + 'static, Error>
    {
        prns_group_text::Text::new(&request.text).map_err(|_| Error::InvalidInput)?;
        self.admit_publication(request.group, Command::SendText(request))
    }
    pub async fn send_text(&self, request: SendText) -> Result<RecordId, Error> {
        self.admit_send_text(request)?.await
    }
    /// Admit one generic publication on the caller-owned durable lane. Dropping
    /// the returned waiter does not cancel accepted work; retry the same ID/input.
    pub fn admit_publish(
        &self,
        request: PublishRecord,
    ) -> Result<impl std::future::Future<Output = Result<RecordId, Error>> + Send + 'static, Error>
    {
        if request.body.len() > crate::MAX_PUBLICATION_BODY_BYTES
            || request.targets.len() > crate::MAX_PUBLICATION_TARGETS
            || request.targets.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::InvalidInput);
        }
        if request.targets.is_empty() {
            return Err(Error::NoRecipients);
        }
        self.admit_publication(request.group, Command::Publish(request))
    }
    fn admit_publication(
        &self,
        group: GroupId,
        command: Command,
    ) -> Result<impl std::future::Future<Output = Result<RecordId, Error>> + Send + 'static, Error>
    {
        if self.group.is_some_and(|bound| bound != group) {
            return Err(Error::Configuration);
        }
        let (reply, receiver) = oneshot::channel();
        self.submitter.submit(Job {
            group: Some(group),
            control: self.control,
            command,
            wake: Arc::clone(&self.wake),
            reply,
        })?;
        Ok(async move {
            match receiver.await.map_err(|_| Error::Unavailable)?? {
                Reply::Sent(value) => Ok(value),
                _ => Err(Error::Corrupt),
            }
        })
    }
    pub async fn publish(&self, request: PublishRecord) -> Result<RecordId, Error> {
        self.admit_publish(request)?.await
    }
    /// Newest retained records first, across all application namespaces.
    pub async fn records(
        &self,
        group: GroupId,
        before_sequence: Option<u64>,
        limit: usize,
        clock: ClockReading,
    ) -> Result<RecordPage, Error> {
        if limit == 0 || limit > 100 {
            return Err(Error::InvalidInput);
        }
        match self
            .call(Command::Records {
                group,
                before_sequence,
                limit,
                clock,
            })
            .await?
        {
            Reply::Records(page) => Ok(page),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn record(
        &self,
        group: GroupId,
        record: RecordId,
        clock: ClockReading,
    ) -> Result<Option<StoredRecord>, Error> {
        match self
            .call(Command::Record {
                group,
                record,
                clock,
            })
            .await?
        {
            Reply::Record(record) => Ok(record),
            _ => Err(Error::Corrupt),
        }
    }
    /// Device receipt evidence is reconstructed from committed state.
    pub async fn delivery_progress(
        &self,
        group: GroupId,
        record: RecordId,
        clock: ClockReading,
    ) -> Result<Option<RecordDeliveryProgress>, Error> {
        match self
            .call(Command::Progress {
                group,
                record,
                clock,
            })
            .await?
        {
            Reply::Progress(progress) => Ok(progress),
            _ => Err(Error::Corrupt),
        }
    }
    /// Read durable changes after a saved cursor. Hints/notifications are not
    /// authoritative, and `history_pruned` requires rebuilding retained state.
    pub async fn changes(
        &self,
        group: GroupId,
        after: Option<ChangeToken>,
        limit: usize,
    ) -> Result<RecordChanges, Error> {
        if limit == 0 || limit > 100 {
            return Err(Error::InvalidInput);
        }
        match self
            .call(Command::Changes {
                group,
                after,
                limit,
            })
            .await?
        {
            Reply::Changes(changes) => Ok(changes),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn identity(&self) -> Result<prns_host::IdentityHash, Error> {
        match self.call(Command::Identity).await? {
            Reply::Identity(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn network_configs(&self) -> Result<Vec<NetworkConfig>, Error> {
        match self.call(Command::Configs).await? {
            Reply::Configs(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub fn admit_enrollment(
        &self,
        request: crate::EnrollmentRequest,
    ) -> Result<
        impl std::future::Future<Output = Result<crate::EnrollmentResult, Error>> + Send + 'static,
        Error,
    > {
        let (reply, receiver) = oneshot::channel();
        self.submitter.submit(Job {
            command: Command::Enrollment(request),
            wake: Arc::clone(&self.wake),
            group: None,
            control: None,
            reply,
        })?;
        let wake = Arc::clone(&self.wake);
        wake.notify_one();
        Ok(async move {
            let reply = receiver.await.map_err(|_| Error::Unavailable)??;
            wake.notify_one();
            match reply {
                Reply::Enrollment(value) => Ok(value),
                _ => Err(Error::Corrupt),
            }
        })
    }
    pub async fn invitations(&self, now: u64) -> Result<Vec<crate::enrollment::Invitation>, Error> {
        match self.call(Command::Invitations(now)).await? {
            Reply::Invitations(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn enrollment_work(&self, now: u64) -> Result<Vec<crate::enrollment::Work>, Error> {
        match self.call(Command::EnrollmentWork(now)).await? {
            Reply::EnrollmentWork(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn enrollment_handle(
        &self,
        requester: prns_host::IdentityHash,
        frame: Vec<u8>,
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        match self
            .call(Command::EnrollmentHandle {
                requester,
                frame,
                now,
            })
            .await?
        {
            Reply::EnrollmentFrame(value) => Ok(value),
            _ => Err(Error::Corrupt),
        }
    }
    pub async fn enrollment_complete(
        &self,
        id: [u8; 32],
        response: Vec<u8>,
        now: u64,
    ) -> Result<crate::enrollment::CompletionActivity, Error> {
        match self
            .call(Command::EnrollmentComplete { id, response, now })
            .await?
        {
            Reply::EnrollmentCompleted(activity) => {
                if activity == crate::enrollment::CompletionActivity::Progress {
                    self.wake();
                }
                Ok(activity)
            }
            _ => Err(Error::Corrupt),
        }
    }
    /// Run one bounded maintenance pass on the owned storage lane.
    pub async fn maintain(&self, clock: ClockReading) -> Result<(), Error> {
        match self.call(Command::Maintain(clock)).await? {
            Reply::Done => Ok(()),
            _ => Err(Error::Corrupt),
        }
    }
}

pub(crate) enum Command {
    Identity,
    Configs,
    Enrollment(crate::EnrollmentRequest),
    Invitations(u64),
    EnrollmentWork(u64),
    EnrollmentHandle {
        requester: prns_host::IdentityHash,
        frame: Vec<u8>,
        now: u64,
    },
    EnrollmentComplete {
        id: [u8; 32],
        response: Vec<u8>,
        now: u64,
    },
    Config,
    Conversations,
    Messages {
        group: GroupId,
        before_sequence: Option<u64>,
        limit: usize,
        clock: ClockReading,
    },
    SendText(SendText),
    Publish(PublishRecord),
    Records {
        group: GroupId,
        before_sequence: Option<u64>,
        limit: usize,
        clock: ClockReading,
    },
    Record {
        group: GroupId,
        record: RecordId,
        clock: ClockReading,
    },
    Progress {
        group: GroupId,
        record: RecordId,
        clock: ClockReading,
    },
    Changes {
        group: GroupId,
        after: Option<ChangeToken>,
        limit: usize,
    },
    Maintain(ClockReading),
}
pub(crate) enum Reply {
    Identity(prns_host::IdentityHash),
    Configs(Vec<NetworkConfig>),
    Enrollment(crate::EnrollmentResult),
    Invitations(Vec<crate::enrollment::Invitation>),
    EnrollmentWork(Vec<crate::enrollment::Work>),
    EnrollmentFrame(Vec<u8>),
    EnrollmentCompleted(crate::enrollment::CompletionActivity),
    Config(NetworkConfig),
    Conversations(Vec<Conversation>),
    Messages(MessagePage),
    Sent(RecordId),
    Records(RecordPage),
    Record(Option<StoredRecord>),
    Progress(Option<RecordDeliveryProgress>),
    Changes(RecordChanges),
    Done,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::enrollment::CompletionActivity;
    use futures_util::FutureExt;

    struct DeferredOwner(tokio::sync::mpsc::UnboundedSender<Job>);
    impl Submit for DeferredOwner {
        fn submit(&self, job: Job) -> Result<(), Error> {
            self.0.send(job).map_err(|_| Error::Unavailable)
        }
    }

    #[tokio::test]
    async fn admitted_send_wakes_after_commit_even_if_the_caller_drops_the_waiter() {
        let now = prns_groups_service::Timestamp(1_792_000_000);
        let clock =
            ClockReading::Trusted(prns_groups_service::ClockInterval::new(now, now).unwrap());
        let identities = [
            prns_host::IdentityHash::new([1; 16]),
            prns_host::IdentityHash::new([2; 16]),
        ];
        let fixtures = crate::fixture::generate("Commit wake", &identities, now).unwrap();
        // Both public owner paths must signal: standalone and the app manager.
        for managed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let db = redb::Database::create(directory.path().join("wake.redb")).unwrap();
            let config =
                crate::fixture::load_and_install(&db, &fixtures[0], identities[0]).unwrap();
            let (sender, mut jobs) = tokio::sync::mpsc::unbounded_channel();
            let client = Client::new(Arc::new(DeferredOwner(sender)));
            let waiter = client
                .admit_send_text(SendText {
                    group: config.group,
                    client_request_id: crate::RequestToken::new(now.0, [7; 20])
                        .unwrap()
                        .into_bytes(),
                    text: "A cancelled waiter must not strand this send".into(),
                    clock,
                })
                .unwrap();
            drop(waiter);
            assert!(client.wake.notified().now_or_never().is_none());
            let job = jobs.recv().await.unwrap();
            if managed {
                let mut owner =
                    crate::GroupManager::open(&db, identities[0], Some(config)).unwrap();
                owner.execute(job);
            } else {
                let mut session = StoreSession::open(&db, config).unwrap();
                job.execute(&mut session);
            }
            assert!(client.wake.notified().now_or_never().is_some());
            assert!(client.wake.notified().now_or_never().is_none());
        }
    }

    struct CompletionOwner(CompletionActivity);
    impl Submit for CompletionOwner {
        fn submit(&self, job: Job) -> Result<(), Error> {
            if !matches!(job.command, Command::EnrollmentComplete { .. }) {
                job.fail(Error::InvalidInput);
                return Ok(());
            }
            let _ = job.reply.send(Ok(Reply::EnrollmentCompleted(self.0)));
            Ok(())
        }
    }

    #[tokio::test]
    async fn only_committed_enrollment_progress_wakes_content_scheduling() {
        for activity in [CompletionActivity::Idle, CompletionActivity::Progress] {
            let client = Client::new(Arc::new(CompletionOwner(activity)));
            // The client consumes only a typed storage result, not a network
            // reply tag; correlation/validation remains on the enrollment lane.
            assert_eq!(
                client
                    .enrollment_complete([1; 32], vec![0], 1_800_000_000)
                    .await
                    .unwrap(),
                activity
            );
            assert_eq!(
                client.wake.notified().now_or_never().is_some(),
                activity == CompletionActivity::Progress
            );
        }
    }
}
