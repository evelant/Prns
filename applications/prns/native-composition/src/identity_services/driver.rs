//! Poll bridge from the allocation-free controller to caller-driven native work.
use super::*;
use prns_identities_core::{DependencyReport, LocalIdentityId};
use prns_identities_runtime::*;
use std::{future::Future, pin::Pin};

type WorkFuture = Pin<Box<dyn Future<Output = Result<Reply, super::Error>> + Send>>;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Fence,
    Preflight,
    Prepare(KeyReference),
    Register,
    Pause,
    Release,
}
enum Reply {
    Fence(bool),
    Resources(Resources),
    Prepared(PreparedService),
    Registered,
    Paused(QuiescedService),
    Released(ReleasedIdentity<1>),
}
struct Pending {
    scope: Scope,
    work: Work,
    future: Option<WorkFuture>,
    reply: Option<Reply>,
}
pub(crate) struct Entry {
    id: LocalIdentityId,
    pub(crate) services: Arc<tokio::sync::Mutex<MessagingIdentity>>,
    last_activation: Option<Scope>,
    events: Arc<Mutex<Events>>,
}

/// One serialized native driver with a caller-chosen bound. Keep it alongside
/// the caller-owned RuntimeState through uncertain acknowledgements. Each pending
/// future is retained here across cancellation of `advance`; do not replace the
/// driver while a live host still has any of its service resources.
pub struct MessagingDriver {
    entries: Vec<Entry>,
    maximum: usize,
    pending: Option<Pending>,
}
impl MessagingDriver {
    pub fn new(maximum: usize) -> Result<Self, super::Error> {
        if maximum == 0 || maximum > 64 {
            return Err(error("driver capacity"));
        }
        Ok(Self {
            entries: Vec::new(),
            maximum,
            pending: None,
        })
    }
    pub fn install(&mut self, identity: MessagingIdentity) -> Result<(), super::Error> {
        let id = identity.storage_scope.identity;
        if self.entries.len() == self.maximum || self.entries.iter().any(|entry| entry.id == id) {
            return Err(error("duplicate identity or full driver"));
        }
        let events = identity.events.clone();
        self.entries.push(Entry {
            id,
            services: Arc::new(tokio::sync::Mutex::new(identity)),
            last_activation: None,
            events,
        });
        Ok(())
    }
    /// Fence fresh network ingress/dispatch without waiting for a service job's
    /// mutex. Already-admitted direct callbacks remain enabled until endpoint
    /// retirement drains their durable writes through the ordinary pause path.
    pub(crate) fn fence_network_admission(&self, id: LocalIdentityId) -> Result<(), super::Error> {
        let mut events = self.entry(id)?.events.lock().map_err(error)?;
        events.enabled = false;
        if let Some(scope) = &events.scope {
            scope.fence();
        }
        Ok(())
    }
    pub(crate) fn entry(&self, id: LocalIdentityId) -> Result<&Entry, super::Error> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| error("identity not installed"))
    }
    pub(crate) fn cached_published_name(
        &self,
        id: LocalIdentityId,
    ) -> Result<Option<String>, super::Error> {
        Ok(self
            .entry(id)?
            .events
            .lock()
            .map_err(error)?
            .published_name
            .clone())
    }
    pub fn groups(
        &self,
        id: LocalIdentityId,
    ) -> impl Future<Output = Result<GroupClient, super::Error>> + Send + 'static {
        let services = self.entry(id).map(|entry| entry.services.clone());
        async move { services?.lock().await.groups() }
    }
    pub fn direct(
        &self,
        id: LocalIdentityId,
    ) -> impl Future<Output = Result<Arc<DurableDirectLxmfService>, super::Error>> + Send + 'static
    {
        let services = self.entry(id).map(|entry| entry.services.clone());
        async move { services?.lock().await.direct() }
    }
    pub fn announcements(
        &self,
        id: LocalIdentityId,
    ) -> impl Future<Output = Result<prns_groups_runner_native::AnnounceControl, super::Error>>
           + Send
           + 'static {
        let services = self.entry(id).map(|entry| entry.services.clone());
        async move { services?.lock().await.announcements() }
    }
    pub fn dependencies(
        &self,
        id: LocalIdentityId,
    ) -> impl Future<Output = Result<ServiceDependency, super::Error>> + Send + 'static {
        let services = self.entry(id).map(|entry| entry.services.clone());
        async move {
            let services = services?;
            let entry = services.lock().await;
            if entry.storage_close_failed {
                return Err(error("storage close failed; restart recovery required"));
            }
            if entry.store.is_some() {
                entry.observe_dependencies().await
            } else {
                entry
                    .dependencies
                    .ok_or_else(|| error("dependency report missing"))
            }
        }
    }
    /// Native plan bounds owned registrations, outgoing Links and admitted host
    /// operations. Incoming Links/requests also consume shared engine capacity;
    /// this adapter does not yet enforce per-identity inbound quotas. Reserve
    /// management separately and do not treat this as adversarial resource isolation.
    pub fn plan() -> ActivationPlan<1> {
        ActivationPlan {
            identity_resources: Resources::ZERO,
            services: [Some(ServicePlan {
                service: MESSAGING_SERVICE,
                resources: Resources {
                    held_keys: 3,
                    destinations: 3,
                    request_handlers: 2,
                    links: MAX_LINKS as u32,
                    queue_slots: MAX_OPERATIONS as u32,
                },
                requires_receive_state: false,
                capabilities: Capabilities::QUALIFIED,
            })],
        }
    }
    /// Execute the queued controller step while holding the embedding's exclusive
    /// lifecycle gate. NativeStore::runtime_authority must be used when available.
    /// This borrow of authority must cover the complete call. On cancellation,
    /// call again with current authority; the actual future is retained.
    pub async fn advance<A: Authority>(&mut self, authority: &A) -> Result<(), super::Error> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| error("no pending step"))?;
        authority
            .validate_effect(pending.scope.lifecycle)
            .map_err(error)?;
        if pending.reply.is_some() {
            return Ok(());
        }
        if pending.future.is_none() {
            let scope = pending.scope;
            let work = pending.work;
            let services = self.entry(scope.identity.id)?.services.clone();
            let future = Box::pin(async move {
                let mut entry = services.lock().await;
                if entry.storage_scope.identity != scope.identity.id
                    || entry.storage_scope.public_hash != scope.identity.public_hash
                {
                    return Err(error("wrong immutable service owner"));
                }
                match work {
                    Work::Fence => {
                        if entry.storage_close_failed
                            || entry.network.is_some()
                            || !entry.endpoints.is_empty()
                        {
                            entry.pause().await?;
                        }
                        Ok(Reply::Fence(true))
                    }
                    Work::Preflight => Ok(Reply::Resources(Self::plan().total().map_err(error)?)),
                    Work::Prepare(key) => {
                        if key != entry.key_reference {
                            return Err(error("key reference mismatch"));
                        }
                        entry.prepare().await?;
                        Ok(Reply::Prepared(PreparedService {
                            storage_ready: true,
                            receive_state_restored: true,
                        }))
                    }
                    Work::Register => {
                        entry.register().await?;
                        Ok(Reply::Registered)
                    }
                    Work::Pause => {
                        let dependencies = entry.pause().await?;
                        Ok(Reply::Paused(QuiescedService {
                            evidence: QuiescenceEvidence::COMPLETE,
                            dependencies,
                        }))
                    }
                    Work::Release => {
                        if entry.storage_close_failed
                            || entry.registered != 0
                            || entry.network.is_some()
                            || entry.store.is_some()
                        {
                            return Err(error("identity still owns live work"));
                        }
                        let dependencies = entry
                            .dependencies
                            .ok_or_else(|| error("missing final store observation"))?;
                        Ok(Reply::Released(ReleasedIdentity {
                            owned_reservations_released: true,
                            shared_owners_preserved: true,
                            report: DependencyReport::new(&[dependencies], 0).map_err(error)?,
                        }))
                    }
                }
            });
            self.pending
                .as_mut()
                .ok_or_else(|| error("pending step disappeared"))?
                .future = Some(future);
        }
        let pending = self
            .pending
            .as_mut()
            .ok_or_else(|| error("pending step disappeared"))?;
        let result = pending
            .future
            .as_mut()
            .ok_or_else(|| error("pending future missing"))?
            .await;
        pending.future = None;
        pending.reply = Some(result?);
        Ok(())
    }
    /// Reconcile a pending async step after an authoritative catalog reload.
    /// Already-started work settles under its original owner, then its complete
    /// service resources are fenced/drained before the fresh action can proceed.
    /// Caller-owned RuntimeState and mutation leases are never reset here.
    pub async fn recover_pending<A: Authority>(
        &mut self,
        authority: &A,
        action: prns_identities_core::Action<1>,
    ) -> Result<(), super::Error> {
        authority.validate_action(&action).map_err(error)?;
        let Some(pending) = &self.pending else {
            return Ok(());
        };
        if pending.scope.identity.id != action.ticket().identity
            || (pending.scope.lifecycle.session >= action.ticket().session
                && pending.scope.lifecycle.lifecycle_revision >= action.ticket().lifecycle_revision)
        {
            return Err(error(
                "pending recovery needs a newer authority for the same identity",
            ));
        }
        let services = self.entry(action.ticket().identity)?.services.clone();
        {
            // A retained future may currently hold the async service lock.
            // Fence independently, then poll it to completion before reacquiring.
            let mut events = self
                .entry(action.ticket().identity)?
                .events
                .lock()
                .map_err(error)?;
            events.enabled = false;
            if let Some(scope) = &events.scope {
                scope.fence();
            }
        }
        if let Some(future) = self
            .pending
            .as_mut()
            .and_then(|pending| pending.future.as_mut())
        {
            let _settled = future.await;
            self.pending
                .as_mut()
                .ok_or_else(|| error("pending disappeared"))?
                .future = None;
        }
        services.lock().await.pause().await?;
        self.pending = None;
        Ok(())
    }
    /// Unlock outgoing work only after BOTH catalog acknowledgement and runtime
    /// confirmation. Each new activation uses a fresh scope; old clients stay fenced.
    pub async fn start_network<A: Authority, const I: usize, const L: usize>(
        &mut self,
        authority: &A,
        runtime: &Controller<'_, I, 1, L>,
        permit: NetworkPermit,
    ) -> Result<(), super::Error> {
        runtime.validate_network(authority, permit).map_err(error)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| {
                entry.last_activation.is_some_and(|scope| {
                    scope.owner == permit.owner() && scope.generation == permit.generation()
                })
            })
            .ok_or_else(|| error("wrong network generation"))?;
        entry.services.lock().await.start_network().await
    }
    fn poll(&mut self, scope: Scope, work: Work) -> Result<Option<Reply>, super::Error> {
        self.entry(scope.identity.id)?;
        if let Some(pending) = &mut self.pending {
            if pending.scope != scope || pending.work != work {
                return Err(error("another lifecycle step is pending"));
            }
            if let Some(reply) = pending.reply.take() {
                self.pending = None;
                return Ok(Some(reply));
            }
        } else {
            self.pending = Some(Pending {
                scope,
                work,
                future: None,
                reply: None,
            });
        }
        Ok(None)
    }
}
impl ScopedDriver<1> for MessagingDriver {
    type Error = super::Error;
    fn preflight(
        &mut self,
        scope: Scope,
        plan: &ActivationPlan<1>,
    ) -> Result<Progress<Evidence<Resources>>, Self::Error> {
        if *plan != Self::plan() {
            return Err(error("unsupported service plan"));
        }
        match self.poll(scope, Work::Preflight)? {
            Some(Reply::Resources(value)) => Ok(Progress::Complete(Evidence { scope, value })),
            None => Ok(Progress::Pending),
            _ => Err(error("step reply")),
        }
    }
    fn prepare_service(
        &mut self,
        scope: Scope,
        key: KeyReference,
        _plan: ServicePlan,
    ) -> Result<Progress<Evidence<PreparedService>>, Self::Error> {
        match self.poll(scope, Work::Prepare(key))? {
            Some(Reply::Prepared(value)) => Ok(Progress::Complete(Evidence { scope, value })),
            None => Ok(Progress::Pending),
            _ => Err(error("step reply")),
        }
    }
    fn activate_service(
        &mut self,
        scope: Scope,
        _plan: ServicePlan,
    ) -> Result<Progress<Evidence<()>>, Self::Error> {
        match self.poll(scope, Work::Register)? {
            Some(Reply::Registered) => {
                let entry = self
                    .entries
                    .iter_mut()
                    .find(|entry| entry.id == scope.identity.id)
                    .ok_or_else(|| error("owner missing"))?;
                entry.last_activation = Some(scope);
                Ok(Progress::Complete(Evidence { scope, value: () }))
            }
            None => Ok(Progress::Pending),
            _ => Err(error("step reply")),
        }
    }
    fn fence_previous_workers(
        &mut self,
        scope: Scope,
    ) -> Result<Progress<Evidence<bool>>, Self::Error> {
        match self.poll(scope, Work::Fence)? {
            Some(Reply::Fence(value)) => Ok(Progress::Complete(Evidence { scope, value })),
            None => Ok(Progress::Pending),
            _ => Err(error("step reply")),
        }
    }
    fn quiesce_service(
        &mut self,
        scope: Scope,
        _plan: ServicePlan,
    ) -> Result<Progress<Evidence<QuiescedService>>, Self::Error> {
        match self.poll(scope, Work::Pause)? {
            Some(Reply::Paused(value)) => Ok(Progress::Complete(Evidence { scope, value })),
            None => Ok(Progress::Pending),
            _ => Err(error("step reply")),
        }
    }
    fn release_identity(
        &mut self,
        scope: Scope,
    ) -> Result<Progress<Evidence<ReleasedIdentity<1>>>, Self::Error> {
        match self.poll(scope, Work::Release)? {
            Some(Reply::Released(value)) => Ok(Progress::Complete(Evidence { scope, value })),
            None => Ok(Progress::Pending),
            _ => Err(error("step reply")),
        }
    }
}
