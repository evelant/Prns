//! The shared platform DNS-SD adapter is implemented by the reusable interface layer.
pub use personal_rns::wifi_auto::{
    PlatformServiceDiscoveryBridge as AndroidServiceDiscoveryBridge,
    PublicationNameError, PublicationRegistrationOutcome, ServiceRecordRejection,
    ServiceRemovalOutcome, ServiceResolutionOutcome, SnapshotClearOutcome,
    TakeServiceDiscoveryError, DISCOVERY_CAPACITY, RESOLVED_CANDIDATE_INPUT_CAPACITY,
};
