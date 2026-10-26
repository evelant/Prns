//! Local identity references are installation scoped, never routing addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct NamedIdentityReference {
    pub incarnation: [u8; 16],
    pub sequence: u64,
    pub public_hash: [u8; 16],
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum NamedIdentityState {
    Enabled,
    Paused,
    Retired,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct NamedIdentitySummary {
    pub reference: NamedIdentityReference,
    pub label: String,
    pub published_name: Option<String>,
    pub state: NamedIdentityState,
    /// Desired state is durable, but service activation or drainage remains.
    pub transitioning: bool,
    pub ready: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct NamedIdentityCatalog {
    pub identities: Vec<NamedIdentitySummary>,
    pub selected: Option<NamedIdentityReference>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum NamedIdentityCatalogOutcome {
    Ready { catalog: NamedIdentityCatalog },
    Busy,
    Unavailable { detail: String },
    Rejected { detail: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct CreateNamedIdentityInput {
    pub request_id: [u8; 32],
    pub label: String,
    pub published_name: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum NamedIdentityChange {
    Select,
    Rename { label: String },
    Pause,
    Resume,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ChangeNamedIdentityInput {
    pub identity: NamedIdentityReference,
    pub change: NamedIdentityChange,
}

/// Local conversation ownership; never a combined history or network receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum IdentityActivityScope {
    Group { group_id: [u8; 32] },
    Direct { destination: [u8; 16] },
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct IdentityConversationActivity {
    pub scope: IdentityActivityScope,
    pub unread_count: u32,
    /// Durable local record position. A screen acknowledges only positions it displayed.
    pub through_sequence: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct IdentityActivitySummary {
    pub actionable_invitations: u32,
    pub unread_groups: u32,
    pub unread_direct: u32,
    pub conversations: Vec<IdentityConversationActivity>,
    /// The bounded retained-record scan omitted older content or conversations.
    /// Counts are lower bounds when true, never evidence of an empty inbox.
    pub partial: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum IdentityActivityOutcome {
    Ready { summary: IdentityActivitySummary },
    Unavailable { detail: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct NamedIdentityActivity {
    pub identity: NamedIdentityReference,
    pub activity: IdentityActivityOutcome,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum NamedIdentityActivityListOutcome {
    Ready {
        activities: Vec<NamedIdentityActivity>,
    },
    Busy,
    Unavailable {
        detail: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MarkIdentityActivitySeenInput {
    pub scope: IdentityActivityScope,
    pub through_sequence: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum IdentityActivitySeenOutcome {
    Saved,
    Capacity,
    InvalidInput { detail: String },
    Unavailable { detail: String },
}
