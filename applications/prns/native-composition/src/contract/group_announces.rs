//! Reachability preferences for the device's two shared group destinations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupAnnounceSettings {
    /// None disables automatic startup and periodic announces; manual still works.
    pub interval_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupAnnounceSettingsOutcome {
    Ready { settings: GroupAnnounceSettings },
    Busy,
    InvalidInput { detail: String },
    Unavailable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupAnnounceOutcome {
    /// Coalesced in this runtime; waiting for an interface or the shared budget.
    Queued,
    /// The local host accepted both commands. This is not peer receipt evidence.
    Requested,
    Throttled {
        retry_after_millis: u64,
    },
    Busy,
    Unavailable {
        detail: String,
    },
}
