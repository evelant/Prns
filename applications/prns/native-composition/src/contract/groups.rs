//! Application projections for durable group communication and enrollment.

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupConversation {
    pub group_id: [u8; 32],
    pub title: String,
    pub revision: u64,
    /// Current membership provenance, distinct from the record/change revision.
    pub control_head: [u8; 32],
    pub members: Vec<GroupMember>,
    pub member_count: u32,
    pub owner_identity: Option<[u8; 16]>,
    pub can_invite: bool,
    pub lifecycle: GroupLifecycle,
    pub can_send: bool,
    pub can_remove_members: bool,
    pub can_leave: bool,
    pub can_close: bool,
    pub can_delete_local: bool,
    pub synchronization: GroupSynchronizationState,
    /// Bounded, transient peer activity; durable delivery evidence remains on records.
    pub progress: GroupSynchronizationProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupLifecycle {
    Active,
    Leaving,
    Removed,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupMember {
    pub endpoint_id: [u8; 32],
    pub endpoint_incarnation: u64,
    pub identity: [u8; 16],
    pub is_local: bool,
    pub is_owner: bool,
}

/// Stable rejection categories; old saved actions have no category and decode
/// as Unknown. Names and UI advice are separate application projections.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupRejectionReason {
    #[default]
    Unknown,
    InvalidInput,
    NoRecipients,
    RequestConflict,
    InvitationExpired,
    NotFound,
    MembershipConflict,
    NotOwner,
    NotJoined,
    AuthenticationFailed,
    AlreadyJoined,
    GroupClosed,
    OwnerMustClose,
    MemberNotFound,
    StaleMembership,
    UnsupportedVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupSynchronizationProgress {
    pub running: bool,
    pub observed: bool,
    pub peers: Vec<GroupPeerSynchronizationProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupPeerSynchronizationProgress {
    pub endpoint_id: [u8; 32],
    pub endpoint_incarnation: u64,
    pub identity: [u8; 16],
    pub phase: GroupSynchronizationPhase,
    /// None means the outgoing queue has not been observed, not that it is empty.
    pub pending: Option<bool>,
    pub retry_after_millis: Option<u64>,
    pub failures: u8,
    pub updated_ago_millis: u64,
    pub last_issue: Option<GroupSynchronizationIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupSynchronizationPhase {
    Unknown,
    Waiting,
    Connecting,
    Synchronizing,
    RetryScheduled,
    StorageBlocked,
    MembershipCatchUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupSynchronizationIssue {
    Transport,
    StorageBusy,
    StorageFull,
    StorageUnavailable,
    MembershipChanged,
    TimeUncertain,
    Rejected,
    UnsupportedPeer,
    Configuration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupSynchronizationState {
    Stopped,
    /// The native synchronization worker is enabled; this is not delivery evidence.
    Running,
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupConversationListOutcome {
    Ready {
        conversations: Vec<GroupConversation>,
    },
    NotProvisioned,
    Busy,
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ListGroupMessagesInput {
    pub group_id: [u8; 32],
    pub before_sequence: Option<u64>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupTextMessage {
    pub message_id: [u8; 32],
    pub client_request_id: Option<[u8; 32]>,
    pub sequence: u64,
    pub author: [u8; 16],
    pub text: String,
    pub outgoing: bool,
    /// Authenticated device receipts, not human read receipts.
    pub received_count: u32,
    pub recipient_count: u32,
    pub membership_changed: bool,
    pub sent_at_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupMessageListOutcome {
    Ready {
        messages: Vec<GroupTextMessage>,
        revision: u64,
        next_before_sequence: Option<u64>,
    },
    NotProvisioned,
    InvalidInput {
        detail: String,
    },
    Busy,
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct SendGroupTextInput {
    pub group_id: [u8; 32],
    pub client_request_id: [u8; 32],
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupTextSendOutcome {
    /// Durable local acceptance; peer receipt counts are queried separately.
    Accepted {
        message_id: [u8; 32],
    },
    NotProvisioned,
    InvalidInput {
        reason: GroupRejectionReason,
        detail: String,
    },
    /// Recovery evidence has been retired. Do not automatically mint a new ID.
    RetryExpired,
    /// Bounded admission is full; existing work and this retry ID remain intact.
    StorageFull,
    Busy,
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupRequestIdOutcome {
    Ready { client_request_id: [u8; 32] },
    Unavailable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct CreateGroupInput {
    pub client_request_id: [u8; 32],
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum CreateGroupOutcome {
    Accepted {
        group_id: [u8; 32],
    },
    /// Recovery evidence has been retired. Do not automatically mint a new ID.
    RetryExpired,
    /// Bounded admission is full; existing work and this retry ID remain intact.
    StorageFull,
    Busy,
    InvalidInput {
        reason: GroupRejectionReason,
        detail: String,
    },
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct InviteToGroupInput {
    pub client_request_id: [u8; 32],
    pub group_id: [u8; 32],
    pub recipient_identity: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum InviteToGroupOutcome {
    Queued {
        invitation_id: [u8; 32],
    },
    /// Recovery evidence has been retired. Do not automatically mint a new ID.
    RetryExpired,
    /// Bounded admission is full; existing work and this retry ID remain intact.
    StorageFull,
    Busy,
    InvalidInput {
        reason: GroupRejectionReason,
        detail: String,
    },
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupInvitationState {
    Offered,
    Joining,
    /// Incoming: enrollment committed locally. Outgoing: authenticated peer
    /// acknowledgement after its durable enrollment.
    Joined,
    Declined,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupInvitation {
    pub invitation_id: [u8; 32],
    pub group_id: [u8; 32],
    pub title: String,
    pub inviter_identity: [u8; 16],
    pub recipient_identity: [u8; 16],
    pub expires_at_millis: u64,
    pub outgoing: bool,
    pub state: GroupInvitationState,
    /// The owner has deferred Add for its own unfinished current-era deliveries.
    /// This does not imply that every member's outgoing queue has been checked.
    pub waiting_for_delivery: bool,
    pub can_cancel: bool,
    pub can_dismiss: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupInvitationListOutcome {
    Ready { invitations: Vec<GroupInvitation> },
    Busy,
    Unavailable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct RespondGroupInvitationInput {
    pub client_request_id: [u8; 32],
    pub invitation_id: [u8; 32],
    pub accept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum RespondGroupInvitationOutcome {
    /// The decision is durable; joining can still require network exchanges.
    Saved,
    /// Recovery evidence has been retired. Do not automatically mint a new ID.
    RetryExpired,
    /// Bounded admission is full; existing work and this retry ID remain intact.
    StorageFull,
    Busy,
    InvalidInput {
        reason: GroupRejectionReason,
        detail: String,
    },
    Unavailable {
        detail: String,
    },
}
