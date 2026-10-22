use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::thread::JoinHandle;

use prns_lxmf::mailbox::{
    execute_mailbox_request, initialize_mailbox_tables, validate_mailbox_database, MailboxFailure,
    MailboxFuture, MailboxReply, MailboxRequest, MailboxSubmitter,
};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use tokio::sync::oneshot;

use crate::directory::{self, DirectoryRequest, DirectoryResponse};
use prns_core::identity::{vault::IdentitySecretKey, PrivateIdentityMaterial};
use prns_groups_runner_native::{
    Client as GroupClient, Error as GroupError, Job as GroupJob, Submit,
};
use prns_identities_native::scoped_store::{ScopedDatabase, StorageScope};

pub(crate) const DEVELOPMENT_META: TableDefinition<&str, u64> =
    TableDefinition::new("development_meta");
pub(crate) const CONTACTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("contacts");
pub(crate) const APPLICATION_SETTINGS: TableDefinition<&str, &str> =
    TableDefinition::new("application_settings");
const MESSAGING_NAME_KEY: &str = "messaging_name";

const DEVELOPMENT_FORMAT_KEY: &str = "format";
const DEVELOPMENT_FORMAT: u64 = 2;
const BLUETOOTH_ENABLED_KEY: &str = "bluetooth_enabled";
const STORE_LANE_CAPACITY: usize = 8;
const RECEIVE_STATE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("identity_receive_state_v1");
const DEPENDENCY_REVISION: TableDefinition<u8, &[u8]> =
    TableDefinition::new("identity_dependency_revision_v1");
pub(crate) const MAX_RECEIVE_STATE_BYTES: usize = 256 * 1024;

pub(crate) type StoreReply = Result<DirectoryResponse, DevelopmentStoreFailure>;
pub(crate) type MailboxStoreReply = Result<MailboxReply, MailboxFailure>;
pub(crate) type ReceiveStateReply =
    Result<Option<zeroize::Zeroizing<Vec<u8>>>, DevelopmentStoreFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DevelopmentStoreFailure {
    Unavailable(String),
    ResetRequired(String),
}

impl DevelopmentStoreFailure {
    pub(crate) fn unavailable(detail: impl Into<String>) -> Self {
        Self::Unavailable(detail.into())
    }

    pub(crate) fn reset_required(reason: impl Into<String>) -> Self {
        Self::ResetRequired(reason.into())
    }
}

enum StoreJob {
    Connections {
        input: Option<crate::contract::ConnectionSettingsUpdateInput>,
        completion: Box<
            dyn FnOnce(
                    Result<crate::connections::StoredSettings, crate::connections::SettingsError>,
                ) + Send,
        >,
    },
    Activity {
        operation: crate::named_identities::activity::Operation,
        response: oneshot::Sender<Result<crate::named_identities::activity::Reply, String>>,
    },
    DependencyRevision {
        fingerprint: [u8; 32],
        response: oneshot::Sender<Result<u64, DevelopmentStoreFailure>>,
    },
    DrainAsync(oneshot::Sender<()>),
    ReceiveState {
        destination: [u8; 16],
        state: Option<zeroize::Zeroizing<Vec<u8>>>,
        response: oneshot::Sender<ReceiveStateReply>,
    },
    ContactCard {
        root: PathBuf,
        request: crate::contact_cards::Request,
        response: oneshot::Sender<crate::contact_cards::Reply>,
    },
    BluetoothDiscovery {
        groups: Option<prns_core::interfaces::DiscoveryGroupSet>,
        completion: Box<
            dyn FnOnce(
                    Result<
                        prns_core::interfaces::DiscoveryGroupSet,
                        crate::bluetooth_discovery::SettingsError,
                    >,
                ) + Send,
        >,
    },
    Drain(SyncSender<()>),
    InitializeGroups {
        identity: prns_host::IdentityHash,
        response: SyncSender<Result<(), DevelopmentStoreFailure>>,
    },
    Group(GroupJob),
    GroupAction {
        request: crate::group_actions::Request,
        response: oneshot::Sender<Result<crate::group_actions::Reply, crate::group_actions::Error>>,
    },
    GroupAnnounces {
        settings: Option<crate::contract::GroupAnnounceSettings>,
        completion: Box<
            dyn FnOnce(Result<crate::contract::GroupAnnounceSettings, DevelopmentStoreFailure>)
                + Send,
        >,
    },
    MessagingName {
        name: Option<String>,
        completion: Box<dyn FnOnce(Result<String, DevelopmentStoreFailure>) + Send>,
    },
    Bluetooth {
        enabled: Option<bool>,
        completion: Box<dyn FnOnce(Result<bool, DevelopmentStoreFailure>) + Send>,
    },
    DirectoryAsync {
        request: DirectoryRequest,
        response: oneshot::Sender<StoreReply>,
    },
    MailboxAsync {
        request: MailboxRequest,
        response: oneshot::Sender<MailboxStoreReply>,
    },
    #[cfg(test)]
    Barrier {
        entered: SyncSender<()>,
        release: Receiver<()>,
    },
}

struct StoreLane {
    owner_id: u64,
    bluetooth_changing: std::sync::atomic::AtomicBool,
    jobs: StdMutex<Option<SyncSender<StoreJob>>>,
    group_action_executing: std::sync::atomic::AtomicBool,
    group_identity: StdMutex<Option<[u8; 16]>>,
    admission: std::sync::atomic::AtomicU8,
}

enum StoreAdmissionError {
    Full,
    Disconnected,
}

impl StoreLane {
    fn admit_default(&self, job: StoreJob) -> Result<(), StoreAdmissionError> {
        self.admit(job, false)
    }
    /// Check and enqueue under the same lock as lifecycle fencing. An admission
    /// cannot race past a fence by retaining a sender clone.
    fn admit(&self, job: StoreJob, inspection: bool) -> Result<(), StoreAdmissionError> {
        if inspection && !matches!(&job, StoreJob::Group(job) if job.is_lifecycle_inspection()) {
            return Err(StoreAdmissionError::Disconnected);
        }
        let jobs = self
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let phase = self.admission.load(std::sync::atomic::Ordering::Relaxed);
        let allowed = phase == 0
            || match &job {
            StoreJob::Group(job) => inspection && job.is_lifecycle_inspection(),
            StoreJob::MailboxAsync { request, .. } => match request {
                MailboxRequest::List(_) | MailboxRequest::ListConversations(_) | MailboxRequest::ListQueued => true,
                // Accepted network callbacks and send attempts drain before the
                // final fence; new application sends remain rejected here.
                MailboxRequest::InsertInbound(_) | MailboxRequest::CompleteAttempt { .. } => phase == 1,
                _ => false,
            },
            StoreJob::GroupAction { request, .. } => match request {
                crate::group_actions::Request::List | crate::group_actions::Request::Read(_) => true,
                crate::group_actions::Request::Finish { .. } => phase == 1,
                _ => false,
            },
            StoreJob::DirectoryAsync { request: DirectoryRequest::Get { .. } | DirectoryRequest::List, .. }
            | StoreJob::MessagingName { name: None, .. }
            | StoreJob::GroupAnnounces { settings: None, .. }
            | StoreJob::Bluetooth { enabled: None, .. }
            | StoreJob::BluetoothDiscovery { groups: None, .. }
            | StoreJob::Connections { input: None, .. }
            | StoreJob::Drain(_)
            | StoreJob::DrainAsync(_)
            // This private owner method is used by the scoped teardown driver,
            // not exposed through ordinary application or network clients.
            | StoreJob::ReceiveState { .. }
            | StoreJob::DependencyRevision { .. } => true,
            _ => false,
        };
        let Some(jobs) = jobs.as_ref().filter(|_| allowed) else {
            return Err(StoreAdmissionError::Disconnected);
        };
        jobs.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => StoreAdmissionError::Full,
            TrySendError::Disconnected(_) => StoreAdmissionError::Disconnected,
        })
    }

    fn fence(&self, phase: u8) {
        let _jobs = self
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.admission
            .fetch_max(phase, std::sync::atomic::Ordering::Relaxed);
    }
}

static NEXT_OWNER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) struct BluetoothChange(Arc<StoreLane>);
impl Drop for BluetoothChange {
    fn drop(&mut self) {
        self.0
            .bluetooth_changing
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

struct DevelopmentGroupSubmitter {
    lane: Weak<StoreLane>,
    inspection: bool,
}

#[derive(Clone)]
pub(crate) struct GroupActionClient {
    lane: Weak<StoreLane>,
}

pub(crate) struct GroupActionExecution(Arc<StoreLane>);

impl Drop for GroupActionExecution {
    fn drop(&mut self) {
        self.0
            .group_action_executing
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

impl GroupActionClient {
    pub(crate) fn identity(&self) -> Result<[u8; 16], crate::group_actions::Error> {
        let unavailable = || {
            crate::group_actions::Error::Unavailable("The local group identity is not open.".into())
        };
        let lane = self.lane.upgrade().ok_or_else(unavailable)?;
        let identity = *lane
            .group_identity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        identity.ok_or_else(unavailable)
    }
    /// Only one UI recovery execution may observe and settle a Pending action
    /// at once. Native accepted jobs still finish after cancellation; any later
    /// journal read is ordered after their admitted writes on this same lane.
    pub(crate) fn start_execution(
        &self,
    ) -> Result<GroupActionExecution, crate::group_actions::Error> {
        let lane = self.lane.upgrade().ok_or_else(|| {
            crate::group_actions::Error::Unavailable("The group action store has closed.".into())
        })?;
        lane.group_action_executing
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Acquire,
                std::sync::atomic::Ordering::Relaxed,
            )
            .map_err(|_| crate::group_actions::Error::Busy)?;
        Ok(GroupActionExecution(lane))
    }
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        self.lane.ptr_eq(&other.lane)
    }

    pub(crate) fn admit(
        &self,
        request: crate::group_actions::Request,
    ) -> Result<
        impl std::future::Future<
                Output = Result<crate::group_actions::Reply, crate::group_actions::Error>,
            > + Send
            + 'static,
        crate::group_actions::Error,
    > {
        use crate::group_actions::Error;
        let unavailable = || Error::Unavailable("The group action store has closed.".into());
        let lane = self.lane.upgrade().ok_or_else(unavailable)?;
        let (response, receiver) = oneshot::channel();
        lane.admit(StoreJob::GroupAction { request, response }, false)
            .map_err(|error| match error {
                StoreAdmissionError::Full => Error::Busy,
                StoreAdmissionError::Disconnected => unavailable(),
            })?;
        Ok(async move { receiver.await.map_err(|_| unavailable())? })
    }

    pub(crate) async fn execute(
        &self,
        request: crate::group_actions::Request,
    ) -> Result<crate::group_actions::Reply, crate::group_actions::Error> {
        self.admit(request)?.await
    }
}

impl Submit for DevelopmentGroupSubmitter {
    fn submit(&self, job: GroupJob) -> Result<(), GroupError> {
        let lane = self.lane.upgrade().ok_or(GroupError::Unavailable)?;
        lane.admit(StoreJob::Group(job), self.inspection)
            .map_err(|error| match error {
                StoreAdmissionError::Full => GroupError::Busy,
                StoreAdmissionError::Disconnected => GroupError::Unavailable,
            })
    }
}

#[derive(Clone)]
pub(crate) struct DevelopmentMailboxSubmitter {
    lane: Weak<StoreLane>,
}

impl MailboxSubmitter for DevelopmentMailboxSubmitter {
    fn submit(&self, request: MailboxRequest) -> MailboxFuture<'_> {
        let submitter = self.clone();
        Box::pin(async move {
            let Some(lane) = submitter.lane.upgrade() else {
                return Err(MailboxFailure::Unavailable(
                    "the development database owner has closed".to_owned(),
                ));
            };
            let (response, receiver) = oneshot::channel();
            match lane.admit(StoreJob::MailboxAsync { request, response }, false) {
                Ok(()) => receiver.await.map_err(|_| {
                    MailboxFailure::Unavailable(
                        "the development database owner stopped before replying".to_owned(),
                    )
                })?,
                Err(StoreAdmissionError::Full) => Err(MailboxFailure::Busy),
                Err(StoreAdmissionError::Disconnected) => Err(MailboxFailure::Unavailable(
                    "the development database owner has stopped".to_owned(),
                )),
            }
        })
    }
}

pub(crate) struct DevelopmentStoreOwner {
    pub(crate) root: PathBuf,
    scope: Option<StorageScope>,
    lane: Arc<StoreLane>,
    group_client: GroupClient,
    group_health: crate::groups::GroupHealth,
    join: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy)]
pub(crate) enum ScopedOpen {
    Create,
    Existing,
}

impl DevelopmentStoreOwner {
    pub(crate) fn admit_connections(
        &self,
        input: Option<crate::contract::ConnectionSettingsUpdateInput>,
        completion: impl FnOnce(Result<crate::connections::StoredSettings, crate::connections::SettingsError>)
            + Send
            + 'static,
    ) -> Result<(), DevelopmentStoreFailure> {
        self.require_installation_owner()?;
        self.lane
            .admit_default(StoreJob::Connections {
                input,
                completion: Box::new(completion),
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The connection settings lane is full or closed.",
                )
            })
    }

    pub(crate) fn read_connections_blocking(
        &self,
    ) -> Result<crate::connections::StoredSettings, crate::connections::SettingsError> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.admit_connections(None, move |result| {
            let _ = reply.send(result);
        })
        .map_err(crate::connections::SettingsError::Storage)?;
        receive.recv().map_err(|_| {
            crate::connections::SettingsError::Storage(DevelopmentStoreFailure::unavailable(
                "Connection settings owner stopped.",
            ))
        })?
    }

    pub(crate) fn admit_activity(
        &self,
        operation: crate::named_identities::activity::Operation,
    ) -> Result<
        oneshot::Receiver<Result<crate::named_identities::activity::Reply, String>>,
        DevelopmentStoreFailure,
    > {
        if self.scope.is_none() {
            return Err(DevelopmentStoreFailure::unavailable(
                "Activity requires an explicit identity owner.",
            ));
        }
        let (response, receive) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::Activity {
                operation,
                response,
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable("Activity storage is busy or closed.")
            })?;
        Ok(receive)
    }
    #[cfg(test)]
    pub(crate) fn scope(&self) -> Option<StorageScope> {
        self.scope
    }

    pub(crate) fn fence_fresh_mutations(&self) {
        self.lane.fence(1);
    }

    /// Call after service workers have joined, before final dependency reads.
    /// Exact completion writes are now fenced too; already queued jobs drain.
    pub(crate) fn fence_all_mutations(&self) {
        self.lane.fence(2);
    }

    /// Exclusive lifecycle-owner inspection only. It can recover old accepted
    /// group work, so keep this client private and await it before final report.
    pub(crate) fn inspection_group_client(&self) -> GroupClient {
        GroupClient::new(Arc::new(DevelopmentGroupSubmitter {
            lane: Arc::downgrade(&self.lane),
            inspection: true,
        }))
    }

    /// A barrier through every previously admitted store job. The caller must
    /// close fresh lifecycle admission before using this as shutdown evidence.
    pub(crate) async fn drain(&self) -> Result<(), DevelopmentStoreFailure> {
        let (response, receiver) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::DrainAsync(response))
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable("The storage drain lane is full or closed.")
            })?;
        receiver.await.map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "The storage owner stopped before its drain barrier.",
            )
        })
    }

    /// Opaque destination receive-key checkpoint in this identity's database.
    /// The service adapter validates the destination and codec before calling;
    /// it must save successfully before unregistering/untracking live ratchets.
    pub(crate) fn admit_receive_state(
        &self,
        destination: [u8; 16],
        state: Option<zeroize::Zeroizing<Vec<u8>>>,
    ) -> Result<oneshot::Receiver<ReceiveStateReply>, DevelopmentStoreFailure> {
        if self.scope.is_none() {
            return Err(DevelopmentStoreFailure::unavailable(
                "Receive-state checkpoints require an explicit local identity scope.",
            ));
        }
        if state
            .as_ref()
            .is_some_and(|bytes| bytes.is_empty() || bytes.len() > MAX_RECEIVE_STATE_BYTES)
        {
            return Err(DevelopmentStoreFailure::unavailable(
                "Receive-state checkpoint exceeds its byte bounds.",
            ));
        }
        let (response, receiver) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::ReceiveState {
                destination,
                state,
                response,
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The receive-state storage lane is full or closed.",
                )
            })?;
        Ok(receiver)
    }

    /// Publish the revision of a stable dependency observation. Only the owner
    /// lifecycle adapter may call this; ordinary clients cannot mint evidence.
    /// Observe all facts after fencing/draining before supplying the digest.
    pub(crate) fn admit_dependency_revision(
        &self,
        fingerprint: [u8; 32],
    ) -> Result<oneshot::Receiver<Result<u64, DevelopmentStoreFailure>>, DevelopmentStoreFailure>
    {
        if self.scope.is_none() {
            return Err(DevelopmentStoreFailure::unavailable(
                "Dependency evidence requires an explicit local identity scope.",
            ));
        }
        let (response, receiver) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::DependencyRevision {
                fingerprint,
                response,
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The dependency observation lane is full or closed.",
                )
            })?;
        Ok(receiver)
    }

    pub(crate) fn owner_id(&self) -> u64 {
        self.lane.owner_id
    }

    pub(crate) fn start_bluetooth_change(&self) -> Option<BluetoothChange> {
        self.lane
            .bluetooth_changing
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Acquire,
                std::sync::atomic::Ordering::Relaxed,
            )
            .ok()?;
        Some(BluetoothChange(Arc::clone(&self.lane)))
    }

    pub(crate) fn admit_bluetooth_discovery(
        &self,
        groups: Option<prns_core::interfaces::DiscoveryGroupSet>,
        completion: impl FnOnce(
                Result<
                    prns_core::interfaces::DiscoveryGroupSet,
                    crate::bluetooth_discovery::SettingsError,
                >,
            ) + Send
            + 'static,
    ) -> Result<(), DevelopmentStoreFailure> {
        self.require_installation_owner()?;
        self.lane
            .admit_default(StoreJob::BluetoothDiscovery {
                groups,
                completion: Box::new(completion),
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The Bluetooth settings lane is full or closed.",
                )
            })
    }

    pub(crate) fn read_bluetooth_discovery_blocking(
        &self,
    ) -> Result<prns_core::interfaces::DiscoveryGroupSet, DevelopmentStoreFailure> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.admit_bluetooth_discovery(None, move |result| {
            let _ = reply.send(result);
        })?;
        receive
            .recv()
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable("Bluetooth discovery settings owner stopped.")
            })?
            .map_err(crate::bluetooth_discovery::SettingsError::into_store_failure)
    }

    pub(crate) fn drain_blocking(&self) -> Result<(), DevelopmentStoreFailure> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.jobs()?.send(StoreJob::Drain(reply)).map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "The development database owner stopped before draining admitted work.",
            )
        })?;
        receiver.recv().map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "The development database owner stopped while draining admitted work.",
            )
        })
    }

    pub(crate) fn group_client(&self) -> GroupClient {
        self.group_client.clone()
    }

    pub(crate) fn group_action_client(&self) -> GroupActionClient {
        GroupActionClient {
            lane: Arc::downgrade(&self.lane),
        }
    }

    pub(crate) fn group_health(&self) -> crate::groups::GroupHealth {
        self.group_health.clone()
    }

    /// Only the native lifecycle queue may wait for fixture installation.
    pub(crate) fn initialize_groups_blocking(
        &self,
        identity: prns_host::IdentityHash,
    ) -> Result<(), DevelopmentStoreFailure> {
        let (response, receiver) = mpsc::sync_channel(1);
        self.lane
            .admit_default(StoreJob::InitializeGroups { identity, response })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The database lane cannot admit group initialization.",
                )
            })?;
        receiver.recv().map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "The group storage owner stopped before initialization.",
            )
        })??;
        *self
            .lane
            .group_identity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(*identity.as_bytes());
        Ok(())
    }

    pub(crate) fn admit_group_announces(
        &self,
        settings: Option<crate::contract::GroupAnnounceSettings>,
        completion: impl FnOnce(Result<crate::contract::GroupAnnounceSettings, DevelopmentStoreFailure>)
            + Send
            + 'static,
    ) -> Result<(), DevelopmentStoreFailure> {
        let live = self.group_health.announces();
        self.lane
            .admit_default(StoreJob::GroupAnnounces {
                settings,
                completion: Box::new(move |result| {
                    if let Ok(settings) = &result {
                        live.update(settings.clone());
                    }
                    completion(result);
                }),
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The group announcement settings lane is full or closed.",
                )
            })
    }

    pub(crate) fn admit_messaging_name(
        &self,
        name: Option<String>,
        completion: impl FnOnce(Result<String, DevelopmentStoreFailure>) + Send + 'static,
    ) -> Result<(), DevelopmentStoreFailure> {
        self.lane
            .admit_default(StoreJob::MessagingName {
                name,
                completion: Box::new(completion),
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "the development database lane is full or closed",
                )
            })
    }

    pub(crate) fn read_messaging_name_blocking(&self) -> Result<String, DevelopmentStoreFailure> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.admit_messaging_name(None, move |result| {
            let _ = reply.send(result);
        })?;
        receive.recv().map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "the development database owner stopped before reading the messaging name",
            )
        })?
    }

    pub(crate) fn admit_bluetooth(
        &self,
        enabled: Option<bool>,
        completion: impl FnOnce(Result<bool, DevelopmentStoreFailure>) + Send + 'static,
    ) -> Result<(), DevelopmentStoreFailure> {
        self.require_installation_owner()?;
        self.lane
            .admit_default(StoreJob::Bluetooth {
                enabled,
                completion: Box::new(completion),
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "the development database lane is full or closed",
                )
            })
    }

    /// Native lifecycle queue only; startup must observe prior admitted preference commits.
    pub(crate) fn read_bluetooth_blocking(&self) -> Result<bool, DevelopmentStoreFailure> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.admit_bluetooth(None, move |result| {
            let _ = reply.send(result);
        })?;
        receive.recv().map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "the development database owner stopped before reading Bluetooth settings",
            )
        })?
    }

    pub(crate) fn open(root: &Path, database_path: &Path) -> Result<Self, DevelopmentStoreFailure> {
        Self::open_inner(root, database_path, None)
    }

    /// Immutable application-data owner. Every cloned client and admitted job
    /// stays on this identity's lane; changing UI selection cannot retarget it.
    /// The lifecycle coordinator must authorize admission and drain this owner
    /// before completing a stop. Neither reopening nor selecting creates keys.
    pub(crate) fn open_scoped(
        root: &Path,
        scope: StorageScope,
        secret: IdentitySecretKey,
        mode: ScopedOpen,
    ) -> Result<Self, DevelopmentStoreFailure> {
        let identity = PrivateIdentityMaterial::from(secret);
        if identity.identity_hash().as_bytes() != &scope.public_hash.0 {
            return Err(DevelopmentStoreFailure::reset_required(
                "The scoped database key belongs to another local identity.",
            ));
        }
        let directory = root.join("identity-data");
        if matches!(mode, ScopedOpen::Create) {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(DevelopmentStoreFailure::unavailable(error.to_string())),
            }
        }
        let metadata = std::fs::symlink_metadata(&directory)
            .map_err(|error| DevelopmentStoreFailure::unavailable(error.to_string()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(DevelopmentStoreFailure::reset_required(
                "Identity data requires its own ordinary directory.",
            ));
        }
        let path = scope
            .path_under(&directory)
            .map_err(|error| DevelopmentStoreFailure::unavailable(error.to_string()))?;
        Self::open_inner(root, &path, Some((scope, identity, mode)))
    }

    fn require_installation_owner(&self) -> Result<(), DevelopmentStoreFailure> {
        if self.scope.is_some() {
            Err(DevelopmentStoreFailure::unavailable(
                "Bluetooth preferences belong to the installation, not a local identity.",
            ))
        } else {
            Ok(())
        }
    }

    fn open_inner(
        root: &Path,
        database_path: &Path,
        scoped: Option<(StorageScope, PrivateIdentityMaterial, ScopedOpen)>,
    ) -> Result<Self, DevelopmentStoreFailure> {
        let scope = scoped.as_ref().map(|(scope, _, _)| *scope);
        let (jobs_tx, jobs_rx) = mpsc::sync_channel(STORE_LANE_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let database_path = database_path.to_path_buf();
        let group_fixture_path = root.join("group-fixture.json");
        let join = std::thread::Builder::new()
            .name("prns-app-development-store".to_owned())
            .spawn(move || {
                // The wrapper remains alive for the complete store lane and
                // proves its durable owner before any service schema is read.
                let scoped_database = scoped
                    .as_ref()
                    .map(|(scope, _, mode)| {
                        match mode {
                            ScopedOpen::Create => {
                                ScopedDatabase::create_new(&database_path, *scope)
                            }
                            ScopedOpen::Existing => ScopedDatabase::open(&database_path, *scope),
                        }
                        .map_err(map_scoped_failure)
                    })
                    .transpose();
                let scoped_database = match scoped_database {
                    Ok(database) => database,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let legacy_database = if scoped_database.is_none() {
                    Some(open_database(&database_path))
                } else {
                    None
                };
                let database = if let Some(database) = &scoped_database {
                    let validation = match scoped.as_ref().map(|(_, _, mode)| *mode) {
                        Some(ScopedOpen::Create) => {
                            initialize_database_with_name(database.database(), Some(""))
                        }
                        _ => validate_existing_database(database.database()),
                    };
                    validation.map(|()| database.database())
                } else {
                    match legacy_database.as_ref() {
                        Some(Ok(database)) => Ok(database),
                        Some(Err(error)) => Err(error.clone()),
                        None => Err(DevelopmentStoreFailure::unavailable(
                            "No database owner was prepared.",
                        )),
                    }
                };
                match database {
                    Ok(database) => {
                        let settings = crate::group_announces::settings(database, None);
                        if settings.is_err() {
                            let _ = ready_tx.send(settings);
                            return;
                        }
                        let _ = ready_tx.send(settings);
                        run_owner(
                            database,
                            &jobs_rx,
                            scope.is_none().then_some(group_fixture_path.as_path()),
                            scoped.as_ref().map(|(_, identity, _)| identity),
                        );
                    }
                    Err(failure) => {
                        let _ = ready_tx.send(Err(failure));
                    }
                }
            })
            .map_err(|error| {
                DevelopmentStoreFailure::unavailable(format!(
                    "could not start the development database owner: {error}"
                ))
            })?;

        // redb open is a blocking, non-cancellable operation. Waiting here keeps
        // that thread owned; a timeout would detach a future database owner that
        // reset could neither drain nor join. All work uses the bounded
        // admission lane above; read-only callers may bound their waits, while
        // admitted mutation waiters are retained through a definitive reply.
        match ready_rx.recv() {
            Ok(Ok(announce_settings)) => {
                let group_health = crate::groups::GroupHealth::new();
                group_health.announces().update(announce_settings);
                let lane = Arc::new(StoreLane {
                    owner_id: NEXT_OWNER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    bluetooth_changing: std::sync::atomic::AtomicBool::new(false),
                    jobs: StdMutex::new(Some(jobs_tx)),
                    group_action_executing: std::sync::atomic::AtomicBool::new(false),
                    group_identity: StdMutex::new(None),
                    admission: std::sync::atomic::AtomicU8::new(0),
                });
                let group_client = GroupClient::new(Arc::new(DevelopmentGroupSubmitter {
                    lane: Arc::downgrade(&lane),
                    inspection: false,
                }));
                Ok(Self {
                    root: root.to_path_buf(),
                    scope,
                    lane,
                    group_client,
                    group_health,
                    join: Some(join),
                })
            }
            Ok(Err(failure)) => {
                let _ = join.join();
                Err(failure)
            }
            Err(_) => {
                let _ = join.join();
                Err(DevelopmentStoreFailure::unavailable(
                    "the development database owner exited before reporting readiness",
                ))
            }
        }
    }

    pub(crate) fn admit_contact_card(
        &self,
        request: crate::contact_cards::Request,
    ) -> Result<oneshot::Receiver<crate::contact_cards::Reply>, DevelopmentStoreFailure> {
        let (response, receiver) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::ContactCard {
                root: self.root.clone(),
                request,
                response,
            })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "The contact-card storage lane is full or closed.",
                )
            })?;
        Ok(receiver)
    }

    pub(crate) fn admit_directory_async(
        &self,
        request: DirectoryRequest,
    ) -> Result<oneshot::Receiver<StoreReply>, DevelopmentStoreFailure> {
        let (response, receiver) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::DirectoryAsync { request, response })
            .map_err(|_| {
                DevelopmentStoreFailure::unavailable(
                    "the development database lane is full or closed",
                )
            })?;
        Ok(receiver)
    }

    pub(crate) fn admit_mailbox_async(
        &self,
        request: MailboxRequest,
    ) -> Result<oneshot::Receiver<MailboxStoreReply>, MailboxFailure> {
        let (response, receiver) = oneshot::channel();
        self.lane
            .admit_default(StoreJob::MailboxAsync { request, response })
            .map_err(|error| match error {
                StoreAdmissionError::Full => MailboxFailure::Busy,
                StoreAdmissionError::Disconnected => {
                    MailboxFailure::Unavailable("The identity mailbox is fenced or closed.".into())
                }
            })?;
        Ok(receiver)
    }

    pub(crate) fn mailbox_submitter(&self) -> Arc<dyn MailboxSubmitter> {
        Arc::new(DevelopmentMailboxSubmitter {
            lane: Arc::downgrade(&self.lane),
        })
    }

    #[cfg(test)]
    pub(crate) fn admit_test_barrier(
        &self,
        entered: SyncSender<()>,
        release: Receiver<()>,
    ) -> Result<(), DevelopmentStoreFailure> {
        self.jobs()?
            .try_send(StoreJob::Barrier { entered, release })
            .map_err(|error| match error {
                TrySendError::Full(_) => DevelopmentStoreFailure::unavailable(
                    "the bounded development database lane is full",
                ),
                TrySendError::Disconnected(_) => DevelopmentStoreFailure::unavailable(
                    "the development database owner has stopped",
                ),
            })
    }

    fn jobs(&self) -> Result<SyncSender<StoreJob>, DevelopmentStoreFailure> {
        self.lane
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                DevelopmentStoreFailure::unavailable("the development database owner is closing")
            })
    }

    pub(crate) fn close(mut self) -> Result<(), DevelopmentStoreFailure> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> Result<(), DevelopmentStoreFailure> {
        *self
            .lane
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        join.join().map_err(|_| {
            DevelopmentStoreFailure::unavailable(
                "the development database owner panicked while closing",
            )
        })
    }
}

impl Drop for DevelopmentStoreOwner {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}

fn run_owner(
    database: &Database,
    jobs: &Receiver<StoreJob>,
    group_fixture: Option<&Path>,
    scoped_identity: Option<&PrivateIdentityMaterial>,
) {
    let mut group_session = None;
    let mut group_failed = false;
    let mut group_identity = None;
    while let Ok(job) = jobs.recv() {
        match job {
            StoreJob::Activity {
                operation,
                response,
            } => {
                let _ = response.send(crate::named_identities::activity::execute(
                    database,
                    group_session.as_mut(),
                    operation,
                ));
            }
            StoreJob::DependencyRevision {
                fingerprint,
                response,
            } => {
                let _ = response.send(dependency_revision(database, fingerprint));
            }
            StoreJob::DrainAsync(reply) => {
                let _ = reply.send(());
            }
            StoreJob::ReceiveState {
                destination,
                state,
                response,
            } => {
                let _ = response.send(receive_state(database, destination, state));
            }
            StoreJob::Drain(reply) => {
                let _ = reply.send(());
            }
            StoreJob::InitializeGroups { identity, response } => {
                let result = if scoped_identity
                    .is_some_and(|local| local.identity_hash().as_bytes() != identity.as_bytes())
                {
                    Err(DevelopmentStoreFailure::reset_required(
                        "Group initialization cannot change the local storage owner.",
                    ))
                } else if group_session.is_some() {
                    if group_identity == Some(identity) {
                        Ok(())
                    } else {
                        Err(DevelopmentStoreFailure::unavailable(
                            "The group fixture belongs to another native identity.",
                        ))
                    }
                } else {
                    group_fixture
                        .map(|path| crate::groups::open_fixture(database, path, identity))
                        .unwrap_or(Ok(None))
                        .and_then(|legacy| {
                            let session = prns_groups_runner_native::GroupManager::open(
                                database, identity, legacy,
                            )
                            .map_err(|error| {
                                DevelopmentStoreFailure::unavailable(error.to_string())
                            })?;
                            group_session = Some(session);
                            group_identity = Some(identity);
                            Ok(())
                        })
                };
                group_failed = result.is_err();
                let _ = response.send(result);
            }
            StoreJob::Group(job) => match group_session.as_mut() {
                Some(session) => session.execute(job),
                None => job.fail(if group_failed {
                    GroupError::Configuration
                } else {
                    GroupError::NotProvisioned
                }),
            },
            StoreJob::GroupAction { request, response } => {
                // Like accepted group operations, admitted intent writes finish
                // even when a bridge caller disappears before receiving them.
                let _ = response.send(crate::group_actions::execute(
                    database,
                    group_identity,
                    request,
                ));
            }
            StoreJob::BluetoothDiscovery { groups, completion } => {
                completion(crate::bluetooth_discovery::settings(database, groups));
            }
            StoreJob::Connections { input, completion } => {
                completion(crate::connections::settings(database, input));
            }
            StoreJob::GroupAnnounces {
                settings,
                completion,
            } => {
                completion(crate::group_announces::settings(database, settings));
            }
            StoreJob::ContactCard {
                root,
                request,
                response,
            } => {
                let reply = match scoped_identity {
                    Some(identity) => {
                        crate::contact_cards::execute_scoped(database, identity, request)
                    }
                    None => crate::contact_cards::execute(database, &root, request),
                };
                let _ = response.send(reply);
            }
            StoreJob::MessagingName { name, completion } => {
                completion(messaging_name(database, name));
            }
            StoreJob::Bluetooth {
                enabled,
                completion,
            } => {
                completion(bluetooth_settings(database, enabled));
            }
            StoreJob::DirectoryAsync { request, response } => {
                // Admission owns a mutation through commit even if its caller
                // has dropped the receiving future.
                let _ = response.send(directory::execute(database, request));
            }
            StoreJob::MailboxAsync { request, response } => {
                let result = if let Some(identity) = scoped_identity {
                    let destination = crate::messaging_profile::messaging_destination(
                        *identity.identity_hash().as_bytes(),
                    );
                    let belongs = match &request {
                        MailboxRequest::InsertOutbound(message) => {
                            Some(message.source) == destination
                        }
                        MailboxRequest::InsertInbound(message) => {
                            Some(message.destination) == destination
                        }
                        _ => true,
                    };
                    if belongs {
                        execute_mailbox_request(database, request)
                    } else {
                        Err(MailboxFailure::Unavailable(
                            "The message belongs to another local identity.".into(),
                        ))
                    }
                } else {
                    execute_mailbox_request(database, request)
                };
                let _ = response.send(result);
            }
            #[cfg(test)]
            StoreJob::Barrier { entered, release } => {
                let _ = entered.send(());
                let _ = release.recv();
            }
        }
    }
}

fn map_scoped_failure(
    error: prns_identities_native::scoped_store::Error,
) -> DevelopmentStoreFailure {
    use prns_identities_native::scoped_store::Error;
    match error {
        Error::Database(error) => {
            classify_redb_error(error, "open identity-scoped service database")
        }
        Error::MissingBinding | Error::WrongBinding | Error::UnsafePath => {
            DevelopmentStoreFailure::reset_required(error.to_string())
        }
        Error::Io(_) | Error::InvalidScope | Error::UnsupportedPlatform => {
            DevelopmentStoreFailure::unavailable(error.to_string())
        }
    }
}

fn dependency_revision(
    database: &Database,
    fingerprint: [u8; 32],
) -> Result<u64, DevelopmentStoreFailure> {
    let mut write = database
        .begin_write()
        .map_err(|error| classify_redb_error(error.into(), "write dependency revision"))?;
    write
        .set_durability(redb::Durability::Immediate)
        .map_err(|error| classify_redb_error(error.into(), "set dependency durability"))?;
    let revision;
    {
        let mut table = write
            .open_table(DEPENDENCY_REVISION)
            .map_err(|error| classify_redb_error(error.into(), "open dependency revision"))?;
        let existing = table
            .get(0)
            .map_err(|error| classify_redb_error(error.into(), "read dependency revision"))?;
        let previous = match existing.as_ref() {
            Some(value) => {
                let bytes = value.value();
                if bytes.len() != 40 {
                    return Err(DevelopmentStoreFailure::reset_required(
                        "Invalid dependency observation record.",
                    ));
                }
                let revision = u64::from_be_bytes(bytes[..8].try_into().map_err(|_| {
                    DevelopmentStoreFailure::reset_required("Invalid dependency revision.")
                })?);
                if revision == 0 {
                    return Err(DevelopmentStoreFailure::reset_required(
                        "Invalid zero dependency revision.",
                    ));
                }
                if bytes[8..] == fingerprint {
                    return Ok(revision);
                }
                revision
            }
            None => 0,
        };
        drop(existing);
        revision = previous.checked_add(1).ok_or_else(|| {
            DevelopmentStoreFailure::reset_required("Dependency revision exhausted.")
        })?;
        let mut encoded = [0; 40];
        encoded[..8].copy_from_slice(&revision.to_be_bytes());
        encoded[8..].copy_from_slice(&fingerprint);
        table
            .insert(0, encoded.as_slice())
            .map_err(|error| classify_redb_error(error.into(), "save dependency revision"))?;
    }
    write
        .commit()
        .map_err(|error| classify_redb_error(error.into(), "commit dependency revision"))?;
    Ok(revision)
}

fn receive_state(
    database: &Database,
    destination: [u8; 16],
    state: Option<zeroize::Zeroizing<Vec<u8>>>,
) -> ReceiveStateReply {
    if let Some(state) = state {
        let mut tx = database
            .begin_write()
            .map_err(|error| classify_redb_error(error.into(), "save receive state"))?;
        tx.set_durability(redb::Durability::Immediate)
            .map_err(|error| classify_redb_error(error.into(), "set receive state durability"))?;
        tx.open_table(RECEIVE_STATE)
            .map_err(|error| classify_redb_error(error.into(), "open receive state"))?
            .insert(destination.as_slice(), state.as_slice())
            .map_err(|error| classify_redb_error(error.into(), "write receive state"))?;
        tx.commit()
            .map_err(|error| classify_redb_error(error.into(), "commit receive state"))?;
        return Ok(None);
    }
    let read = database
        .begin_read()
        .map_err(|error| classify_redb_error(error.into(), "read receive state"))?;
    let table = match read.open_table(RECEIVE_STATE) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(error) => return Err(classify_redb_error(error.into(), "open receive state")),
    };
    let value = table
        .get(destination.as_slice())
        .map_err(|error| classify_redb_error(error.into(), "get receive state"))?;
    let Some(value) = value else {
        return Ok(None);
    };
    if value.value().is_empty() || value.value().len() > MAX_RECEIVE_STATE_BYTES {
        return Err(DevelopmentStoreFailure::reset_required(
            "The saved receive state exceeds its byte bounds.",
        ));
    }
    Ok(Some(zeroize::Zeroizing::new(value.value().to_vec())))
}

#[cfg(test)]
mod scoped_tests;

pub(crate) fn messaging_name(
    database: &Database,
    name: Option<String>,
) -> Result<String, DevelopmentStoreFailure> {
    if let Some(name) = name {
        let name = if name.is_empty() {
            name
        } else {
            crate::messaging_profile::normalize_name(&name)
                .map_err(DevelopmentStoreFailure::unavailable)?
        };
        let write = database
            .begin_write()
            .map_err(|error| classify_redb_error(error.into(), "write messaging name"))?;
        {
            let mut settings = write
                .open_table(APPLICATION_SETTINGS)
                .map_err(|error| classify_redb_error(error.into(), "open application settings"))?;
            settings
                .insert(MESSAGING_NAME_KEY, name.as_str())
                .map_err(|error| classify_redb_error(error.into(), "save messaging name"))?;
        }
        write
            .commit()
            .map_err(|error| classify_redb_error(error.into(), "commit messaging name"))?;
        return Ok(name);
    }
    let read = database
        .begin_read()
        .map_err(|error| classify_redb_error(error.into(), "read messaging name"))?;
    let settings = match read.open_table(APPLICATION_SETTINGS) {
        Ok(settings) => settings,
        Err(redb::TableError::TableDoesNotExist(_)) => {
            return Ok(crate::messaging_profile::DEFAULT_MESSAGING_NAME.to_owned());
        }
        Err(error) => {
            return Err(classify_redb_error(
                error.into(),
                "open application settings",
            ))
        }
    };
    let stored = settings
        .get(MESSAGING_NAME_KEY)
        .map_err(|error| classify_redb_error(error.into(), "load messaging name"))?
        .map(|name| name.value().to_owned())
        .unwrap_or_else(|| crate::messaging_profile::DEFAULT_MESSAGING_NAME.to_owned());
    if stored.is_empty() {
        return Ok(stored);
    }
    match crate::messaging_profile::normalize_name(&stored) {
        Ok(normalized) if normalized == stored => Ok(stored),
        _ => Err(DevelopmentStoreFailure::reset_required(
            "the stored messaging name is not in canonical form",
        )),
    }
}

fn bluetooth_settings(
    database: &Database,
    enabled: Option<bool>,
) -> Result<bool, DevelopmentStoreFailure> {
    if let Some(enabled) = enabled {
        let write = database
            .begin_write()
            .map_err(|error| classify_redb_error(error.into(), "write Bluetooth preference"))?;
        {
            let mut meta = write
                .open_table(DEVELOPMENT_META)
                .map_err(|error| classify_redb_error(error.into(), "open Bluetooth preference"))?;
            meta.insert(BLUETOOTH_ENABLED_KEY, u64::from(enabled))
                .map_err(|error| classify_redb_error(error.into(), "save Bluetooth preference"))?;
        }
        write
            .commit()
            .map_err(|error| classify_redb_error(error.into(), "commit Bluetooth preference"))?;
        return Ok(enabled);
    }
    let read = database
        .begin_read()
        .map_err(|error| classify_redb_error(error.into(), "read Bluetooth preference"))?;
    let meta = read
        .open_table(DEVELOPMENT_META)
        .map_err(|error| classify_redb_error(error.into(), "open Bluetooth preference"))?;
    match meta
        .get(BLUETOOTH_ENABLED_KEY)
        .map_err(|error| classify_redb_error(error.into(), "load Bluetooth preference"))?
        .map(|value| value.value())
    {
        None | Some(1) => Ok(true),
        Some(0) => Ok(false),
        Some(_) => Err(DevelopmentStoreFailure::unavailable(
            "the stored Bluetooth preference is invalid",
        )),
    }
}

fn open_database(path: &Path) -> Result<Database, DevelopmentStoreFailure> {
    let (database, fresh) = match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => (
            Database::builder().create_file(file).map_err(|error| {
                classify_redb_error(error.into(), "create development database")
            })?,
            true,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
            Database::open(path)
                .map_err(|error| classify_redb_error(error.into(), "open development database"))?,
            false,
        ),
        Err(error) => {
            return Err(DevelopmentStoreFailure::unavailable(format!(
                "could not create the development database file: {error}"
            )))
        }
    };

    if fresh {
        initialize_database(&database)?;
    } else {
        validate_existing_database(&database)?;
    }
    Ok(database)
}

fn initialize_database(database: &Database) -> Result<(), DevelopmentStoreFailure> {
    initialize_database_with_name(database, None)
}
fn initialize_database_with_name(
    database: &Database,
    initial_name: Option<&str>,
) -> Result<(), DevelopmentStoreFailure> {
    let write = database
        .begin_write()
        .map_err(|error| classify_redb_error(error.into(), "initialize development database"))?;
    {
        let mut meta = write.open_table(DEVELOPMENT_META).map_err(|error| {
            classify_redb_error(error.into(), "open development metadata table")
        })?;
        meta.insert(DEVELOPMENT_FORMAT_KEY, DEVELOPMENT_FORMAT)
            .map_err(|error| {
                classify_redb_error(error.into(), "write development format marker")
            })?;
    }
    {
        write.open_table(CONTACTS).map_err(|error| {
            classify_redb_error(error.into(), "create development contacts table")
        })?;
    }
    initialize_mailbox_tables(&write).map_err(map_mailbox_failure)?;
    if let Some(name) = initial_name {
        write
            .open_table(APPLICATION_SETTINGS)
            .map_err(|error| classify_redb_error(error.into(), "initialize scoped profile"))?
            .insert(MESSAGING_NAME_KEY, name)
            .map_err(|error| classify_redb_error(error.into(), "initialize scoped name"))?;
    }
    write.commit().map_err(|error| {
        classify_redb_error(error.into(), "commit development database initialization")
    })
}

fn validate_existing_database(database: &Database) -> Result<(), DevelopmentStoreFailure> {
    let read = database
        .begin_read()
        .map_err(|error| classify_redb_error(error.into(), "read development database"))?;
    let meta = read
        .open_table(DEVELOPMENT_META)
        .map_err(|error| classify_redb_error(error.into(), "open development metadata table"))?;
    let marker = meta
        .get(DEVELOPMENT_FORMAT_KEY)
        .map_err(|error| classify_redb_error(error.into(), "read development format marker"))?
        .map(|value| value.value());
    if marker != Some(DEVELOPMENT_FORMAT) {
        return Err(DevelopmentStoreFailure::reset_required(match marker {
            Some(found) => format!(
                "the development database format marker is {found}, expected {DEVELOPMENT_FORMAT}"
            ),
            None => "the development database format marker is missing".to_owned(),
        }));
    }
    let contacts = read
        .open_table(CONTACTS)
        .map_err(|error| classify_redb_error(error.into(), "open development contacts table"))?;
    let entries = contacts
        .iter()
        .map_err(|error| classify_redb_error(error.into(), "scan development contacts table"))?;
    for entry in entries {
        let (key, value) = entry.map_err(|error| {
            classify_redb_error(error.into(), "scan development contact record")
        })?;
        directory::decode_contact(key.value(), value.value())?;
    }
    drop(contacts);
    drop(read);
    validate_mailbox_database(database).map_err(map_mailbox_failure)
}

fn map_mailbox_failure(failure: MailboxFailure) -> DevelopmentStoreFailure {
    match failure {
        MailboxFailure::Busy => DevelopmentStoreFailure::unavailable(
            "the development mailbox owner was unexpectedly busy",
        ),
        MailboxFailure::Unavailable(detail) => DevelopmentStoreFailure::unavailable(detail),
        MailboxFailure::ResetRequired(reason) => DevelopmentStoreFailure::reset_required(reason),
    }
}

pub(crate) fn classify_redb_error(error: redb::Error, operation: &str) -> DevelopmentStoreFailure {
    let detail = format!("could not {operation}: {error}");
    match error {
        redb::Error::Corrupted(_)
        | redb::Error::UpgradeRequired(_)
        | redb::Error::RepairAborted
        | redb::Error::TableTypeMismatch { .. }
        | redb::Error::TableIsMultimap(_)
        | redb::Error::TableIsNotMultimap(_)
        | redb::Error::TypeDefinitionChanged { .. }
        | redb::Error::TableDoesNotExist(_) => DevelopmentStoreFailure::reset_required(detail),
        redb::Error::Io(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            DevelopmentStoreFailure::reset_required(detail)
        }
        _ => DevelopmentStoreFailure::unavailable(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{ContactListOutcome, ContactMutationOutcome};
    use crate::directory::DirectoryRequest;

    #[test]
    fn messaging_name_adds_to_existing_database_without_changing_mail() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("application.redb");
        let database = open_database(&path).unwrap();
        assert_eq!(messaging_name(&database, None).unwrap(), "prns");
        execute_mailbox_request(
            &database,
            MailboxRequest::InsertOutbound(outbound_message()),
        )
        .unwrap();
        assert_eq!(
            messaging_name(&database, Some("  My phone  ".into())).unwrap(),
            "My phone"
        );
        drop(database);
        let database = open_database(&path).unwrap();
        assert_eq!(messaging_name(&database, None).unwrap(), "My phone");
        assert!(
            matches!(execute_mailbox_request(&database, mailbox_list()).unwrap(), MailboxReply::Listed { messages, .. } if messages.len() == 1)
        );
        let write = database.begin_write().unwrap();
        write
            .open_table(APPLICATION_SETTINGS)
            .unwrap()
            .insert(MESSAGING_NAME_KEY, "\ninvalid\n")
            .unwrap();
        write.commit().unwrap();
        assert!(matches!(
            messaging_name(&database, None),
            Err(DevelopmentStoreFailure::ResetRequired(_))
        ));
    }

    #[test]
    fn bluetooth_preference_commits_and_reopens_without_touching_mail() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("application.redb");
        let database = open_database(&path).unwrap();
        assert!(bluetooth_settings(&database, None).unwrap());
        let message = outbound_message();
        assert!(
            execute_mailbox_request(&database, MailboxRequest::InsertOutbound(message)).is_ok()
        );
        assert!(!bluetooth_settings(&database, Some(false)).unwrap());
        drop(database);
        let database = open_database(&path).unwrap();
        assert!(!bluetooth_settings(&database, None).unwrap());
        let listed = execute_mailbox_request(&database, mailbox_list()).unwrap();
        assert!(matches!(listed, MailboxReply::Listed { messages, .. } if messages.len() == 1));
        assert!(bluetooth_settings(&database, Some(true)).unwrap());
    }

    #[test]
    fn corrupt_bluetooth_preference_is_explicit_and_never_defaults_on() {
        let temporary = tempfile::tempdir().unwrap();
        let database = open_database(&temporary.path().join("application.redb")).unwrap();
        let write = database.begin_write().unwrap();
        write
            .open_table(DEVELOPMENT_META)
            .unwrap()
            .insert(BLUETOOTH_ENABLED_KEY, 4)
            .unwrap();
        write.commit().unwrap();
        assert!(matches!(
            bluetooth_settings(&database, None),
            Err(DevelopmentStoreFailure::Unavailable(_))
        ));
    }

    fn mailbox_list() -> MailboxRequest {
        MailboxRequest::List(prns_lxmf::mailbox::MailboxListRequest {
            peer: None,
            direction: None,
            before: None,
            limit: 100,
        })
    }

    fn outbound_message() -> prns_lxmf::mailbox::NewOutboundMessage {
        let identity = prns_lxmf::direct::LocalLxmfIdentity::from_secret_bytes(
            &[0x71; personal_rns::identity::IDENTITY_SECRET_KEY_LEN],
        )
        .expect("fixed local identity");
        let source = identity.destination();
        let destination = [0x42; 16];
        let mut output = [0_u8; prns_lxmf::wire::MAX_BASIC_LXMF_WIRE_BYTES];
        let prepared = prns_lxmf::wire::compose_basic_direct_lxmf(
            destination,
            source,
            1_700_000_009_000,
            b"Retained waiter",
            b"Exact commit",
            None,
            &identity,
            &mut output,
        )
        .expect("small direct message");
        prns_lxmf::mailbox::NewOutboundMessage {
            message_id: prepared.message_id(),
            source,
            destination,
            timestamp_unix_ms: 1_700_000_009_000,
            title: b"Retained waiter".to_vec(),
            content: b"Exact commit".to_vec(),
            exact_wire: output[..usize::from(prepared.wire_len())].to_vec(),
        }
    }

    fn call(owner: &DevelopmentStoreOwner, request: DirectoryRequest) -> StoreReply {
        owner
            .admit_directory_async(request)
            .unwrap()
            .blocking_recv()
            .unwrap()
    }

    #[test]
    fn first_open_is_atomic_and_reopens() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        assert_eq!(
            call(&owner, DirectoryRequest::List).unwrap(),
            DirectoryResponse::List(ContactListOutcome::Listed { contacts: vec![] })
        );
        owner.close().unwrap();

        let database = Database::open(&path).unwrap();
        let read = database.begin_read().unwrap();
        let meta = read.open_table(DEVELOPMENT_META).unwrap();
        assert_eq!(
            meta.get(DEVELOPMENT_FORMAT_KEY).unwrap().unwrap().value(),
            DEVELOPMENT_FORMAT
        );
        read.open_table(CONTACTS).unwrap();
        drop(read);
        prns_lxmf::mailbox::validate_mailbox_database(&database).unwrap();
        drop(database);

        DevelopmentStoreOwner::open(root.path(), &path)
            .unwrap()
            .close()
            .unwrap();
    }

    #[test]
    fn dropping_an_admitted_async_waiter_does_not_cancel_its_commit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        let jobs = owner.jobs().unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        assert!(jobs
            .try_send(StoreJob::Barrier {
                entered: entered_tx,
                release: release_rx,
            })
            .is_ok());
        entered_rx.recv().expect("owner reached the barrier");

        let (response, waiter) = oneshot::channel();
        assert!(jobs
            .try_send(StoreJob::MailboxAsync {
                request: MailboxRequest::InsertOutbound(outbound_message()),
                response,
            })
            .is_ok());
        drop(waiter);
        release_tx.send(()).expect("release owner");
        drop(jobs);
        owner.close().unwrap();

        let reopened = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        let reply = reopened
            .admit_mailbox_async(mailbox_list())
            .unwrap()
            .blocking_recv()
            .unwrap()
            .unwrap();
        let MailboxReply::Listed { messages, .. } = reply else {
            panic!("unexpected mailbox reply");
        };
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            messages[0].delivery_state,
            prns_lxmf::mailbox::DurableLxmfDeliveryState::Queued { .. }
        ));
        reopened.close().unwrap();
    }

    #[test]
    fn missing_marker_requires_reset() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let database = Database::create(&path).unwrap();
        let write = database.begin_write().unwrap();
        write.open_table(DEVELOPMENT_META).unwrap();
        write.open_table(CONTACTS).unwrap();
        write.commit().unwrap();
        drop(database);

        assert!(matches!(
            DevelopmentStoreOwner::open(root.path(), &path),
            Err(DevelopmentStoreFailure::ResetRequired(_))
        ));
    }

    #[test]
    fn group_send_survives_caller_drop_and_reopen_on_the_shared_store_lane() {
        use crate::test_support::foreign_block_on;
        use prns_groups_runner_native::{fixture, Clock, SendText, SystemClock};
        use std::{
            future::Future,
            task::{Context, Waker},
        };

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let identity = prns_host::IdentityHash::new([61; 16]);
        let clock = SystemClock.now();
        let now = match clock {
            prns_groups_service::ClockReading::Trusted(interval) => interval.upper(),
            prns_groups_service::ClockReading::Unknown => panic!("host clock unavailable"),
        };
        let fixtures = fixture::generate(
            "Native recovery",
            &[identity, prns_host::IdentityHash::new([62; 16])],
            now,
        )
        .unwrap();
        std::fs::write(root.path().join("group-fixture.json"), &fixtures[0]).unwrap();
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        owner.initialize_groups_blocking(identity).unwrap();
        let client = owner.group_client();
        let group = foreign_block_on(client.network_config()).unwrap().group;
        let request = SendText {
            group,
            client_request_id: prns_groups_runner_native::RequestToken::new(now.0, [71; 20])
                .unwrap()
                .into_bytes(),
            text: "Survives caller teardown".into(),
            clock,
        };
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        owner.admit_test_barrier(entered_tx, release_rx).unwrap();
        entered_rx.recv().unwrap();
        let mut waiter = Box::pin(client.send_text(request.clone()));
        let mut context = Context::from_waker(Waker::noop());
        assert!(waiter.as_mut().poll(&mut context).is_pending());
        drop(waiter);
        release_tx.send(()).unwrap();
        owner.drain_blocking().unwrap();
        owner.close().unwrap();
        assert!(matches!(
            foreign_block_on(client.conversations()),
            Err(GroupError::Unavailable)
        ));

        let reopened = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        reopened.initialize_groups_blocking(identity).unwrap();
        let client = reopened.group_client();
        let page = foreign_block_on(client.messages(group, None, 50, clock)).unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(
            page.messages[0].client_request_id,
            Some(request.client_request_id)
        );
        let original = page.messages[0].id;
        assert_eq!(
            foreign_block_on(client.send_text(request.clone())).unwrap(),
            original
        );
        let conflicting = SendText {
            text: "Changed text".into(),
            ..request
        };
        assert!(matches!(
            foreign_block_on(client.send_text(conflicting)),
            Err(GroupError::IdempotencyConflict)
        ));
        assert_eq!(
            foreign_block_on(client.messages(group, None, 50, clock))
                .unwrap()
                .messages
                .len(),
            1
        );
        reopened.close().unwrap();
    }

    #[test]
    fn existing_empty_database_requires_reset() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        std::fs::File::create(&path).unwrap();

        let failure = match DevelopmentStoreOwner::open(root.path(), &path) {
            Ok(_) => panic!("an existing empty database unexpectedly opened"),
            Err(failure) => failure,
        };
        assert!(
            matches!(failure, DevelopmentStoreFailure::ResetRequired(_)),
            "unexpected classification: {failure:?}"
        );
    }

    #[test]
    fn wrong_marker_requires_reset() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        DevelopmentStoreOwner::open(root.path(), &path)
            .unwrap()
            .close()
            .unwrap();
        let database = Database::open(&path).unwrap();
        let write = database.begin_write().unwrap();
        {
            let mut meta = write.open_table(DEVELOPMENT_META).unwrap();
            meta.insert(DEVELOPMENT_FORMAT_KEY, DEVELOPMENT_FORMAT + 1)
                .unwrap();
        }
        write.commit().unwrap();
        drop(database);

        assert!(matches!(
            DevelopmentStoreOwner::open(root.path(), &path),
            Err(DevelopmentStoreFailure::ResetRequired(_))
        ));
    }

    #[test]
    fn schema_v1_requires_explicit_reset_without_migration() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let database = Database::create(&path).unwrap();
        let write = database.begin_write().unwrap();
        {
            let mut meta = write.open_table(DEVELOPMENT_META).unwrap();
            meta.insert(DEVELOPMENT_FORMAT_KEY, 1).unwrap();
        }
        write.open_table(CONTACTS).unwrap();
        write.commit().unwrap();
        drop(database);

        assert!(matches!(
            DevelopmentStoreOwner::open(root.path(), &path),
            Err(DevelopmentStoreFailure::ResetRequired(reason)) if reason.contains("is 1, expected 2")
        ));
    }

    #[test]
    fn directory_and_mailbox_jobs_share_one_bounded_owner() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        assert!(matches!(
            call(
                &owner,
                DirectoryRequest::CreateManual {
                    destination: [7; 16],
                    identity: None,
                    alias: Some("Shared owner".to_owned()),
                },
            )
            .unwrap(),
            DirectoryResponse::Mutation(ContactMutationOutcome::Saved { .. })
        ));
        let mailbox = owner
            .admit_mailbox_async(mailbox_list())
            .unwrap()
            .blocking_recv()
            .unwrap()
            .unwrap();
        assert!(matches!(
            mailbox,
            MailboxReply::Listed { messages, revision: 0 } if messages.is_empty()
        ));
        assert!(matches!(
            call(&owner, DirectoryRequest::List).unwrap(),
            DirectoryResponse::List(ContactListOutcome::Listed { contacts }) if contacts.len() == 1
        ));
        owner.close().unwrap();
    }

    #[tokio::test]
    async fn cloned_mailbox_submitter_cannot_keep_the_owner_open() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        let submitter = owner.mailbox_submitter();
        owner.close().unwrap();

        assert!(matches!(
            submitter.submit(mailbox_list()).await,
            Err(MailboxFailure::Unavailable(_))
        ));
        drop(Database::open(path).unwrap());
    }

    #[test]
    fn malformed_record_is_rejected_during_open_scan() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        DevelopmentStoreOwner::open(root.path(), &path)
            .unwrap()
            .close()
            .unwrap();
        let database = Database::open(&path).unwrap();
        let write = database.begin_write().unwrap();
        {
            let mut contacts = write.open_table(CONTACTS).unwrap();
            contacts
                .insert(
                    [9_u8; 16].as_slice(),
                    br#"{"alias":null,"pinned":false}"#.as_slice(),
                )
                .unwrap();
        }
        write.commit().unwrap();
        drop(database);

        assert!(matches!(
            DevelopmentStoreOwner::open(root.path(), &path),
            Err(DevelopmentStoreFailure::ResetRequired(_))
        ));
    }

    #[test]
    fn noncanonical_records_are_rejected_during_open_scan() {
        for encoded in [
            br#"{"alias":" Alice ","identity":null,"pinned":false}"#.as_slice(),
            br#"{"alias":null,"identity":null,"pinned":true}"#.as_slice(),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("application.redb");
            DevelopmentStoreOwner::open(root.path(), &path)
                .unwrap()
                .close()
                .unwrap();
            let database = Database::open(&path).unwrap();
            let write = database.begin_write().unwrap();
            {
                let mut contacts = write.open_table(CONTACTS).unwrap();
                contacts.insert([9_u8; 16].as_slice(), encoded).unwrap();
            }
            write.commit().unwrap();
            drop(database);

            assert!(matches!(
                DevelopmentStoreOwner::open(root.path(), &path),
                Err(DevelopmentStoreFailure::ResetRequired(_))
            ));
        }
    }

    #[test]
    fn database_lock_is_unavailable_without_reset_advice() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        DevelopmentStoreOwner::open(root.path(), &path)
            .unwrap()
            .close()
            .unwrap();
        let held = Database::open(&path).unwrap();
        assert!(matches!(
            DevelopmentStoreOwner::open(root.path(), &path),
            Err(DevelopmentStoreFailure::Unavailable(_))
        ));
        drop(held);
    }

    #[test]
    fn owner_drains_before_database_can_be_removed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        let response = owner.admit_directory_async(DirectoryRequest::CreateManual {
            destination: [7; 16],
            identity: Some([8; 16]),
            alias: Some(" Alice ".to_owned()),
        });
        let mailbox = owner.admit_mailbox_async(mailbox_list());
        owner.close().unwrap();
        assert!(matches!(
            response.unwrap().blocking_recv().unwrap().unwrap(),
            DirectoryResponse::Mutation(ContactMutationOutcome::Saved { .. })
        ));
        assert!(matches!(
            mailbox.unwrap().blocking_recv().unwrap().unwrap(),
            MailboxReply::Listed { .. }
        ));
        std::fs::remove_file(path).unwrap();
    }
}
