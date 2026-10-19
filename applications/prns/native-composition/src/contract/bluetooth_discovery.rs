//! Local transport discovery settings. These names are not access credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct LocalBluetoothDiscoveryGroupsSettings {
    /// Opaque process-local storage owner fence; reread after reconnecting to native.
    pub owner_id: u64,
    pub generation_id: u64,
    /// Durably saved, canonical names used on subsequent starts.
    pub groups: Vec<String>,
    /// Last settled enabled supervisor configuration. None means unconfirmed or inactive.
    /// This does not establish a peer connection or prove a traffic route.
    pub applied_groups: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct SetBluetoothDiscoveryGroupsInput {
    pub owner_id: u64,
    pub generation_id: u64,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum LocalBluetoothDiscoveryGroupsOutcome {
    Ready {
        settings: LocalBluetoothDiscoveryGroupsSettings,
    },
    /// Stored names are malformed; these fresh fences permit an explicit repair.
    InvalidSaved {
        owner_id: u64,
        generation_id: u64,
        detail: String,
    },
    Busy,
    GenerationChanged,
    Invalid {
        detail: String,
    },
    /// A mutation may already be saved. Reread before proposing another change.
    Unavailable {
        detail: String,
    },
}
