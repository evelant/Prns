use super::{admission, supervisor, Supervisor};
use crate::contract::{GroupAnnounceOutcome, GroupAnnounceSettings, GroupAnnounceSettingsOutcome};
use crate::development_store::DevelopmentStoreFailure;
use tokio::sync::oneshot;

pub(crate) async fn settings(
    update: Option<GroupAnnounceSettings>,
) -> GroupAnnounceSettingsOutcome {
    settings_with_supervisor(supervisor(), update).await
}

async fn settings_with_supervisor(
    supervisor: &Supervisor,
    update: Option<GroupAnnounceSettings>,
) -> GroupAnnounceSettingsOutcome {
    if update
        .as_ref()
        .is_some_and(|settings| !crate::group_announces::validate(settings.interval_minutes))
    {
        return GroupAnnounceSettingsOutcome::InvalidInput {
            detail: "Choose off, 15 or 30 minutes, or 1, 3 or 6 hours.".into(),
        };
    }
    let receive = {
        let state = match admission::try_state(supervisor) {
            Ok(state) => state,
            Err(_) => return GroupAnnounceSettingsOutcome::Busy,
        };
        let Some(owner) = state.application_owner.as_ref() else {
            return GroupAnnounceSettingsOutcome::Unavailable {
                detail: "Prepare native storage before reading group announcement settings.".into(),
            };
        };
        let (reply, receive) = oneshot::channel();
        if let Err(error) = owner.admit_group_announces(update, move |result| {
            let _ = reply.send(result);
        }) {
            return failure(error);
        }
        receive
    };
    match receive.await {
        Ok(Ok(settings)) => GroupAnnounceSettingsOutcome::Ready { settings },
        Ok(Err(error)) => failure(error),
        Err(_) => GroupAnnounceSettingsOutcome::Unavailable {
            detail: "The database owner stopped before confirming group announcement settings."
                .into(),
        },
    }
}

fn failure(error: DevelopmentStoreFailure) -> GroupAnnounceSettingsOutcome {
    let detail = match error {
        DevelopmentStoreFailure::Unavailable(detail)
        | DevelopmentStoreFailure::ResetRequired(detail) => detail,
    };
    GroupAnnounceSettingsOutcome::Unavailable { detail }
}

pub(crate) async fn announce() -> GroupAnnounceOutcome {
    let control = {
        let state = match admission::try_state(supervisor()) {
            Ok(state) => state,
            Err(_) => return GroupAnnounceOutcome::Busy,
        };
        state
            .application_owner
            .as_ref()
            .and_then(|owner| owner.group_health().announces().control())
    };
    let Some(control) = control else {
        return GroupAnnounceOutcome::Unavailable {
            detail: "Start the node to announce group services.".into(),
        };
    };
    use prns_groups_runner_native::AnnounceOutcome;
    match control.announce().await {
        AnnounceOutcome::Requested => GroupAnnounceOutcome::Requested,
        AnnounceOutcome::Throttled { retry_after_millis } => GroupAnnounceOutcome::Throttled { retry_after_millis },
        AnnounceOutcome::Busy => GroupAnnounceOutcome::Busy,
        AnnounceOutcome::Unavailable => GroupAnnounceOutcome::Unavailable { detail: "The node could not confirm both group announcement requests. One may already have been sent.".into() },
    }
}
