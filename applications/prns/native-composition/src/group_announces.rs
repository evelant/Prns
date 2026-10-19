//! App persistence and live control; no settings or extra executor in generic core.
use crate::contract::GroupAnnounceSettings;
use crate::development_store::DevelopmentStoreFailure;
use prns_groups_runner_native::{announce_control, AnnounceControl, AnnounceRequests};
use redb::{Database, ReadableDatabase, TableDefinition};
use std::sync::{Arc, Mutex};

const SETTINGS: TableDefinition<&str, u64> = TableDefinition::new("group_announce_settings");
const INTERVAL: &str = "interval_minutes";
pub(crate) const DEFAULT_MINUTES: u32 = 60;

pub(crate) fn validate(interval: Option<u32>) -> bool {
    interval.is_none_or(|minutes| matches!(minutes, 15 | 30 | 60 | 180 | 360))
}

pub(crate) fn settings(
    database: &Database,
    update: Option<GroupAnnounceSettings>,
) -> Result<GroupAnnounceSettings, DevelopmentStoreFailure> {
    let failure = |error: String| {
        DevelopmentStoreFailure::unavailable(format!(
            "Group announcement preferences could not be read or saved: {error}"
        ))
    };
    if let Some(settings) = update {
        if !validate(settings.interval_minutes) {
            return Err(failure("invalid interval".into()));
        }
        let write = database
            .begin_write()
            .map_err(|error| failure(error.to_string()))?;
        {
            let mut table = write
                .open_table(SETTINGS)
                .map_err(|error| failure(error.to_string()))?;
            table
                .insert(INTERVAL, u64::from(settings.interval_minutes.unwrap_or(0)))
                .map_err(|error| failure(error.to_string()))?;
        }
        write.commit().map_err(|error| failure(error.to_string()))?;
        return Ok(settings);
    }
    let read = database
        .begin_read()
        .map_err(|error| failure(error.to_string()))?;
    let table = match read.open_table(SETTINGS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => {
            return Ok(GroupAnnounceSettings {
                interval_minutes: Some(DEFAULT_MINUTES),
            })
        }
        Err(error) => return Err(failure(error.to_string())),
    };
    let value = table
        .get(INTERVAL)
        .map_err(|error| failure(error.to_string()))?
        .map(|value| value.value())
        .unwrap_or(u64::from(DEFAULT_MINUTES));
    let value = u32::try_from(value).map_err(|error| failure(error.to_string()))?;
    let interval_minutes = (value != 0).then_some(value);
    if !validate(interval_minutes) {
        return Err(failure("invalid stored interval".into()));
    }
    Ok(GroupAnnounceSettings { interval_minutes })
}

#[derive(Clone)]
pub(crate) struct LiveAnnounces(Arc<Mutex<State>>);
struct State {
    settings: GroupAnnounceSettings,
    active: Option<AnnounceControl>,
}
impl LiveAnnounces {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            settings: GroupAnnounceSettings {
                interval_minutes: Some(DEFAULT_MINUTES),
            },
            active: None,
        })))
    }
    pub(crate) fn update(&self, settings: GroupAnnounceSettings) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(control) = &state.active {
            let _ = control.set_interval(
                settings
                    .interval_minutes
                    .map(|minutes| u64::from(minutes) * 60_000),
            );
        }
        state.settings = settings;
    }
    pub(crate) fn start(&self) -> Result<AnnounceRequests, prns_groups_runner_native::Error> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (control, requests) = announce_control(
            state
                .settings
                .interval_minutes
                .map(|minutes| u64::from(minutes) * 60_000),
        )?;
        state.active = Some(control);
        Ok(requests)
    }
    pub(crate) fn stop(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active = None;
    }
    pub(crate) fn control(&self) -> Option<AnnounceControl> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
            .clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn preference_migrates_without_erasure_and_survives_reopen_including_off() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.redb");
        for choice in [Some(15), Some(30), Some(60), Some(180), Some(360), None] {
            let database = Database::create(&path).unwrap();
            if choice == Some(15) {
                assert_eq!(
                    settings(&database, None).unwrap().interval_minutes,
                    Some(60)
                );
            }
            settings(
                &database,
                Some(GroupAnnounceSettings {
                    interval_minutes: choice,
                }),
            )
            .unwrap();
            drop(database);
            let reopened = Database::open(&path).unwrap();
            assert_eq!(settings(&reopened, None).unwrap().interval_minutes, choice);
        }
        let database = Database::open(&path).unwrap();
        assert!(settings(
            &database,
            Some(GroupAnnounceSettings {
                interval_minutes: Some(1)
            })
        )
        .is_err());
        assert_eq!(settings(&database, None).unwrap().interval_minutes, None);
    }
    #[test]
    fn admitted_preference_is_owned_through_commit_and_applied_before_reply() {
        use crate::development_store::DevelopmentStoreOwner;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application.redb");
        let owner = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        let live = owner.group_health().announces();
        let requests = live.start().unwrap();
        let (reply, receive) = std::sync::mpsc::sync_channel(1);
        owner
            .admit_group_announces(
                Some(GroupAnnounceSettings {
                    interval_minutes: None,
                }),
                move |result| {
                    let _ = reply.send(result);
                },
            )
            .unwrap();
        drop(receive);
        owner.drain_blocking().unwrap();
        assert_eq!(live.0.lock().unwrap().settings.interval_minutes, None);
        drop(requests);
        owner.close().unwrap();
        let reopened = DevelopmentStoreOwner::open(root.path(), &path).unwrap();
        assert_eq!(
            reopened
                .group_health()
                .announces()
                .0
                .lock()
                .unwrap()
                .settings
                .interval_minutes,
            None
        );
        reopened.close().unwrap();
    }
}
