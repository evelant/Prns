//! Native connection preferences. No network operation runs on the storage lane.
use crate::contract::{ConnectionSettings, ConnectionSettingsUpdateInput, TcpPeerProfile};
use crate::development_store::{DevelopmentStoreFailure, APPLICATION_SETTINGS};
use redb::{Database, ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::IpAddr;

pub(crate) mod runtime;
pub(crate) use runtime::{ConnectionBootstrap, ConnectionRuntime};

const KEY: &str = "connection_settings_v1";
const VERSION: u32 = 1;
const MAX_PEERS: usize = 16;
const MAX_LABEL_BYTES: usize = 64;
const MAX_HOST_BYTES: usize = 253;
const MAX_SETTINGS_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSettings {
    version: u32,
    pub(crate) revision: u64,
    pub(crate) settings: ConnectionSettings,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            version: VERSION,
            revision: 0,
            settings: ConnectionSettings::default(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum SettingsError {
    Invalid(String),
    InvalidSaved(String),
    Conflict(StoredSettings),
    Storage(DevelopmentStoreFailure),
}

fn storage(error: impl std::fmt::Display) -> SettingsError {
    SettingsError::Storage(DevelopmentStoreFailure::unavailable(error.to_string()))
}

pub(crate) fn endpoint(peer: &TcpPeerProfile) -> String {
    if peer.host.contains(':') {
        format!("[{}]:{}", peer.host, peer.port)
    } else {
        format!("{}:{}", peer.host, peer.port)
    }
}

fn normalize(
    mut settings: ConnectionSettings,
    create_ids: bool,
) -> Result<ConnectionSettings, String> {
    if settings.tcp_peers.len() > MAX_PEERS {
        return Err(format!("Save at most {MAX_PEERS} TCP peers."));
    }
    let mut ids = BTreeSet::new();
    let mut endpoints = BTreeSet::new();
    for peer in &mut settings.tcp_peers {
        if peer.id.is_empty() && create_ids {
            let mut bytes = [0_u8; 16];
            getrandom::getrandom(&mut bytes)
                .map_err(|_| "Could not create a connection identifier.")?;
            peer.id = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        }
        if peer.id.len() != 32
            || !peer
                .id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !ids.insert(peer.id.clone())
        {
            return Err("TCP peer identifiers must be unique native identifiers.".into());
        }
        peer.label = peer.label.trim().to_owned();
        if peer.label.is_empty()
            || peer.label.len() > MAX_LABEL_BYTES
            || peer.label.chars().any(char::is_control)
        {
            return Err(format!(
                "Give each peer a name of at most {MAX_LABEL_BYTES} UTF-8 bytes."
            ));
        }
        let host = peer.host.trim();
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        if host.is_empty() || host.len() > MAX_HOST_BYTES || !host.is_ascii() {
            return Err("Enter an IP address or DNS hostname without a scheme or port.".into());
        }
        if let Ok(address) = host.parse::<IpAddr>() {
            peer.host = address.to_string();
        } else {
            let dns = host.strip_suffix('.').unwrap_or(host);
            if dns.is_empty()
                || dns.split('.').any(|label| {
                    label.is_empty()
                        || label.len() > 63
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
            {
                return Err("Enter an IP address or DNS hostname without a scheme or port.".into());
            }
            peer.host = dns.to_ascii_lowercase();
        }
        if peer.port == 0 {
            return Err("TCP ports must be between 1 and 65535.".into());
        }
        prns_host::InterfaceConfig::TcpClient {
            target: endpoint(peer),
            bitrate: prns_host::Bitrate::Auto,
        }
        .validate()
        .map_err(|_| "The TCP peer address is invalid.")?;
        if !endpoints.insert(endpoint(peer)) {
            return Err("That TCP peer address is already saved.".into());
        }
    }
    Ok(settings)
}

fn decode(raw: Option<&str>) -> Result<StoredSettings, SettingsError> {
    let Some(raw) = raw else {
        return Ok(StoredSettings::default());
    };
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsError::InvalidSaved(
            "Saved connection settings exceed their size limit.".into(),
        ));
    }
    let saved: StoredSettings = serde_json::from_str(raw)
        .map_err(|_| SettingsError::InvalidSaved("Saved connection settings are malformed or use an unsupported format. They have been preserved.".into()))?;
    if saved.version != VERSION {
        return Err(SettingsError::InvalidSaved(
            "Saved connection settings use an unsupported version. They have been preserved."
                .into(),
        ));
    }
    let normalized =
        normalize(saved.settings.clone(), false).map_err(SettingsError::InvalidSaved)?;
    if normalized != saved.settings {
        return Err(SettingsError::InvalidSaved(
            "Saved connection settings are not canonical. They have been preserved.".into(),
        ));
    }
    Ok(saved)
}

pub(crate) fn settings(
    database: &Database,
    input: Option<ConnectionSettingsUpdateInput>,
) -> Result<StoredSettings, SettingsError> {
    let Some(input) = input else {
        let read = database.begin_read().map_err(storage)?;
        let table = match read.open_table(APPLICATION_SETTINGS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(StoredSettings::default()),
            Err(error) => return Err(storage(error)),
        };
        let value = table.get(KEY).map_err(storage)?;
        return decode(value.as_ref().map(|value| value.value()));
    };
    let normalized = normalize(input.settings, true).map_err(SettingsError::Invalid)?;
    let write = database.begin_write().map_err(storage)?;
    let updated = {
        let mut table = write.open_table(APPLICATION_SETTINGS).map_err(storage)?;
        let saved = {
            let raw = table.get(KEY).map_err(storage)?;
            decode(raw.as_ref().map(|value| value.value()))?
        };
        if saved.revision != input.expected_revision {
            return Err(SettingsError::Conflict(saved));
        }
        if saved.settings == normalized {
            return Ok(saved);
        }
        let next = StoredSettings {
            version: VERSION,
            revision: saved.revision.checked_add(1).ok_or_else(|| {
                SettingsError::InvalidSaved("Connection settings revision is exhausted.".into())
            })?,
            settings: normalized,
        };
        let encoded = serde_json::to_string(&next).map_err(storage)?;
        if encoded.len() > MAX_SETTINGS_BYTES {
            return Err(SettingsError::Invalid(
                "Connection settings exceed their size limit.".into(),
            ));
        }
        table.insert(KEY, encoded.as_str()).map_err(storage)?;
        next
    };
    write.commit().map_err(storage)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database(path: &std::path::Path) -> Database {
        let database = Database::create(path).unwrap();
        let write = database.begin_write().unwrap();
        write.open_table(APPLICATION_SETTINGS).unwrap();
        write.commit().unwrap();
        database
    }
    fn peer(host: &str) -> TcpPeerProfile {
        TcpPeerProfile {
            id: String::new(),
            label: " Test peer ".into(),
            host: host.into(),
            port: 4242,
            enabled: true,
        }
    }
    fn input(revision: u64, peers: Vec<TcpPeerProfile>) -> ConnectionSettingsUpdateInput {
        ConnectionSettingsUpdateInput {
            owner_id: 7,
            expected_revision: revision,
            settings: ConnectionSettings {
                lan_enabled: true,
                tcp_peers: peers,
            },
        }
    }
    #[test]
    fn optional_settings_commit_reopen_and_revision_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.redb");
        let db = database(&path);
        assert_eq!(settings(&db, None).unwrap(), StoredSettings::default());
        let saved = settings(&db, Some(input(0, vec![peer(" Example.COM. ")]))).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.settings.tcp_peers[0].host, "example.com");
        assert_eq!(saved.settings.tcp_peers[0].label, "Test peer");
        assert_eq!(saved.settings.tcp_peers[0].id.len(), 32);
        assert!(matches!(
            settings(&db, Some(input(0, vec![]))),
            Err(SettingsError::Conflict(_))
        ));
        drop(db);
        let db = Database::open(path).unwrap();
        assert_eq!(settings(&db, None).unwrap(), saved);
        assert_eq!(
            settings(&db, Some(input(1, saved.settings.tcp_peers.clone())))
                .unwrap()
                .revision,
            1
        );
    }
    #[test]
    fn invalid_and_future_saved_settings_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let db = database(&temp.path().join("settings.redb"));
        for raw in [
            "not json",
            r#"{"version":99,"revision":8,"settings":{"lan_enabled":true,"tcp_peers":[]}}"#,
        ] {
            let write = db.begin_write().unwrap();
            write
                .open_table(APPLICATION_SETTINGS)
                .unwrap()
                .insert(KEY, raw)
                .unwrap();
            write.commit().unwrap();
            assert!(matches!(
                settings(&db, None),
                Err(SettingsError::InvalidSaved(_))
            ));
            assert!(matches!(
                settings(&db, Some(input(0, vec![]))),
                Err(SettingsError::InvalidSaved(_))
            ));
            assert_eq!(
                db.begin_read()
                    .unwrap()
                    .open_table(APPLICATION_SETTINGS)
                    .unwrap()
                    .get(KEY)
                    .unwrap()
                    .unwrap()
                    .value(),
                raw
            );
        }
    }
    #[test]
    fn validation_has_no_network_dependency_and_rejects_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let db = database(&temp.path().join("settings.redb"));
        let saved = settings(
            &db,
            Some(input(
                0,
                vec![peer("[2001:db8::1]"), peer("unreachable.invalid")],
            )),
        )
        .unwrap();
        assert_eq!(endpoint(&saved.settings.tcp_peers[0]), "[2001:db8::1]:4242");
        assert!(matches!(
            settings(
                &db,
                Some(input(1, vec![peer("EXAMPLE.COM"), peer("example.com.")]))
            ),
            Err(SettingsError::Invalid(_))
        ));
        for host in ["https://example.com", "host:4242", "", "a b", "-bad.local"] {
            assert!(matches!(
                settings(&db, Some(input(1, vec![peer(host)]))),
                Err(SettingsError::Invalid(_))
            ));
        }
        assert_eq!(settings(&db, None).unwrap(), saved);
    }
}
