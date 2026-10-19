//! Bounded durable desired discovery names; the lifecycle owns live settlement separately.
use crate::development_store::{DevelopmentStoreFailure, APPLICATION_SETTINGS};
use prns_core::interfaces::{DiscoveryGroupId, DiscoveryGroupSet, MAX_DISCOVERY_GROUPS};
use redb::{Database, ReadableDatabase};

pub(crate) const KEY: &str = "bluetooth_discovery_groups_v1";
// Four 32-byte names, including worst-case JSON escaping, plus separators.
const MAX_ENCODED_LEN: usize = 800;

#[derive(Debug)]
pub(crate) enum SettingsError {
    InvalidSaved,
    Storage(DevelopmentStoreFailure),
}
impl SettingsError {
    pub(crate) fn into_store_failure(self) -> DevelopmentStoreFailure {
        match self {
            Self::InvalidSaved => DevelopmentStoreFailure::unavailable("Saved Bluetooth discovery groups are invalid. Restore them in Connections before starting the node."),
            Self::Storage(failure) => failure,
        }
    }
}

pub(crate) fn parse(names: &[String]) -> Result<DiscoveryGroupSet, String> {
    if names.is_empty() || names.len() > MAX_DISCOVERY_GROUPS {
        return Err("Choose between one and four discovery groups.".into());
    }
    let groups = names
        .iter()
        .map(|name| {
            DiscoveryGroupId::parse(name)
                .map_err(|_| "Each discovery group must contain 1–32 UTF-8 bytes.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    DiscoveryGroupSet::try_from_slice(&groups)
        .map_err(|_| "Discovery group names must be distinct.".into())
}

pub(crate) fn names(groups: DiscoveryGroupSet) -> Vec<String> {
    groups.iter().map(|name| name.as_str().to_owned()).collect()
}

pub(crate) fn settings(
    database: &Database,
    groups: Option<DiscoveryGroupSet>,
) -> Result<DiscoveryGroupSet, SettingsError> {
    let failure = |error: &dyn std::fmt::Display| {
        SettingsError::Storage(DevelopmentStoreFailure::unavailable(format!(
            "Could not access Bluetooth discovery settings: {error}"
        )))
    };
    if let Some(groups) = groups {
        let encoded = serde_json::to_string(&names(groups)).map_err(|e| failure(&e))?;
        let write = database.begin_write().map_err(|e| failure(&e))?;
        {
            let mut table = write
                .open_table(APPLICATION_SETTINGS)
                .map_err(|e| failure(&e))?;
            table
                .insert(KEY, encoded.as_str())
                .map_err(|e| failure(&e))?;
        }
        write.commit().map_err(|e| failure(&e))?;
        return Ok(groups);
    }
    let read = database.begin_read().map_err(|e| failure(&e))?;
    let table = match read.open_table(APPLICATION_SETTINGS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(DiscoveryGroupSet::reticulum()),
        Err(error) => return Err(failure(&error)),
    };
    let Some(value) = table.get(KEY).map_err(|e| failure(&e))? else {
        return Ok(DiscoveryGroupSet::reticulum());
    };
    let invalid = || SettingsError::InvalidSaved;
    if value.value().len() > MAX_ENCODED_LEN {
        return Err(invalid());
    }
    let saved: Vec<String> = serde_json::from_str(value.value()).map_err(|_| invalid())?;
    parse(&saved).map_err(|_| invalid())
}

#[cfg(test)]
mod tests;
