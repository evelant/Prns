//! Persisted transport configuration with owner/generation fencing and cancellation-safe settlement.
use super::*;
use crate::bluetooth_discovery::{names, parse};
use crate::contract::{
    LocalBluetoothDiscoveryGroupsOutcome as Outcome,
    LocalBluetoothDiscoveryGroupsSettings as Settings,
};

pub(crate) async fn settings(input: Option<SetBluetoothDiscoveryGroupsInput>) -> Outcome {
    settings_with_supervisor(supervisor(), input).await
}

async fn settings_with_supervisor(
    supervisor: &Supervisor,
    input: Option<SetBluetoothDiscoveryGroupsInput>,
) -> Outcome {
    let groups = match input.as_ref().map(|input| parse(&input.groups)).transpose() {
        Ok(groups) => groups,
        Err(detail) => return Outcome::Invalid { detail },
    };
    let (receive, owner_id, generation) = {
        let state = match admission::try_state(supervisor) {
            Ok(state) => state,
            Err(_) => return Outcome::Busy,
        };
        let snapshot = supervisor.snapshots.read();
        if matches!(
            snapshot.runtime,
            DevelopmentNodeRuntime::Starting | DevelopmentNodeRuntime::Stopping
        ) || supervisor.snapshots.is_explicit_stop_in_progress()
        {
            return Outcome::Busy;
        }
        let Some(owner) = state.application_owner.as_ref() else {
            return unavailable(
                "Prepare native storage before editing Bluetooth discovery groups.",
            );
        };
        let owner_id = owner.owner_id();
        let generation = snapshot.generation_id;
        if input
            .as_ref()
            .is_some_and(|input| input.owner_id != owner_id || input.generation_id != generation)
        {
            return Outcome::GenerationChanged;
        }
        if input.is_some()
            && snapshot.runtime == DevelopmentNodeRuntime::Running
            && state
                .worker
                .as_ref()
                .and_then(|worker| worker.bluetooth.get())
                .is_some_and(|control| !control.can_replace_groups())
        {
            return Outcome::Busy;
        }
        let change = if input.is_some() {
            let Some(change) = owner.start_bluetooth_change() else {
                return Outcome::Busy;
            };
            Some(change)
        } else {
            None
        };
        let running = snapshot.runtime == DevelopmentNodeRuntime::Running;
        let worker = state.worker.as_ref().filter(|_| running).map(|worker| {
            (
                Arc::clone(&worker.bluetooth),
                Arc::clone(&worker.runtime),
                worker.shutdown.clone(),
            )
        });
        let (reply, receive) = oneshot::channel();
        if let Err(failure) = owner.admit_bluetooth_discovery(groups, move |result| {
            let saved = match result {
                Ok(saved) => saved,
                Err(crate::bluetooth_discovery::SettingsError::InvalidSaved) => {
                    let _ = reply.send(Outcome::InvalidSaved { owner_id, generation_id: generation,
                        detail: "Saved discovery groups are invalid. Explicitly restore the default to reconnect normally.".into() });
                    return;
                }
                Err(crate::bluetooth_discovery::SettingsError::Storage(failure)) => {
                    let _ = reply.send(failed(failure)); return;
                }
            };
            if let Some((control, runtime, shutdown)) = worker.as_ref() {
                if let (Some(control), Some(runtime)) = (control.get(), runtime.get()) {
                    if groups.is_some() && !shutdown.requested.load(Ordering::Acquire) {
                        // The database owner hands the admitted work to this exact runtime.
                        // A dropped foreign waiter cannot cancel peer teardown or release admission.
                        let replacement = control.replace_groups(saved);
                        let worker = worker.clone();
                        runtime.spawn(async move {
                            let _change = change;
                            let result = if replacement.await {
                                ready(owner_id, generation, saved, worker.as_ref())
                            } else { unavailable("Bluetooth discovery groups were saved but live replacement failed. Reread settings.") };
                            let _ = reply.send(result);
                        });
                        return;
                    }
                }
            }
            let _change = change;
            let _ = reply.send(ready(owner_id, generation, saved, worker.as_ref()));
        }) { return failed(failure); }
        (receive, owner_id, generation)
    };
    let result = admission::bounded_reply(receive, Duration::from_secs(10)).await
        .unwrap_or_else(|_| unavailable("Bluetooth discovery confirmation is still pending or stopped. The setting may be saved; reread it."));
    let state = match admission::try_state(supervisor) {
        Ok(state) => state,
        Err(_) => return Outcome::Busy,
    };
    if state
        .application_owner
        .as_ref()
        .map(DevelopmentStoreOwner::owner_id)
        != Some(owner_id)
        || supervisor.snapshots.read().generation_id != generation
    {
        return Outcome::GenerationChanged;
    }
    // Stop does not necessarily increment the generation, so never return an old live claim.
    match result {
        Outcome::Ready { mut settings } => {
            if supervisor.snapshots.read().runtime != DevelopmentNodeRuntime::Running {
                settings.applied_groups = None;
            }
            Outcome::Ready { settings }
        }
        other => other,
    }
}

type LiveWorker = (
    Arc<OnceLock<BluetoothControl>>,
    Arc<OnceLock<tokio::runtime::Handle>>,
    ShutdownSignal,
);

fn ready(
    owner_id: u64,
    generation_id: u64,
    saved: prns_core::interfaces::DiscoveryGroupSet,
    worker: Option<&LiveWorker>,
) -> Outcome {
    let applied_groups = worker
        .filter(|(_, _, shutdown)| !shutdown.requested.load(Ordering::Acquire))
        .and_then(|(control, _, _)| control.get())
        .and_then(BluetoothControl::applied_groups)
        .map(names);
    Outcome::Ready {
        settings: Settings {
            owner_id,
            generation_id,
            groups: names(saved),
            applied_groups,
        },
    }
}
fn unavailable(detail: &str) -> Outcome {
    Outcome::Unavailable {
        detail: detail.into(),
    }
}
fn failed(failure: DevelopmentStoreFailure) -> Outcome {
    match failure {
        DevelopmentStoreFailure::Unavailable(detail)
        | DevelopmentStoreFailure::ResetRequired(detail) => Outcome::Unavailable { detail },
    }
}

#[cfg(test)]
mod tests;
