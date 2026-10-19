//! One unsolicited announce owner across all named identities on this host.
//! Discovery path responses stay under the unchanged Reticulum protocol rules.
use super::*;
use prns_identities_native::announces::{AnnounceKey, ManualRequest, NativeAnnounces, Reservation};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const GROUP_SERVICE: core::ServiceId = core::ServiceId(1);
const DIRECT_SERVICE: core::ServiceId = core::ServiceId(2);
const SYNC: u64 = 1;
const ENROLLMENT: u64 = 2;
const GROUP_CHANNELS: u64 = SYNC | ENROLLMENT;
const SLOTS: usize = MAX_IDENTITIES * 2;

pub(super) struct SharedAnnounces {
    owner: NativeAnnounces<SLOTS>,
    origin: Instant,
    last_error: Option<String>,
}
impl SharedAnnounces {
    pub(super) fn open(root: &Path) -> Result<Self, String> {
        let path = root.join("named-announces");
        let owner = if path.try_exists().map_err(detail)? {
            NativeAnnounces::open(path, 0, wall_time())
        } else {
            NativeAnnounces::create_new(path, 0, wall_time())
        }
        .map_err(detail)?;
        Ok(Self {
            owner,
            origin: Instant::now(),
            last_error: None,
        })
    }
    fn now(&self) -> u64 {
        self.origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
}
fn wall_time() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
}
fn key(identity: core::LocalIdentityId, service: core::ServiceId) -> AnnounceKey {
    AnnounceKey { identity, service }
}

pub(super) enum AnnounceRequest {
    Settings {
        identity: NamedIdentityReference,
        update: Option<GroupAnnounceSettings>,
        reply: oneshot::Sender<GroupAnnounceSettingsOutcome>,
    },
    Manual {
        identity: NamedIdentityReference,
        service: core::ServiceId,
        reply: oneshot::Sender<ManualOutcome>,
    },
}
pub(super) enum ManualOutcome {
    Queued,
    Throttled(u64),
    Unavailable(String),
}

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl NamedIdentityClient {
    pub async fn get_group_announce_settings(&self) -> GroupAnnounceSettingsOutcome {
        self.announce_settings(None).await
    }
    pub async fn set_group_announce_settings(
        &self,
        settings: GroupAnnounceSettings,
    ) -> GroupAnnounceSettingsOutcome {
        self.announce_settings(Some(settings)).await
    }
    pub async fn announce_groups(&self) -> GroupAnnounceOutcome {
        match self.manual_announce(GROUP_SERVICE).await {
            Ok(ManualOutcome::Queued) => GroupAnnounceOutcome::Queued,
            Ok(ManualOutcome::Throttled(retry_after_millis)) => {
                GroupAnnounceOutcome::Throttled { retry_after_millis }
            }
            Ok(ManualOutcome::Unavailable(detail)) | Err(detail) => {
                GroupAnnounceOutcome::Unavailable { detail }
            }
        }
    }
    pub async fn announce_lxmf(&self) -> AnnounceLxmfOutcome {
        match self.manual_announce(DIRECT_SERVICE).await {
            Ok(ManualOutcome::Queued) => AnnounceLxmfOutcome::Queued,
            Ok(ManualOutcome::Throttled(_)) => AnnounceLxmfOutcome::Throttled,
            Ok(ManualOutcome::Unavailable(_)) => AnnounceLxmfOutcome::Failed,
            Err(_) => AnnounceLxmfOutcome::LocalNodeStopped,
        }
    }
}
impl NamedIdentityClient {
    async fn announce_settings(
        &self,
        update: Option<GroupAnnounceSettings>,
    ) -> GroupAnnounceSettingsOutcome {
        let owner = match global() {
            Ok(owner) => owner,
            Err(detail) => return GroupAnnounceSettingsOutcome::Unavailable { detail },
        };
        let (reply, receive) = oneshot::channel();
        if owner
            .commands
            .try_send(Request::Announces(AnnounceRequest::Settings {
                identity: self.identity.clone(),
                update,
                reply,
            }))
            .is_err()
        {
            return GroupAnnounceSettingsOutcome::Busy;
        }
        receive
            .await
            .unwrap_or_else(|_| GroupAnnounceSettingsOutcome::Unavailable {
                detail: "The identity owner stopped.".into(),
            })
    }
    async fn manual_announce(&self, service: core::ServiceId) -> Result<ManualOutcome, String> {
        let (reply, receive) = oneshot::channel();
        global()?
            .commands
            .try_send(Request::Announces(AnnounceRequest::Manual {
                identity: self.identity.clone(),
                service,
                reply,
            }))
            .map_err(|_| "The identity command lane is busy or stopped.".to_owned())?;
        receive
            .await
            .map_err(|_| "The identity owner stopped.".to_owned())
    }
}

impl Actor<'_> {
    pub(super) async fn announce_request(&mut self, request: AnnounceRequest) {
        match request {
            AnnounceRequest::Settings {
                identity,
                update,
                reply,
            } => {
                let outcome = if update
                    .as_ref()
                    .is_some_and(|value| !crate::group_announces::validate(value.interval_minutes))
                {
                    GroupAnnounceSettingsOutcome::InvalidInput {
                        detail: "Choose manual only, 15 or 30 minutes, or 1, 3 or 6 hours.".into(),
                    }
                } else {
                    match self.saved_announce_settings(&identity, update).await {
                        Ok(settings) => {
                            let result = self.configure_announces(identity.id(), &settings);
                            match result {
                                Ok(()) => GroupAnnounceSettingsOutcome::Ready { settings },
                                Err(detail) => GroupAnnounceSettingsOutcome::Unavailable { detail },
                            }
                        }
                        Err(detail) => GroupAnnounceSettingsOutcome::Unavailable { detail },
                    }
                };
                let _ = reply.send(outcome);
            }
            AnnounceRequest::Manual {
                identity,
                service,
                reply,
            } => {
                let outcome = self
                    .manual_announce_for(&identity, service)
                    .await
                    .unwrap_or_else(ManualOutcome::Unavailable);
                let _ = reply.send(outcome);
            }
        }
    }

    async fn saved_announce_settings(
        &mut self,
        reference: &NamedIdentityReference,
        update: Option<GroupAnnounceSettings>,
    ) -> Result<GroupAnnounceSettings, String> {
        let record = self.check(reference)?;
        self.coordinator
            .admit(
                record.id,
                record.public_hash,
                if update.is_some() {
                    core::AdmissionClass::LocalMutation
                } else {
                    core::AdmissionClass::History
                },
            )
            .map_err(detail)?;
        let entry = self.driver.entry(record.id).map_err(detail)?;
        let mut services = entry.services.lock().await;
        services.prepare().await.map_err(detail)?;
        // Refresh the actor's synchronous catalog projection from the scoped
        // profile, never from the initial identity creation metadata.
        direct::profile(&services, None).await?;
        let (reply, receive) = oneshot::channel();
        services
            .store()
            .map_err(detail)?
            .admit_group_announces(update, move |result| {
                let _ = reply.send(result);
            })
            .map_err(detail)?;
        receive.await.map_err(detail)?.map_err(detail)
    }

    fn configure_announces(
        &mut self,
        identity: core::LocalIdentityId,
        settings: &GroupAnnounceSettings,
    ) -> Result<(), String> {
        let record = self.coordinator.identity(identity).map_err(detail)?;
        let enabled = self
            .coordinator
            .admit(identity, record.public_hash, core::AdmissionClass::Network)
            .is_ok();
        let now = self.announces.now();
        let wall = wall_time();
        self.announces
            .owner
            .configure(
                key(identity, GROUP_SERVICE),
                GROUP_CHANNELS,
                settings
                    .interval_minutes
                    .map(|minutes| u64::from(minutes) * 60_000),
                enabled,
                now,
                wall,
            )
            .map_err(detail)?;
        self.announces
            .owner
            .configure(key(identity, DIRECT_SERVICE), 1, None, enabled, now, wall)
            .map_err(detail)
    }

    /// Refresh lifecycle gates before every batch; no per-timer writes when
    /// unchanged. The identity-local database remains authoritative for prefs.
    pub(super) async fn reconcile_announces(&mut self) -> Result<(), String> {
        let records: Vec<_> = self.coordinator.snapshot().identities().copied().collect();
        for record in records {
            let cached = self
                .announces
                .owner
                .snapshot()
                .entries
                .iter()
                .flatten()
                .find(|entry| entry.key == key(record.id, GROUP_SERVICE))
                .map(|entry| entry.configuration.interval_ms);
            let settings = match cached {
                Some(interval) => GroupAnnounceSettings {
                    interval_minutes: interval.map(|millis| (millis / 60_000) as u32),
                },
                None => {
                    self.saved_announce_settings(
                        &NamedIdentityReference::from_record(&record),
                        None,
                    )
                    .await?
                }
            };
            self.configure_announces(record.id, &settings)?;
        }
        Ok(())
    }

    /// Startup reload must re-read identity-local preferences even when an
    /// interrupted prior save left a different scheduler configuration on disk.
    pub(super) async fn restore_announces(&mut self) -> Result<(), String> {
        let records: Vec<_> = self.coordinator.snapshot().identities().copied().collect();
        for record in records {
            let settings = self
                .saved_announce_settings(&NamedIdentityReference::from_record(&record), None)
                .await?;
            self.configure_announces(record.id, &settings)?;
        }
        Ok(())
    }

    async fn manual_announce_for(
        &mut self,
        reference: &NamedIdentityReference,
        service: core::ServiceId,
    ) -> Result<ManualOutcome, String> {
        let record = self.check(reference)?;
        self.coordinator
            .admit(record.id, record.public_hash, core::AdmissionClass::Network)
            .map_err(detail)?;
        self.reconcile_announces().await?;
        let requested_key = key(record.id, service);
        match self
            .announces
            .owner
            .request_manual(requested_key, self.announces.now(), wall_time())
            .map_err(detail)?
        {
            ManualRequest::Disabled => Err("Resume this identity before announcing it.".into()),
            ManualRequest::Throttled { retry_after_ms } => {
                Ok(ManualOutcome::Throttled(retry_after_ms))
            }
            ManualRequest::Queued => {
                self.drive_announce().await?;
                Ok(ManualOutcome::Queued)
            }
        }
    }

    pub(super) async fn announce_tick(&mut self) {
        let result = async {
            self.reconcile_announces().await?;
            self.drive_announce().await.map(|_| ())
        }
        .await;
        match result {
            Ok(()) => self.announces.last_error = None,
            Err(error) => {
                if self.announces.last_error.as_ref() != Some(&error) {
                    log::warn!("Named identity announcements: {error}");
                }
                self.announces.last_error = Some(error);
            }
        }
    }

    async fn drive_announce(&mut self) -> Result<Option<Reservation>, String> {
        // An actual online transmit-capable external interface is required.
        // Merely having a configured interface must not consume startup work.
        let bluetooth_ready = crate::lifecycle::named_announce_bluetooth_ready();
        let eligible = self
            .host
            .protocol_operation(move |handle| async move {
                crate::lxmf::has_usable_announce_connection(
                    &handle.interface_timing_inventory(),
                    bluetooth_ready,
                )
            })
            .await
            .map_err(detail)?;
        let mut entropy = [0; 2];
        getrandom::getrandom(&mut entropy).map_err(detail)?;
        let Some(attempt) = self
            .announces
            .owner
            .reserve_next_matching(
                self.announces.now(),
                wall_time(),
                eligible,
                u16::from_le_bytes(entropy),
                |key| !self.jobs.busy(key.identity),
            )
            .map_err(detail)?
        else {
            return Ok(None);
        };
        // The reservation is already durable. The actor owns this task through
        // completion, including caller cancellation and Pause/Stop drainage.
        let dispatch = self.prepare_announce(attempt).await;
        match dispatch {
            Ok(dispatch) => {
                self.jobs.spawn(attempt.key().identity, async move {
                    let requested = dispatch.await.unwrap_or(0);
                    jobs::Completion {
                        identity: attempt.key().identity,
                        lease: None,
                        announce: Some((attempt, requested)),
                    }
                });
            }
            Err(error) => {
                self.complete_announce(attempt, 0)?;
                return Err(error);
            }
        }
        Ok(Some(attempt))
    }

    pub(super) fn complete_announce(
        &mut self,
        attempt: Reservation,
        requested: u64,
    ) -> Result<(), String> {
        self.announces
            .owner
            .complete(attempt, requested, attempt.channels() & !requested)
            .map_err(detail)
    }

    #[cfg(test)]
    async fn dispatch_announce(&mut self, attempt: Reservation) -> Result<u64, String> {
        self.prepare_announce(attempt).await?.await
    }

    async fn prepare_announce(
        &mut self,
        attempt: Reservation,
    ) -> Result<Pin<Box<dyn Future<Output = Result<u64, String>> + Send + 'static>>, String> {
        self.cache_dependency_review(attempt.key().identity).await?;
        let record = *self
            .coordinator
            .identity(attempt.key().identity)
            .map_err(detail)?;
        let guard = self
            .coordinator
            .admit(record.id, record.public_hash, core::AdmissionClass::Network)
            .map_err(detail)?;
        self.controller
            .network_permit(
                &self.disk.runtime_authority(&self.coordinator),
                guard,
                MESSAGING_SERVICE,
            )
            .map_err(detail)?;
        let services = self
            .driver
            .entry(record.id)
            .map_err(detail)?
            .services
            .lock()
            .await;
        if attempt.key().service == DIRECT_SERVICE {
            let direct = services.direct().map_err(detail)?;
            return Ok(Box::pin(async move {
                Ok(if direct.announce().await.is_ok() {
                    1
                } else {
                    0
                })
            }));
        }
        if attempt.key().service != GROUP_SERVICE {
            return Err("Unknown announcement service.".into());
        }
        let network = services.network_scope().map_err(detail)?;
        let identity = prns_host::IdentityHash::new(record.public_hash.0);
        let destinations = [
            (
                SYNC,
                prns_groups_reticulum::destination_for(identity).map_err(detail)?,
            ),
            (
                ENROLLMENT,
                prns_groups_reticulum::enrollment::destination_for(identity).map_err(detail)?,
            ),
        ];
        Ok(Box::pin(async move {
            let mut requested = 0;
            for (channel, destination) in destinations {
                if attempt.channels() & channel == 0 {
                    continue;
                }
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    network.execute(prns_host::HostCommand::Announce {
                        destination,
                        interface: None,
                    }),
                )
                .await;
                if matches!(result, Ok(Ok(Ok(prns_host::CommandOutcome::Announced)))) {
                    requested |= channel;
                }
            }
            Ok(requested)
        }))
    }
}

#[cfg(test)]
#[path = "announces_tests.rs"]
mod tests;
