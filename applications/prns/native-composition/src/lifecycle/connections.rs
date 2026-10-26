//! Installation settings settle on the storage lane; networking stays generation-owned.
use super::*;
use crate::connections::{SettingsError, StoredSettings};

pub(crate) async fn settings(
    input: Option<ConnectionSettingsUpdateInput>,
) -> ConnectionSettingsOutcome {
    settings_with_supervisor(supervisor(), input).await
}

pub(super) async fn settings_with_supervisor(
    supervisor: &Supervisor,
    input: Option<ConnectionSettingsUpdateInput>,
) -> ConnectionSettingsOutcome {
    let (receive, owner_id) = {
        let state = match admission::try_state(supervisor) {
            Ok(state) => state,
            Err(_) => return ConnectionSettingsOutcome::Busy,
        };
        if supervisor.snapshots.is_explicit_stop_in_progress()
            || matches!(
                supervisor.snapshots.read().runtime,
                DevelopmentNodeRuntime::Starting | DevelopmentNodeRuntime::Stopping
            )
        {
            return ConnectionSettingsOutcome::Busy;
        }
        let Some(owner) = state.application_owner.as_ref() else {
            return unavailable("Prepare native storage before editing connections.");
        };
        let owner_id = owner.owner_id();
        if input
            .as_ref()
            .is_some_and(|input| input.owner_id != owner_id)
        {
            return ConnectionSettingsOutcome::OwnerChanged;
        }
        let mutating = input.is_some();
        let worker = state.worker.as_ref().map(|worker| {
            (
                Arc::clone(&worker.connections),
                Arc::clone(&worker.runtime),
                worker.shutdown.clone(),
            )
        });
        let (reply, receive) = oneshot::channel();
        if let Err(error) = owner.admit_connections(input, move |result| {
            let saved = match result {
                Ok(saved) => saved,
                Err(error) => {
                    let _ = reply.send(map_error(owner_id, error));
                    return;
                }
            };
            let snapshot = saved_snapshot(owner_id, &saved);
            if mutating {
                if let Some((connections, runtime, shutdown)) = worker {
                    if !shutdown.requested.load(Ordering::Acquire) {
                        if let (Some(connections), Some(runtime)) =
                            (connections.get(), runtime.get())
                        {
                            let connections = Arc::clone(connections);
                            // Fence retries as soon as the desired revision commits, even
                            // while an earlier generation-owned reconciliation is waiting.
                            connections.publish_desired(&saved);
                            // This task owns the admitted apply even if the foreign waiter is dropped.
                            runtime.spawn(async move {
                                connections.apply(saved).await;
                                let _ = reply.send(ConnectionSettingsOutcome::Ready { snapshot });
                            });
                            return;
                        }
                    }
                }
            }
            let _ = reply.send(ConnectionSettingsOutcome::Ready { snapshot });
        }) {
            return store_unavailable(error);
        }
        (receive, owner_id)
    };
    let outcome = admission::bounded_reply(receive, Duration::from_secs(25))
        .await
        .unwrap_or_else(|_| {
            unavailable(
                "Connection settings confirmation is pending. Refresh before changing them again.",
            )
        });
    let state = match admission::try_state(supervisor) {
        Ok(state) => state,
        Err(_) => return ConnectionSettingsOutcome::Busy,
    };
    if state
        .application_owner
        .as_ref()
        .map(DevelopmentStoreOwner::owner_id)
        != Some(owner_id)
    {
        return ConnectionSettingsOutcome::OwnerChanged;
    }
    outcome
}

pub(crate) async fn snapshot() -> ConnectionSnapshotOutcome {
    snapshot_with_supervisor(supervisor()).await
}

pub(super) async fn snapshot_with_supervisor(supervisor: &Supervisor) -> ConnectionSnapshotOutcome {
    let settings = match settings_with_supervisor(supervisor, None).await {
        ConnectionSettingsOutcome::Ready { snapshot } => snapshot,
        ConnectionSettingsOutcome::Busy => return ConnectionSnapshotOutcome::Busy,
        outcome => {
            return ConnectionSnapshotOutcome::Unavailable {
                detail: format!("Connection settings unavailable: {outcome:?}"),
            }
        }
    };
    let (manager, generation, owner_id, runtime) = {
        let state = match admission::try_state(supervisor) {
            Ok(state) => state,
            Err(_) => return ConnectionSnapshotOutcome::Busy,
        };
        let generation = supervisor.snapshots.read().generation_id;
        if state
            .application_owner
            .as_ref()
            .map(DevelopmentStoreOwner::owner_id)
            != Some(settings.owner_id)
        {
            return failed_snapshot("The native storage owner changed. Refresh connections.");
        }
        if supervisor.snapshots.read().runtime != DevelopmentNodeRuntime::Running {
            return ConnectionSnapshotOutcome::Ready {
                snapshot: crate::connections::runtime::stopped_snapshot(generation, settings),
            };
        }
        let Some(worker) = state.worker.as_ref() else {
            return failed_snapshot("The connection owner is unavailable.");
        };
        let (Some(manager), Some(runtime)) = (worker.connections.get(), worker.runtime.get())
        else {
            return failed_snapshot("Connections are starting.");
        };
        (
            Arc::clone(manager),
            generation,
            settings.owner_id,
            runtime.clone(),
        )
    };
    let (reply, receive) = oneshot::channel();
    runtime.spawn(async move {
        let _ = reply.send(manager.snapshot(settings).await);
    });
    let outcome = match admission::bounded_reply(receive, Duration::from_secs(3)).await {
        Ok(Ok(snapshot)) => ConnectionSnapshotOutcome::Ready { snapshot },
        Ok(Err(detail)) => ConnectionSnapshotOutcome::Unavailable { detail },
        Err(_) => failed_snapshot("Connection inspection is still pending. Refresh to check it."),
    };
    let state = match admission::try_state(supervisor) {
        Ok(state) => state,
        Err(_) => return ConnectionSnapshotOutcome::Busy,
    };
    if supervisor.snapshots.read().generation_id != generation
        || state
            .application_owner
            .as_ref()
            .map(DevelopmentStoreOwner::owner_id)
            != Some(owner_id)
        || supervisor.snapshots.read().runtime != DevelopmentNodeRuntime::Running
    {
        return failed_snapshot("The native connection generation changed. Refresh connections.");
    }
    outcome
}

pub(crate) async fn retry(input: RetryConnectionInput) -> ConnectionSnapshotOutcome {
    let receive = {
        let state = match admission::try_state(supervisor()) {
            Ok(state) => state,
            Err(_) => return ConnectionSnapshotOutcome::Busy,
        };
        if state
            .application_owner
            .as_ref()
            .map(DevelopmentStoreOwner::owner_id)
            != Some(input.owner_id)
            || supervisor().snapshots.read().generation_id != input.generation_id
            || supervisor().snapshots.read().runtime != DevelopmentNodeRuntime::Running
        {
            return failed_snapshot(
                "The native connection owner changed. Refresh before retrying.",
            );
        }
        let Some(worker) = state.worker.as_ref() else {
            return failed_snapshot("Start this phone before retrying.");
        };
        if worker.shutdown.requested.load(Ordering::Acquire) {
            return ConnectionSnapshotOutcome::Busy;
        }
        let (Some(manager), Some(runtime)) = (worker.connections.get(), worker.runtime.get())
        else {
            return failed_snapshot("Connections are starting.");
        };
        let manager = Arc::clone(manager);
        let (reply, receive) = oneshot::channel();
        runtime.spawn(async move {
            let _ = reply.send(manager.retry(&input).await);
        });
        receive
    };
    match admission::bounded_reply(receive, Duration::from_secs(25)).await {
        Ok(Ok(())) => snapshot().await,
        Ok(Err(detail)) => ConnectionSnapshotOutcome::Unavailable { detail },
        Err(_) => failed_snapshot("Connection retry is still settling. Refresh its status."),
    }
}

fn saved_snapshot(owner_id: u64, saved: &StoredSettings) -> ConnectionSettingsSnapshot {
    ConnectionSettingsSnapshot {
        owner_id,
        revision: saved.revision,
        settings: saved.settings.clone(),
    }
}
fn map_error(owner_id: u64, error: SettingsError) -> ConnectionSettingsOutcome {
    match error {
        SettingsError::Invalid(detail) => ConnectionSettingsOutcome::Invalid { detail },
        SettingsError::InvalidSaved(detail) => ConnectionSettingsOutcome::InvalidSaved { detail },
        SettingsError::Conflict(saved) => ConnectionSettingsOutcome::Conflict {
            snapshot: saved_snapshot(owner_id, &saved),
        },
        SettingsError::Storage(error) => store_unavailable(error),
    }
}
fn store_unavailable(error: DevelopmentStoreFailure) -> ConnectionSettingsOutcome {
    match error {
        DevelopmentStoreFailure::Unavailable(detail)
        | DevelopmentStoreFailure::ResetRequired(detail) => unavailable(&detail),
    }
}
fn unavailable(detail: &str) -> ConnectionSettingsOutcome {
    ConnectionSettingsOutcome::Unavailable {
        detail: detail.into(),
    }
}
fn failed_snapshot(detail: &str) -> ConnectionSnapshotOutcome {
    ConnectionSnapshotOutcome::Unavailable {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
