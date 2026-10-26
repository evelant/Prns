use super::{RemoteControlControllerAuthority, RemoteControlRequestKind};

/// Public setup information for this installation, never a messaging identity or secret.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ControllerSetupCard {
    pub text: String,
    pub public_key_hex: String,
    pub controller_identity_fingerprint: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum ControllerSetupCardOutcome {
    Ready { card: ControllerSetupCard },
    Busy,
    LocalNodeStopped,
    Unavailable { detail: String },
}

/// An unverified claim from a local setup tool or administrator. Preview saves nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct TargetAccessCardPreview {
    pub target_identity_fingerprint: [u8; 16],
    pub recipient_controller_fingerprint: [u8; 16],
    pub authority: RemoteControlControllerAuthority,
    pub permissions: Vec<RemoteControlRequestKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum TargetAccessCardPreviewOutcome {
    Valid { preview: TargetAccessCardPreview },
    Invalid { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ImportTargetAccessCardInput {
    pub card: String,
    pub expected_controller_identity: [u8; 16],
    /// Explicit caller confirmation of the setup tool or administrator's identity.
    pub trusted_source_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum TargetAccessCardImportOutcome {
    /// Durable local access only. The target's grant and connectivity remain unchecked.
    Saved {
        target_identity_fingerprint: [u8; 16],
    },
    Existing {
        target_identity_fingerprint: [u8; 16],
    },
    TrustRequired,
    ControllerChanged,
    RecipientMismatch,
    Conflict,
    CapacityExhausted,
    Invalid {
        detail: String,
    },
    Busy,
    LocalNodeStopped,
    PersistenceUnavailable,
    PersistenceFailed,
    Unavailable {
        detail: String,
    },
    /// Admission may have committed. Inspect saved access before explicitly retrying.
    OutcomeUnknown,
}
