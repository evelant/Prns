//! Durable application intents. These are UI recovery records, not delivery receipts.

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct PrepareGroupActionInput {
    /// Caller-visible identity; mismatches fail before any durable mutation.
    pub owner_identity: [u8; 16],
    pub input: GroupActionInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupActionReference {
    pub owner_identity: [u8; 16],
    pub client_request_id: [u8; 32],
}

use super::{
    CreateGroupInput, InviteToGroupInput, RespondGroupInvitationInput, SendGroupTextInput,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupActionInput {
    Create { input: CreateGroupInput },
    Invite { input: InviteToGroupInput },
    Respond { input: RespondGroupInvitationInput },
    Send { input: SendGroupTextInput },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupActionResult {
    Created {
        group_id: [u8; 32],
    },
    Invited {
        invitation_id: [u8; 32],
    },
    Responded,
    Sent {
        message_id: [u8; 32],
    },
    Rejected {
        #[serde(default)]
        reason: super::GroupRejectionReason,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupActionState {
    /// Input is committed; execution may not have started or its reply was lost.
    Pending,
    /// Confirmed local result. This does not indicate remote delivery.
    Completed { result: GroupActionResult },
    /// Execution evidence expired. Requires explicit review before discarding.
    RetryExpired,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct GroupAction {
    pub owner_identity: [u8; 16],
    pub input: GroupActionInput,
    pub state: GroupActionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupActionOutcome {
    Ready { action: GroupAction },
    InvalidInput { detail: String },
    StorageFull,
    Busy,
    Unavailable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupActionListOutcome {
    Ready { actions: Vec<GroupAction> },
    Busy,
    Unavailable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum GroupActionAcknowledgeOutcome {
    Removed,
    /// Pending work cannot be discarded; expired work requires reviewed discard.
    NeedsReview,
    Busy,
    Unavailable {
        detail: String,
    },
}
