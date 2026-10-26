//! Bounded native jobs: one service lock per identity, independently scheduled.
//! Coordinator authority and mutation settlement stay with the owning actor.
use super::*;
use std::collections::VecDeque;
use tokio::task::{JoinError, JoinSet};

const DEFERRED_LIMIT: usize = 32;
pub(super) struct Completion {
    pub identity: core::LocalIdentityId,
    pub lease: Option<runtime::MutationLease>,
    pub announce: Option<(prns_identities_native::announces::Reservation, u64)>,
}
pub(super) struct Jobs {
    pub tasks: JoinSet<Completion>,
    pub deferred: VecDeque<Request>,
    active: Vec<core::LocalIdentityId>,
    reviews: Vec<(core::LocalIdentityId, core::ServiceDependency)>,
}
impl Jobs {
    pub fn new() -> Self {
        Self {
            tasks: JoinSet::new(),
            deferred: VecDeque::new(),
            active: Vec::new(),
            reviews: Vec::new(),
        }
    }
    pub fn busy(&self, id: core::LocalIdentityId) -> bool {
        self.active.contains(&id)
    }
    pub fn spawn(
        &mut self,
        id: core::LocalIdentityId,
        work: impl Future<Output = Completion> + Send + 'static,
    ) {
        self.active.push(id);
        self.tasks.spawn(work);
    }
}
fn request_identity(request: &Request) -> Option<core::LocalIdentityId> {
    match request {
        Request::Service { identity, .. } => Some(identity.id()),
        Request::Change(input, _) => Some(input.identity.id()),
        Request::Announces(announces::AnnounceRequest::Settings { identity, .. }) => {
            Some(identity.id())
        }
        _ => None,
    }
}
pub(super) async fn reject(request: Request, message: &str) {
    match request {
        Request::Catalog(reply) | Request::Create(_, reply) | Request::Change(_, reply) => {
            let _ = reply.send(NamedIdentityCatalogOutcome::Unavailable {
                detail: message.into(),
            });
        }
        Request::Service { work, .. } | Request::Activity { work, .. } => {
            work(Err(message.into())).await
        }
        Request::Announces(announces::AnnounceRequest::Settings { reply, .. }) => {
            let _ = reply.send(GroupAnnounceSettingsOutcome::Unavailable {
                detail: message.into(),
            });
        }
        Request::Announces(announces::AnnounceRequest::Manual { reply, .. }) => {
            let _ = reply.send(announces::ManualOutcome::Unavailable(message.into()));
        }
        Request::Stop(reply) => {
            let _ = reply.send(Err(message.into()));
        }
    }
}
impl Actor<'_> {
    pub(super) fn ready_deferred(&mut self) -> Option<Request> {
        let index =
            self.jobs.deferred.iter().position(|request| {
                request_identity(request).is_none_or(|id| !self.jobs.busy(id))
            })?;
        self.jobs.deferred.remove(index)
    }
    pub(super) fn finish_job(
        &mut self,
        result: Result<Completion, JoinError>,
    ) -> Result<(), String> {
        // A panicked worker cannot prove mutation settlement. Fail closed and
        // drain the remaining jobs; restart performs durable service recovery.
        let completion = result.map_err(detail)?;
        self.jobs.active.retain(|id| *id != completion.identity);
        if let Some(lease) = completion.lease {
            self.controller
                .complete_mutation(lease, runtime::Settlement::Committed)
                .map_err(detail)?;
        }
        if let Some((attempt, requested)) = completion.announce {
            self.complete_announce(attempt, requested)?;
        }
        Ok(())
    }
    pub(super) fn fence_jobs(&self) {
        for record in self.coordinator.snapshot().identities() {
            let _ = self.driver.fence_network_admission(record.id);
        }
    }
    async fn defer(&mut self, request: Request) {
        if self.jobs.deferred.len() < DEFERRED_LIMIT {
            self.jobs.deferred.push_back(request);
        } else {
            reject(
                request,
                "This identity's bounded command lane is busy. Retry shortly.",
            )
            .await;
        }
    }
    pub(super) async fn cache_dependency_review(
        &mut self,
        id: core::LocalIdentityId,
    ) -> Result<(), String> {
        let report = self.driver.dependencies(id).await.map_err(detail)?;
        if let Some(entry) = self.jobs.reviews.iter_mut().find(|entry| entry.0 == id) {
            entry.1 = report;
        } else {
            self.jobs.reviews.push((id, report));
        }
        Ok(())
    }
    /// Pause can durably close admission while an earlier service operation
    /// holds its mutex. This prior actual review is only used for Pause intent;
    /// final quiescence always reads authoritative dependencies after draining.
    fn fence_busy_pause(&mut self, reference: &NamedIdentityReference) -> Result<(), String> {
        let record = self.check(reference)?;
        if record.desired == core::DesiredState::Paused && record.pending.is_none() {
            return Ok(());
        }
        if !matches!(
            self.coordinator
                .recovery_action(record.id)
                .map_err(detail)?,
            Some(core::Action::Quiesce { .. })
        ) {
            let report = self
                .jobs
                .reviews
                .iter()
                .find(|entry| entry.0 == record.id)
                .map(|entry| entry.1)
                .ok_or("Missing admitted-job dependency review")?;
            let review = self
                .coordinator
                .prepare(
                    record.id,
                    core::TransitionTarget::Pause,
                    core::DependencyReport::new(&[report], 0).map_err(detail)?,
                )
                .map_err(detail)?;
            let digest = self.transition_digest(record.id);
            self.coordinator
                .apply_stop(&mut self.disk, review, digest)
                .map_err(detail)?;
            #[cfg(test)]
            crash_checkpoint("pause-intent");
        }
        self.driver
            .fence_network_admission(record.id)
            .map_err(detail)
    }
    pub(super) async fn handle_request(&mut self, request: Request) {
        match request {
            Request::Activity { identity, work } => {
                if self.jobs.busy(identity.id()) {
                    work(Err(
                        "This identity is busy; activity is temporarily unavailable.".into(),
                    ))
                    .await;
                } else {
                    self.service(identity, core::AdmissionClass::History, work)
                        .await;
                }
            }
            Request::Catalog(reply) => {
                let _ = reply.send(self.catalog_result(Ok(())));
            }
            Request::Create(input, reply) => {
                // An exact create retry may need an existing owner's activation
                // lock. Keep it bounded instead of blocking unrelated owners.
                if !self.jobs.active.is_empty() {
                    let _ = reply.send(NamedIdentityCatalogOutcome::Busy);
                    return;
                }
                let before = self.coordinator.snapshot().revision;
                let result = self.create(input).await;
                let outcome = self.finished_command(result, before).await;
                let _ = reply.send(outcome);
            }
            Request::Change(input, reply) => {
                if self.jobs.busy(input.identity.id()) {
                    match input.change {
                        NamedIdentityChange::Pause => {
                            if self.jobs.deferred.len() == DEFERRED_LIMIT {
                                let _ = reply.send(NamedIdentityCatalogOutcome::Busy);
                                return;
                            }
                            let before = self.coordinator.snapshot().revision;
                            if let Err(error) = self.fence_busy_pause(&input.identity) {
                                let _ = reply.send(self.command_failure(error, before));
                                return;
                            }
                            if let Err(detail) = self.reconcile_announces().await {
                                let _ =
                                    reply.send(NamedIdentityCatalogOutcome::Unavailable { detail });
                                return;
                            }
                            // Quiescence belongs ahead of earlier unadmitted
                            // reads. New sends already fail the durable gate.
                            self.jobs.deferred.push_front(Request::Change(input, reply));
                            return;
                        }
                        NamedIdentityChange::Resume => {
                            let _ = reply.send(NamedIdentityCatalogOutcome::Busy);
                            return;
                        }
                        NamedIdentityChange::Select | NamedIdentityChange::Rename { .. } => {}
                    }
                }
                let before = self.coordinator.snapshot().revision;
                let result = self.change(input).await;
                let outcome = self.finished_command(result, before).await;
                let _ = reply.send(outcome);
            }
            Request::Service {
                identity,
                class,
                work,
            } => self.service(identity, class, work).await,
            Request::Announces(request @ announces::AnnounceRequest::Settings { .. }) => {
                if let announces::AnnounceRequest::Settings { identity, .. } = &request {
                    if self.jobs.busy(identity.id()) {
                        self.defer(Request::Announces(request)).await;
                        return;
                    }
                }
                self.announce_request(request).await;
            }
            Request::Announces(request) => self.announce_request(request).await,
            Request::Stop(reply) => {
                let _ = reply.send(Err("Stop must be handled by the owner loop.".into()));
            }
        }
    }
    async fn finished_command(
        &mut self,
        result: Result<(), String>,
        before: u64,
    ) -> NamedIdentityCatalogOutcome {
        match result {
            Err(error) => self.command_failure(error, before),
            Ok(()) => match self.retire_operations() {
                Err(detail) => NamedIdentityCatalogOutcome::Unavailable { detail },
                Ok(()) => match self.reconcile_announces().await {
                    Ok(()) => self.catalog_result(Ok(())),
                    Err(detail) => NamedIdentityCatalogOutcome::Unavailable { detail },
                },
            },
        }
    }
    async fn service(
        &mut self,
        reference: NamedIdentityReference,
        class: core::AdmissionClass,
        work: Work,
    ) {
        let admission = (|| {
            self.check(&reference)?;
            self.coordinator
                .admit(
                    reference.id(),
                    core::PublicIdentityHash(reference.public_hash),
                    class,
                )
                .map_err(detail)
        })();
        let guard = match admission {
            Ok(guard) => guard,
            Err(error) => {
                work(Err(error)).await;
                return;
            }
        };
        if self.jobs.busy(reference.id()) {
            self.defer(Request::Service {
                identity: reference,
                class,
                work,
            })
            .await;
            return;
        }
        if let Err(error) = self.cache_dependency_review(reference.id()).await {
            work(Err(error)).await;
            return;
        }
        let services = match self.driver.entry(reference.id()) {
            Ok(entry) => entry.services.clone(),
            Err(error) => {
                work(Err(detail(error))).await;
                return;
            }
        };
        let lease = if matches!(
            class,
            core::AdmissionClass::ServiceMutation | core::AdmissionClass::Network
        ) {
            let Some(sequence) = self.mutation_sequence.checked_add(1) else {
                work(Err("Identity mutation counter exhausted.".into())).await;
                return;
            };
            self.mutation_sequence = sequence;
            let mut id = [0; 16];
            id[8..].copy_from_slice(&sequence.to_be_bytes());
            match self.controller.admit_mutation(
                &self.disk.runtime_authority(&self.coordinator),
                guard,
                MESSAGING_SERVICE,
                runtime::MutationId(id),
            ) {
                Ok(lease) => Some(lease),
                Err(error) => {
                    work(Err(detail(error))).await;
                    return;
                }
            }
        } else {
            None
        };
        let identity = reference.id();
        self.jobs.spawn(identity, async move {
            let mut services = services.lock().await;
            match services.prepare().await {
                Ok(()) => work(Ok(&mut services)).await,
                Err(error) => work(Err(detail(error))).await,
            }
            Completion {
                identity,
                lease,
                announce: None,
            }
        });
    }
}
