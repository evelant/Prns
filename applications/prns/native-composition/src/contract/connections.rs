//! Installation-wide saved transport intent and separately observed live state.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ConnectionSettings {
    pub lan_enabled: bool,
    pub tcp_peers: Vec<TcpPeerProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct TcpPeerProfile {
    /// Empty on creation; native assigns a stable opaque identifier before commit.
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ConnectionSettingsSnapshot {
    pub owner_id: u64,
    pub revision: u64,
    pub settings: ConnectionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ConnectionSettingsUpdateInput {
    pub owner_id: u64,
    pub expected_revision: u64,
    pub settings: ConnectionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct RetryConnectionInput {
    pub owner_id: u64,
    pub generation_id: u64,
    pub expected_revision: u64,
    /// "lan" or the saved TCP profile ID.
    pub profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum ConnectionSettingsOutcome {
    Ready {
        snapshot: ConnectionSettingsSnapshot,
    },
    Conflict {
        snapshot: ConnectionSettingsSnapshot,
    },
    Invalid {
        detail: String,
    },
    InvalidSaved {
        detail: String,
    },
    Busy,
    OwnerChanged,
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum LocalConnectionState {
    Stopped,
    Disabled,
    Applying,
    Discovering,
    Connecting,
    Connected,
    Reconnecting,
    Unavailable,
    Failed,
    Retiring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum LanDiscoveryMethod {
    Unavailable,
    Bonjour,
    StockMulticast,
    Combined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct LanConnectionSnapshot {
    pub desired_enabled: bool,
    pub state: LocalConnectionState,
    pub discovery_method: LanDiscoveryMethod,
    pub detail: Option<String>,
    pub stock_multicast_available: bool,
    pub connected_peer_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct TcpPeerConnectionSnapshot {
    pub profile: TcpPeerProfile,
    pub interface_id: Option<Vec<u8>>,
    pub state: LocalConnectionState,
    pub detail: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct PhysicalConnectionSnapshot {
    pub interface_id: Vec<u8>,
    pub logical_interface_id: Vec<u8>,
    pub parent_interface_id: Option<Vec<u8>>,
    pub profile_id: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub state: LocalConnectionState,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub endpoint: Option<String>,
    pub local_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ConnectionSnapshot {
    pub generation_id: u64,
    pub settings: ConnectionSettingsSnapshot,
    pub applied_revision: Option<u64>,
    pub lan: LanConnectionSnapshot,
    pub tcp_peers: Vec<TcpPeerConnectionSnapshot>,
    pub physical_connections: Vec<PhysicalConnectionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum ConnectionSnapshotOutcome {
    Ready { snapshot: ConnectionSnapshot },
    Busy,
    Unavailable { detail: String },
}
