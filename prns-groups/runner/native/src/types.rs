use prns_groups_mls::{LocalIdentity, Policy};
use prns_groups_reticulum::Peer;
use prns_groups_service::{ClockReading, ControlReference, Endpoint, GroupId, RecordId};
use prns_groups_store::StoreIncarnation;
use prns_groups_store_native::{NativeError, NativeLimits, StoreNamespace};
use prns_host::IdentityHash;

/// Trusted development enrollment. Private key material stays on the native
/// storage lane and never crosses the application bindings.
#[derive(Clone)]
pub struct Provisioned {
    /// None only for the legacy, fixed development enrollment.
    pub owner_identity: Option<IdentityHash>,
    pub lifecycle: GroupLifecycle,
    pub history_generation: u32,
    /// Retained mappings for authors removed from the current roster.
    pub history_peers: Vec<Peer>,
    pub group: GroupId,
    pub control: ControlReference,
    pub readers: Vec<Endpoint>,
    pub identity: LocalIdentity,
    pub policy: Policy,
    pub namespace: StoreNamespace,
    pub incarnation: StoreIncarnation,
    pub native_limits: NativeLimits,
    pub mls_limits: prns_groups_mls::Limits,
    pub service_limits: prns_groups_service::Limits,
    pub title: String,
    pub peers: Vec<Peer>,
    pub local_reticulum_identity: IdentityHash,
}

#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub control: ControlReference,
    pub group: GroupId,
    pub local: Endpoint,
    pub readers: Vec<Endpoint>,
    pub peers: Vec<Peer>,
    pub incarnation: StoreIncarnation,
    pub local_reticulum_identity: IdentityHash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationMember {
    pub endpoint: Endpoint,
    pub identity: IdentityHash,
    pub is_local: bool,
    pub is_owner: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum GroupLifecycle {
    #[default]
    Active,
    Leaving,
    Removed,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub owner_identity: Option<IdentityHash>,
    pub lifecycle: GroupLifecycle,
    pub can_send: bool,
    pub can_invite: bool,
    pub can_remove_members: bool,
    pub can_leave: bool,
    pub can_close: bool,
    pub can_delete_local: bool,
    pub group: GroupId,
    pub title: String,
    pub revision: u64,
    /// Membership provenance, separate from the record/change revision.
    pub control_head: prns_groups::ControlId,
    /// Complete current readers, including this device. Not connectivity evidence.
    pub members: Vec<ConversationMember>,
    pub member_count: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Delivery evidence stays truthful after an authorization barrier.
    pub membership_changed: bool,
    pub id: RecordId,
    pub client_request_id: Option<[u8; 32]>,
    /// Local durable insertion order. Receipt changes do not move a message.
    pub sequence: u64,
    pub author: IdentityHash,
    pub text: String,
    pub outgoing: bool,
    pub received_count: usize,
    pub recipient_count: usize,
    pub sent_at_millis: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub revision: u64,
    pub next_before_sequence: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct SendText {
    pub group: GroupId,
    pub client_request_id: [u8; 32],
    pub text: String,
    pub clock: ClockReading,
}

/// A definite rejected action. This never classifies an ambiguous storage or
/// transport failure, which must retain its original request for recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectionReason {
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

#[derive(Debug)]
pub enum Error {
    NotProvisioned,
    NoRecipients,
    Enrollment(crate::enrollment::EnrollmentError),
    InvalidInput,
    IdempotencyConflict,
    Retry(prns_groups::retry::RetryError),
    StorageFull,
    Busy,
    Unavailable,
    Configuration,
    Corrupt,
    Clock,
    Storage(NativeError),
    Service(prns_groups_service::Error<NativeError>),
    Transport(prns_groups_reticulum::Error<NativeError>),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native group runner: {self:?}")
    }
}
impl std::error::Error for Error {}
impl Error {
    pub fn rejection_reason(&self) -> Option<RejectionReason> {
        use crate::enrollment::EnrollmentError;
        Some(match self {
            Self::InvalidInput => RejectionReason::InvalidInput,
            Self::NoRecipients => RejectionReason::NoRecipients,
            Self::IdempotencyConflict => RejectionReason::RequestConflict,
            Self::Enrollment(error) => match error {
                EnrollmentError::Invalid => RejectionReason::InvalidInput,
                EnrollmentError::Authentication => RejectionReason::AuthenticationFailed,
                EnrollmentError::Expired => RejectionReason::InvitationExpired,
                EnrollmentError::Missing => RejectionReason::NotFound,
                EnrollmentError::Conflict => RejectionReason::MembershipConflict,
                EnrollmentError::RequestConflict => RejectionReason::RequestConflict,
                EnrollmentError::NotOwner => RejectionReason::NotOwner,
                EnrollmentError::NotJoined => RejectionReason::NotJoined,
                EnrollmentError::AlreadyJoined => RejectionReason::AlreadyJoined,
                EnrollmentError::GroupClosed => RejectionReason::GroupClosed,
                EnrollmentError::OwnerMustClose => RejectionReason::OwnerMustClose,
                EnrollmentError::MemberNotFound => RejectionReason::MemberNotFound,
                EnrollmentError::StaleMembership => RejectionReason::StaleMembership,
                EnrollmentError::UnsupportedVersion => RejectionReason::UnsupportedVersion,
                _ => return None,
            },
            _ => return None,
        })
    }

    pub fn is_retry_expired(&self) -> bool {
        use prns_groups::retry::RetryError;
        matches!(
            self,
            Self::Retry(RetryError::Expired | RetryError::InvalidToken)
                | Self::Enrollment(crate::enrollment::EnrollmentError::Retry(
                    RetryError::Expired | RetryError::InvalidToken
                ))
        )
    }
    /// Bounded admission pressure, not permission to discard accepted work or
    /// change its request ID. Ambiguous database errors remain Unavailable.
    pub fn is_storage_full(&self) -> bool {
        use prns_groups_mls::Error as MlsError;
        use prns_groups_service::{Error as ServiceError, Fault};
        use prns_groups_store::{OperationRejection, StoreError};
        matches!(
            self,
            Self::StorageFull
                | Self::Storage(NativeError::Capacity)
                | Self::Enrollment(crate::enrollment::EnrollmentError::Capacity)
                | Self::Service(ServiceError::Fault(Fault::Capacity))
                | Self::Service(ServiceError::Store(
                    StoreError::Capacity(_)
                        | StoreError::OperationRejected(
                            OperationRejection::ReservedCapacity
                                | OperationRejection::RecoveryWindowFull
                        )
                ))
                | Self::Service(ServiceError::Security(MlsError::Capacity(_)))
                | Self::Service(ServiceError::Security(MlsError::Provider(
                    prns_groups_mls::ProviderError::Capacity(_)
                )))
                | Self::Service(ServiceError::Security(MlsError::Store(
                    StoreError::Capacity(_)
                        | StoreError::OperationRejected(
                            OperationRejection::RecoveryWindowFull
                                | OperationRejection::ReservedCapacity
                        )
                )))
        )
    }
}
impl From<NativeError> for Error {
    fn from(value: NativeError) -> Self {
        Self::Storage(value)
    }
}
impl From<prns_groups_service::Error<NativeError>> for Error {
    fn from(value: prns_groups_service::Error<NativeError>) -> Self {
        Self::Service(value)
    }
}
impl From<prns_groups_reticulum::Error<NativeError>> for Error {
    fn from(value: prns_groups_reticulum::Error<NativeError>) -> Self {
        Self::Transport(value)
    }
}

/// Durable enrollment mutations. Request IDs are caller-owned and must be reused
/// unchanged after an ambiguous reply. Keys and MLS bytes never cross this API.
#[derive(Clone, Debug)]
pub enum EnrollmentRequest {
    CancelInvitation {
        request_id: [u8; 32],
        invitation: [u8; 32],
        now: u64,
    },
    DismissInvitation {
        request_id: [u8; 32],
        invitation: [u8; 32],
        now: u64,
    },
    RemoveMember {
        request_id: [u8; 32],
        group: GroupId,
        member: Endpoint,
        expected_head: [u8; 32],
        now: u64,
    },
    Leave {
        request_id: [u8; 32],
        group: GroupId,
        now: u64,
    },
    Close {
        request_id: [u8; 32],
        group: GroupId,
        expected_head: [u8; 32],
        now: u64,
    },
    DeleteLocal {
        request_id: [u8; 32],
        group: GroupId,
        now: u64,
    },
    Create {
        request_id: [u8; 32],
        title: String,
        now: u64,
    },
    Invite {
        request_id: [u8; 32],
        group: GroupId,
        recipient: IdentityHash,
        now: u64,
    },
    Respond {
        request_id: [u8; 32],
        invitation: [u8; 32],
        accept: bool,
        now: u64,
    },
}
#[derive(Clone, Debug)]
pub enum EnrollmentResult {
    Created(GroupId),
    Invited([u8; 32]),
    Saved,
}
