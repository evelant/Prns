use super::Contact;

/// A signature-checked service claim, not a verified person or observed network route.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ContactCard {
    pub uri: String,
    pub display_name: String,
    pub identity: [u8; 16],
    pub destination: [u8; 16],
    pub service_name_hash: [u8; 10],
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ExportContactCardInput {
    pub expected_local_identity: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct ImportContactCardInput {
    pub uri: String,
    pub expected_local_identity: [u8; 16],
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum ContactCardPreviewOutcome {
    Valid { card: ContactCard },
    Invalid { detail: String },
    UnsupportedService,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum ContactCardExportOutcome {
    Ready { card: ContactCard },
    IdentityChanged,
    MissingIdentity,
    Busy,
    Unavailable { detail: String },
    DevelopmentResetRequired { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum ContactCardImportOutcome {
    Saved {
        contact: Contact,
    },
    Updated {
        contact: Contact,
    },
    Existing {
        contact: Contact,
    },
    IdentityConflict {
        existing: [u8; 16],
        attempted: [u8; 16],
    },
    IdentityChanged,
    MissingIdentity,
    Invalid {
        detail: String,
    },
    UnsupportedService,
    Busy,
    Unavailable {
        detail: String,
    },
    DevelopmentResetRequired {
        reason: String,
    },
}
