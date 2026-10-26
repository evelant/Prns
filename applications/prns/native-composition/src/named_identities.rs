//! Phone composition over the portable identity coordinator. Every service job
//! carries a complete immutable owner reference; selection is only a UI default.
use crate::contract::*;
use crate::identity_services::{
    MessagingDriver, MessagingEvents, MessagingIdentity, MESSAGING_SERVICE,
};
use prns_host_native::owner::HostClient;
use prns_identities_core as core;
use prns_identities_native::{scoped_store::StorageScope, NativeStore};
use prns_identities_runtime as runtime;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::{mpsc, oneshot};

mod actions;
pub(crate) mod activity;
pub use activity::list_named_identity_activity;
mod announces;
mod api;
mod direct;
mod groups;
mod jobs;
mod receipts;

pub const MAX_IDENTITIES: usize = 8;
const OPERATIONS: usize = 64;
type Disk = NativeStore<MAX_IDENTITIES, OPERATIONS, 1>;
type Catalog = core::Catalog<MAX_IDENTITIES, OPERATIONS, 1>;
type Coordinator<'a> = core::Coordinator<'a, MAX_IDENTITIES, OPERATIONS, 1>;
type Controller<'a> = runtime::Controller<'a, MAX_IDENTITIES, 1, 8>;
type WorkFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
type Work =
    Box<dyn for<'a> FnOnce(Result<&'a mut MessagingIdentity, String>) -> WorkFuture<'a> + Send>;

fn detail(value: impl std::fmt::Debug) -> String {
    format!("Named identity: {value:?}")
}
impl NamedIdentityReference {
    fn id(&self) -> core::LocalIdentityId {
        core::LocalIdentityId {
            incarnation: self.incarnation,
            sequence: self.sequence,
        }
    }
    fn from_record(record: &core::IdentityRecord<1>) -> Self {
        Self {
            incarnation: record.id.incarnation,
            sequence: record.id.sequence,
            public_hash: record.public_hash.0,
        }
    }
}
enum Request {
    Activity {
        identity: NamedIdentityReference,
        work: Work,
    },
    Catalog(oneshot::Sender<NamedIdentityCatalogOutcome>),
    Create(
        CreateNamedIdentityInput,
        oneshot::Sender<NamedIdentityCatalogOutcome>,
    ),
    Change(
        ChangeNamedIdentityInput,
        oneshot::Sender<NamedIdentityCatalogOutcome>,
    ),
    Service {
        identity: NamedIdentityReference,
        class: core::AdmissionClass,
        work: Work,
    },
    Announces(announces::AnnounceRequest),
    Stop(oneshot::Sender<Result<(), String>>),
}
/// One actor owns coordinator buffers and runtime leases while bounded service
/// jobs run independently by identity, even after a foreign caller disappears.
pub struct NamedRuntime {
    commands: mpsc::Sender<Request>,
}
static OWNER: OnceLock<Mutex<Option<Arc<NamedRuntime>>>> = OnceLock::new();
pub fn install_global(owner: Option<Arc<NamedRuntime>>) {
    *OWNER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = owner;
}
fn global() -> Result<Arc<NamedRuntime>, String> {
    OWNER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(detail)?
        .clone()
        .ok_or_else(|| "Start the local node to open named identities.".into())
}
impl NamedRuntime {
    pub async fn open(
        root: &Path,
        host: HostClient,
        events: MessagingEvents,
    ) -> Result<Arc<Self>, String> {
        let (commands, receiver) = mpsc::channel(32);
        let (ready, startup) = oneshot::channel();
        let root = root.to_owned();
        tokio::spawn(async move {
            run(root, host, events, receiver, ready).await;
        });
        startup.await.map_err(detail)??;
        Ok(Arc::new(Self { commands }))
    }
    pub async fn stop(&self) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .send(Request::Stop(reply))
            .await
            .map_err(detail)?;
        receive.await.map_err(detail)?
    }
    async fn catalog_request(
        &self,
        make: impl FnOnce(oneshot::Sender<NamedIdentityCatalogOutcome>) -> Request,
    ) -> NamedIdentityCatalogOutcome {
        let (reply, receive) = oneshot::channel();
        if self.commands.try_send(make(reply)).is_err() {
            return NamedIdentityCatalogOutcome::Busy;
        }
        receive
            .await
            .unwrap_or_else(|_| NamedIdentityCatalogOutcome::Unavailable {
                detail: "The identity owner stopped.".into(),
            })
    }
}
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub async fn list_named_identities() -> NamedIdentityCatalogOutcome {
    match global() {
        Ok(owner) => owner.catalog_request(Request::Catalog).await,
        Err(detail) => NamedIdentityCatalogOutcome::Unavailable { detail },
    }
}
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub async fn create_named_identity(input: CreateNamedIdentityInput) -> NamedIdentityCatalogOutcome {
    match global() {
        Ok(owner) => owner.catalog_request(|r| Request::Create(input, r)).await,
        Err(detail) => NamedIdentityCatalogOutcome::Unavailable { detail },
    }
}
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub async fn change_named_identity(input: ChangeNamedIdentityInput) -> NamedIdentityCatalogOutcome {
    match global() {
        Ok(owner) => owner.catalog_request(|r| Request::Change(input, r)).await,
        Err(detail) => NamedIdentityCatalogOutcome::Unavailable { detail },
    }
}
#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Object))]
pub struct NamedIdentityClient {
    identity: NamedIdentityReference,
}
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub fn open_named_identity(identity: NamedIdentityReference) -> Arc<NamedIdentityClient> {
    Arc::new(NamedIdentityClient { identity })
}
impl NamedIdentityClient {
    async fn call<T: Send + 'static>(
        &self,
        class: core::AdmissionClass,
        action: impl for<'a> FnOnce(
                &'a mut MessagingIdentity,
            )
                -> Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>
            + Send
            + 'static,
    ) -> Result<T, String> {
        let (reply, receive) = oneshot::channel();
        let work: Work = Box::new(move |services| {
            Box::pin(async move {
                let result = match services {
                    Ok(services) => action(services).await,
                    Err(error) => Err(error),
                };
                let _ = reply.send(result);
            })
        });
        global()?
            .commands
            .try_send(Request::Service {
                identity: self.identity.clone(),
                class,
                work,
            })
            .map_err(|_| "The identity command lane is busy or stopped.".to_owned())?;
        receive
            .await
            .map_err(|_| "The identity owner stopped before returning the result.".to_owned())?
    }
}

struct Actor<'a> {
    root: PathBuf,
    disk: Disk,
    coordinator: Coordinator<'a>,
    controller: Controller<'a>,
    driver: MessagingDriver,
    host: HostClient,
    events: MessagingEvents,
    selected: Option<NamedIdentityReference>,
    mutation_sequence: u64,
    create_receipts: Vec<receipts::CreateReceipt>,
    announces: announces::SharedAnnounces,
    jobs: jobs::Jobs,
}
fn budget() -> runtime::Budget {
    runtime::Budget {
        capacity: runtime::Resources {
            held_keys: 25,
            destinations: 25,
            request_handlers: 17,
            links: 257,
            queue_slots: 129,
        },
        infrastructure_reserve: runtime::Resources {
            held_keys: 1,
            destinations: 1,
            request_handlers: 1,
            links: 1,
            queue_slots: 1,
        },
    }
}
async fn run(
    root: PathBuf,
    host: HostClient,
    events: MessagingEvents,
    mut requests: mpsc::Receiver<Request>,
    ready: oneshot::Sender<Result<(), String>>,
) {
    let mut catalog = Box::new(Catalog::new([0; 16]));
    let mut staging = Box::new(Catalog::new([0; 16]));
    let mut incarnation = [0; 16];
    if let Err(error) = getrandom::getrandom(&mut incarnation) {
        let _ = ready.send(Err(detail(error)));
        return;
    }
    let mut runtime_state = match runtime::RuntimeState::new(incarnation, budget()) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(detail(e)));
            return;
        }
    };
    let startup = async {
        let path = root.join("named-identities");
        let mut disk = if path.try_exists().map_err(detail)? {
            Disk::open(path)
        } else {
            Disk::create_new(path)
        }
        .map_err(detail)?;
        let coordinator =
            Coordinator::open(&mut disk, &mut catalog, &mut staging).map_err(detail)?;
        let announces = announces::SharedAnnounces::open(&root)?;
        let actor = Actor {
            root,
            disk,
            coordinator,
            controller: Controller::new(&mut runtime_state),
            driver: MessagingDriver::new(MAX_IDENTITIES).map_err(detail)?,
            host,
            events,
            selected: None,
            mutation_sequence: 0,
            create_receipts: Vec::new(),
            announces,
            jobs: jobs::Jobs::new(),
        };
        Ok::<_, String>(actor)
    }
    .await;
    let mut actor = match startup {
        Ok(actor) => actor,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let initialized = async {
        actor.read_receipts()?;
        actor.recover().await?;
        if actor.coordinator.snapshot().identities().next().is_none() {
            actor
                .create(CreateNamedIdentityInput {
                    request_id: [1; 32],
                    label: "Personal".into(),
                    published_name: None,
                })
                .await?;
        }
        actor.retire_operations()?;
        actor.read_selection()?;
        actor.restore_announces().await
    }
    .await;
    if let Err(error) = initialized {
        let cleanup = actor.close().await;
        let _ = ready.send(Err(match cleanup {
            Ok(()) => error,
            Err(cleanup) => format!("{error}; cleanup requires host shutdown: {cleanup}"),
        }));
        return;
    }
    if ready.send(Ok(())).is_err() {
        let _ = actor.close().await;
        return;
    }
    let mut announce_ticks = tokio::time::interval(std::time::Duration::from_secs(1));
    announce_ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut stop_reply = None;
    let mut stopping = false;
    let mut failure = None;
    loop {
        if actor.coordinator.is_poisoned() {
            failure.get_or_insert_with(|| "Identity storage requires restart recovery.".to_owned());
            stopping = true;
        }
        if stopping {
            requests.close();
            actor.fence_jobs();
            while let Ok(request) = requests.try_recv() {
                jobs::reject(request, "The identity owner is stopping.").await;
            }
            while let Some(request) = actor.jobs.deferred.pop_front() {
                jobs::reject(request, "The identity owner is stopping.").await;
            }
            if actor.jobs.tasks.is_empty() {
                break;
            }
        }
        let request = if !stopping {
            actor.ready_deferred()
        } else {
            None
        };
        let request = match request {
            Some(request) => request,
            None => tokio::select! {
                // A service sends its foreign response immediately before task
                // completion. Settle its lease before admitting follow-up work.
                biased;
                result = actor.jobs.tasks.join_next(), if !actor.jobs.tasks.is_empty() => {
                    if let Some(result) = result {
                        if let Err(error) = actor.finish_job(result) {
                            failure.get_or_insert(error);
                            stopping = true;
                        }
                    }
                    continue;
                }
                _ = announce_ticks.tick(), if !stopping => {
                    actor.announce_tick().await;
                    continue;
                }
                request = requests.recv(), if !stopping => match request {
                    Some(request) => request,
                    None => { stopping = true; continue; }
                },
            },
        };
        if let Request::Stop(reply) = request {
            stop_reply = Some(reply);
            stopping = true;
        } else {
            actor.handle_request(request).await;
        }
    }
    let result = actor.close().await.and(match failure {
        Some(error) => Err(error),
        None => Ok(()),
    });
    if let Some(reply) = stop_reply {
        let _ = reply.send(result);
    }
}

macro_rules! finish_step {
    ($expression:expr, $driver:expr, $authority:expr) => {{
        loop {
            match $expression.map_err(detail)? {
                runtime::Progress::Complete(value) => break value,
                runtime::Progress::Pending => $driver.advance(&$authority).await.map_err(detail)?,
            }
        }
    }};
}
impl Actor<'_> {
    fn check(&self, reference: &NamedIdentityReference) -> Result<core::IdentityRecord<1>, String> {
        let record = *self.coordinator.identity(reference.id()).map_err(detail)?;
        if record.public_hash.0 != reference.public_hash {
            return Err("The identity reference belongs to a different key.".into());
        }
        Ok(record)
    }
    fn command_failure(&self, detail: String, before: u64) -> NamedIdentityCatalogOutcome {
        if self.coordinator.is_poisoned() || self.coordinator.snapshot().revision != before {
            NamedIdentityCatalogOutcome::Unavailable { detail }
        } else {
            NamedIdentityCatalogOutcome::Rejected { detail }
        }
    }
    fn catalog_result(&self, result: Result<(), String>) -> NamedIdentityCatalogOutcome {
        if let Err(detail) = result {
            return NamedIdentityCatalogOutcome::Rejected { detail };
        }
        if self.coordinator.is_poisoned() {
            return NamedIdentityCatalogOutcome::Unavailable {
                detail: "Identity storage requires restart recovery.".into(),
            };
        }
        let mut identities = Vec::with_capacity(MAX_IDENTITIES);
        for record in self.coordinator.snapshot().identities() {
            let published_name = match self.driver.cached_published_name(record.id) {
                Ok(Some(name)) => (!name.is_empty()).then_some(name),
                Ok(None) => {
                    return NamedIdentityCatalogOutcome::Unavailable {
                        detail: "The identity profile has not finished loading.".into(),
                    };
                }
                Err(error) => {
                    return NamedIdentityCatalogOutcome::Unavailable {
                        detail: detail(error),
                    };
                }
            };
            let transitioning = match self.coordinator.recovery_action(record.id) {
                Ok(action) => action.is_some(),
                Err(error) => {
                    return NamedIdentityCatalogOutcome::Unavailable {
                        detail: detail(error),
                    };
                }
            };
            identities.push(NamedIdentitySummary {
                reference: NamedIdentityReference::from_record(record),
                label: record.metadata.local_label.as_str().into(),
                published_name,
                state: match record.desired {
                    core::DesiredState::Enabled => NamedIdentityState::Enabled,
                    core::DesiredState::Paused => NamedIdentityState::Paused,
                    core::DesiredState::Retired => NamedIdentityState::Retired,
                },
                transitioning,
                ready: self
                    .coordinator
                    .admit(record.id, record.public_hash, core::AdmissionClass::Network)
                    .is_ok(),
            });
        }
        NamedIdentityCatalogOutcome::Ready {
            catalog: NamedIdentityCatalog {
                selected: self.selected.clone(),
                identities,
            },
        }
    }

    async fn install(
        &mut self,
        id: core::LocalIdentityId,
        allow_create: bool,
    ) -> Result<(), String> {
        if self.driver.entry(id).is_ok() {
            return Ok(());
        }
        let record = *self.coordinator.identity(id).map_err(detail)?;
        let guard = self
            .coordinator
            .admit(id, record.public_hash, core::AdmissionClass::History)
            .map_err(detail)?;
        let secret = self
            .disk
            .local_key(&self.coordinator, guard)
            .map_err(detail)?;
        let scope = StorageScope {
            identity: id,
            public_hash: record.public_hash,
        };
        let exists = scope
            .path_under(&self.root.join("identity-data"))
            .map_err(detail)?
            .try_exists()
            .map_err(detail)?;
        if !exists && !allow_create {
            return Err("The identity service store is missing; refusing to replace its history or MLS state.".into());
        }
        let services = MessagingIdentity::open(
            &self.root,
            scope,
            record.key_reference,
            secret,
            !exists,
            self.host.clone(),
            &self.events,
        )
        .map_err(detail)?;
        self.driver.install(services).map_err(detail)
    }
    async fn recover(&mut self) -> Result<(), String> {
        let operations: Vec<_> = self.coordinator.snapshot().operations().copied().collect();
        for operation in operations {
            if let core::Command::Create(_) = operation.command {
                if operation.state == core::OperationState::Reserved {
                    self.coordinator
                        .begin(&mut self.disk, operation.token, &operation.command)
                        .map_err(detail)?;
                }
                if let Some(core::Action::PublishKey { ticket, .. }) =
                    self.coordinator.action(operation.token).map_err(detail)?
                {
                    let key = self
                        .disk
                        .publish_key(&self.coordinator, ticket, None)
                        .map_err(detail)?;
                    self.coordinator
                        .ack_key(&mut self.disk, ticket, key)
                        .map_err(detail)?;
                }
            }
        }
        let records: Vec<_> = self.coordinator.snapshot().identities().copied().collect();
        for record in records {
            // A completed create may precede initial store publication. Only the
            // original never-enabled key can initialize a missing scoped store.
            self.install(record.id, record.lifecycle_revision == 1)
                .await?;
            if record.lifecycle_revision == 1 && record.desired == core::DesiredState::Paused {
                if !record.metadata.published_name.as_str().is_empty() {
                    let entry = self.driver.entry(record.id).map_err(detail)?;
                    let services = entry.services.lock().await;
                    direct::profile(
                        &services,
                        Some(record.metadata.published_name.as_str().into()),
                    )
                    .await?;
                }
                self.enable(record.id).await?;
            }
            if let Some(action) = self
                .coordinator
                .recovery_action(record.id)
                .map_err(detail)?
            {
                match action {
                    core::Action::Activate { .. } => self.activate(action).await?,
                    core::Action::Quiesce { .. } => self.quiesce(action, true).await?,
                    _ => {}
                }
            }
        }
        Ok(())
    }
    async fn create(&mut self, mut input: CreateNamedIdentityInput) -> Result<(), String> {
        input.published_name = input
            .published_name
            .map(|name| direct::normalize_name(&name))
            .transpose()?;
        if self.created_before(&input)? {
            return Ok(());
        }
        let command = core::Command::Create(core::CreateRequest {
            source: core::KeySource::Generate,
            metadata: core::Metadata::new(input.label.trim(), input.published_name.as_deref())
                .map_err(detail)?,
            services: core::ServiceSet::new(&[MESSAGING_SERVICE]).map_err(detail)?,
        });
        let digest = core::CommandDigest(input.request_id);
        let existing = self
            .coordinator
            .snapshot()
            .operations()
            .find(|o| o.token.digest == digest)
            .copied();
        let token = if let Some(existing) = existing {
            if existing.command != command {
                return Err("This create request already belongs to another identity.".into());
            }
            existing.token
        } else {
            self.coordinator
                .reserve(&mut self.disk, command, digest)
                .map_err(detail)?
        };
        if self.coordinator.operation(token).map_err(detail)?.state
            == core::OperationState::Reserved
        {
            self.coordinator
                .begin(&mut self.disk, token, &command)
                .map_err(detail)?;
        }
        let id = if let Some(core::Action::PublishKey { ticket, .. }) =
            self.coordinator.action(token).map_err(detail)?
        {
            let key = self
                .disk
                .publish_key(&self.coordinator, ticket, None)
                .map_err(detail)?;
            match self
                .coordinator
                .ack_key(&mut self.disk, ticket, key)
                .map_err(detail)?
            {
                core::OperationResult::Created { identity }
                | core::OperationResult::Existing { identity } => identity,
                _ => return Err("Unexpected publication result.".into()),
            }
        } else {
            match self.coordinator.operation(token).map_err(detail)?.state {
                core::OperationState::Completed(
                    core::OperationResult::Created { identity }
                    | core::OperationResult::Existing { identity },
                ) => identity,
                _ => return Err("Identity publication is still pending.".into()),
            }
        };
        #[cfg(test)]
        crash_checkpoint("created-key");
        self.install(id, true).await?;
        let record = *self.coordinator.identity(id).map_err(detail)?;
        if record.desired == core::DesiredState::Paused && record.lifecycle_revision == 1 {
            if let Some(name) = input.published_name {
                let entry = self.driver.entry(id).map_err(detail)?;
                let services = entry.services.lock().await;
                match direct::profile(&services, Some(name)).await? {
                    LocalMessagingProfileOutcome::Ready { .. } => {}
                    outcome => return Err(format!("Initial messaging name: {outcome:?}")),
                }
            }
            self.enable(id).await?;
        }
        if let Some(action @ core::Action::Activate { .. }) =
            self.coordinator.recovery_action(id).map_err(detail)?
        {
            self.activate(action).await?;
        }
        if self.selected.is_none() {
            self.select(NamedIdentityReference::from_record(
                self.coordinator.identity(id).map_err(detail)?,
            ))?;
        }
        Ok(())
    }
    fn transition_digest(&self, id: core::LocalIdentityId) -> core::CommandDigest {
        let mut input = Vec::new();
        input.extend_from_slice(&id.incarnation);
        input.extend_from_slice(&id.sequence.to_be_bytes());
        input.extend_from_slice(&self.coordinator.snapshot().revision.to_be_bytes());
        core::CommandDigest(prns_core::crypto::sha256(&input))
    }
    async fn enable(&mut self, id: core::LocalIdentityId) -> Result<(), String> {
        let dependency = self.driver.dependencies(id).await.map_err(detail)?;
        let report = core::DependencyReport::new(&[dependency], 0).map_err(detail)?;
        let command = core::Command::Transition(
            self.coordinator
                .prepare(id, core::TransitionTarget::Enable, report)
                .map_err(detail)?,
        );
        let digest = self.transition_digest(id);
        let token = self
            .coordinator
            .reserve(&mut self.disk, command, digest)
            .map_err(detail)?;
        self.coordinator
            .begin(&mut self.disk, token, &command)
            .map_err(detail)?;
        let action = self
            .coordinator
            .action(token)
            .map_err(detail)?
            .ok_or("Activation action missing")?;
        self.activate(action).await
    }
    async fn activate(&mut self, action: core::Action<1>) -> Result<(), String> {
        let receipt = {
            let authority = self.disk.runtime_authority(&self.coordinator);
            let owner = self
                .controller
                .begin_activation(&authority, action, MessagingDriver::plan())
                .map_err(detail)?;
            if self.controller.phase(owner).map_err(detail)? == runtime::Phase::FencingActivation {
                finish_step!(
                    self.controller
                        .fence_previous_workers(&authority, &mut self.driver, owner),
                    self.driver,
                    authority
                );
            }
            if self.controller.phase(owner).map_err(detail)? == runtime::Phase::Preflighting {
                finish_step!(
                    self.controller
                        .preflight(&authority, &mut self.driver, owner),
                    self.driver,
                    authority
                );
            }
            if self.controller.phase(owner).map_err(detail)? == runtime::Phase::Activating {
                finish_step!(
                    self.controller.prepare_service(
                        &authority,
                        &mut self.driver,
                        owner,
                        MESSAGING_SERVICE
                    ),
                    self.driver,
                    authority
                );
                finish_step!(
                    self.controller.activate_service(
                        &authority,
                        &mut self.driver,
                        owner,
                        MESSAGING_SERVICE
                    ),
                    self.driver,
                    authority
                );
            }
            self.controller
                .finish_activation(&authority, owner)
                .map_err(detail)?
        };
        self.coordinator
            .ack_activated(&mut self.disk, receipt.ticket())
            .map_err(detail)?;
        let record = *self
            .coordinator
            .identity(action.ticket().identity)
            .map_err(detail)?;
        let guard = self
            .coordinator
            .admit(record.id, record.public_hash, core::AdmissionClass::Network)
            .map_err(detail)?;
        let authority = self.disk.runtime_authority(&self.coordinator);
        self.controller
            .confirm_activation(&authority, receipt, guard)
            .map_err(detail)?;
        let permit = self
            .controller
            .network_permit(&authority, guard, MESSAGING_SERVICE)
            .map_err(detail)?;
        self.driver
            .start_network(&authority, &self.controller, permit)
            .await
            .map_err(detail)
    }
    async fn start_running(&mut self, id: core::LocalIdentityId) -> Result<(), String> {
        let record = *self.coordinator.identity(id).map_err(detail)?;
        let guard = self
            .coordinator
            .admit(id, record.public_hash, core::AdmissionClass::Network)
            .map_err(detail)?;
        let authority = self.disk.runtime_authority(&self.coordinator);
        if let Some(owner) = self.controller.owner(id) {
            let receipt = self.controller.activation_receipt(owner).map_err(detail)?;
            self.controller
                .confirm_activation(&authority, receipt, guard)
                .map_err(detail)?;
        }
        let permit = self
            .controller
            .network_permit(&authority, guard, MESSAGING_SERVICE)
            .map_err(detail)?;
        self.driver
            .start_network(&authority, &self.controller, permit)
            .await
            .map_err(detail)
    }
    async fn quiesce(&mut self, action: core::Action<1>, recovering: bool) -> Result<(), String> {
        let receipt = {
            let authority = self.disk.runtime_authority(&self.coordinator);
            let owner = if recovering {
                self.controller
                    .recover_quiescence(&authority, action, MessagingDriver::plan())
            } else {
                self.controller.begin_quiescence(&authority, action)
            }
            .map_err(detail)?;
            if self.controller.phase(owner).map_err(detail)?
                == runtime::Phase::FencingPreviousWorkers
            {
                finish_step!(
                    self.controller
                        .fence_previous_workers(&authority, &mut self.driver, owner),
                    self.driver,
                    authority
                );
            }
            if self.controller.phase(owner).map_err(detail)? == runtime::Phase::Quiescing {
                finish_step!(
                    self.controller.quiesce_service(
                        &authority,
                        &mut self.driver,
                        owner,
                        MESSAGING_SERVICE
                    ),
                    self.driver,
                    authority
                );
            }
            finish_step!(
                self.controller
                    .finish_quiescence(&authority, &mut self.driver, owner),
                self.driver,
                authority
            )
        };
        if receipt.ticket().operation.is_none() {
            self.coordinator
                .ack_recovery_quiesced(&mut self.disk, receipt.ticket(), receipt.report())
                .map_err(detail)?;
        } else {
            self.coordinator
                .ack_quiesced(&mut self.disk, receipt.ticket(), receipt.report())
                .map_err(detail)?;
        }
        self.controller
            .confirm_quiescence(&self.disk.runtime_authority(&self.coordinator), receipt)
            .map_err(detail)?;
        Ok(())
    }
    async fn change(&mut self, input: ChangeNamedIdentityInput) -> Result<(), String> {
        let record = self.check(&input.identity)?;
        match input.change {
            NamedIdentityChange::Select => self.select(input.identity),
            NamedIdentityChange::Rename { label } => self
                .coordinator
                .update_metadata(
                    &mut self.disk,
                    record.metadata_guard(),
                    core::Metadata::new(
                        label.trim(),
                        Some(record.metadata.published_name.as_str()),
                    )
                    .map_err(detail)?,
                )
                .map_err(detail),
            NamedIdentityChange::Resume => {
                if record.desired == core::DesiredState::Enabled {
                    if let Some(action @ core::Action::Activate { .. }) = self
                        .coordinator
                        .recovery_action(record.id)
                        .map_err(detail)?
                    {
                        return self.activate(action).await;
                    }
                    return self.start_running(record.id).await;
                }
                self.enable(record.id).await
            }
            NamedIdentityChange::Pause => {
                if let Some(action @ core::Action::Quiesce { .. }) = self
                    .coordinator
                    .recovery_action(record.id)
                    .map_err(detail)?
                {
                    return self.quiesce(action, false).await;
                }
                if record.desired == core::DesiredState::Paused && record.pending.is_none() {
                    return Ok(());
                }
                let report = core::DependencyReport::new(
                    &[self.driver.dependencies(record.id).await.map_err(detail)?],
                    0,
                )
                .map_err(detail)?;
                let review = self
                    .coordinator
                    .prepare(record.id, core::TransitionTarget::Pause, report)
                    .map_err(detail)?;
                let digest = self.transition_digest(record.id);
                let token = self
                    .coordinator
                    .apply_stop(&mut self.disk, review, digest)
                    .map_err(detail)?;
                #[cfg(test)]
                crash_checkpoint("pause-intent");
                let action = self
                    .coordinator
                    .action(token)
                    .map_err(detail)?
                    .ok_or("Pause action missing")?;
                self.driver
                    .recover_pending(&self.disk.runtime_authority(&self.coordinator), action)
                    .await
                    .map_err(detail)?;
                self.quiesce(action, false).await
            }
        }
    }
    fn select(&mut self, reference: NamedIdentityReference) -> Result<(), String> {
        self.check(&reference)?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&reference.incarnation);
        bytes.extend_from_slice(&reference.sequence.to_be_bytes());
        bytes.extend_from_slice(&reference.public_hash);
        receipts::replace_file(self.disk.root(), "selection", &bytes)?;
        self.selected = Some(reference);
        Ok(())
    }
    fn read_selection(&mut self) -> Result<(), String> {
        let path = self.disk.root().join("selection");
        if !path.try_exists().map_err(detail)? {
            if let Some(record) = self.coordinator.snapshot().identities().next() {
                self.selected = Some(NamedIdentityReference::from_record(record));
            }
            return Ok(());
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(detail)?;
        if !metadata.file_type().is_file() || metadata.len() != 40 {
            return Err("Invalid saved identity selection.".into());
        }
        let bytes = std::fs::read(path).map_err(detail)?;
        if bytes.len() != 40 {
            return Err("Invalid saved identity selection.".into());
        }
        let reference = NamedIdentityReference {
            incarnation: bytes[..16].try_into().map_err(detail)?,
            sequence: u64::from_be_bytes(bytes[16..24].try_into().map_err(detail)?),
            public_hash: bytes[24..].try_into().map_err(detail)?,
        };
        self.check(&reference)?;
        self.selected = Some(reference);
        Ok(())
    }
    async fn close(&mut self) -> Result<(), String> {
        let identities: Vec<_> = self
            .coordinator
            .snapshot()
            .identities()
            .map(|r| r.id)
            .collect();
        for id in identities {
            if let Ok(entry) = self.driver.entry(id) {
                let mut services = entry.services.lock().await;
                services.pause().await.map_err(detail)?;
                services.close_local().await.map_err(detail)?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
fn crash_checkpoint(point: &str) {
    if std::env::var("PRNS_NAMED_CRASH_POINT").ok().as_deref() == Some(point) {
        std::process::exit(77);
    }
}

#[cfg(test)]
mod concurrency_tests;
