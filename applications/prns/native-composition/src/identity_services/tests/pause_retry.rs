use super::*;

#[tokio::test]
async fn running_worker_cannot_hide_a_fenced_network_scope() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(1).unwrap();
    let host = open_host(&events).await;
    let secret = IdentitySecretKey::new([81; 64]);
    let material = prns_core::identity::PrivateIdentityMaterial::from(secret.clone());
    let scope = StorageScope {
        identity: core::LocalIdentityId {
            incarnation: [82; 16],
            sequence: 1,
        },
        public_hash: core::PublicIdentityHash(*material.identity_hash().as_bytes()),
    };
    let mut identity = MessagingIdentity::open(
        root.path(),
        scope,
        core::KeyReference([83; 32]),
        secret,
        true,
        host.client(),
        &events,
    )
    .unwrap();
    // Exercise the private adapter's retry boundary; catalog admission is covered
    // by the driver/coordinator tests in the parent module.
    identity.register().await.unwrap();
    identity.start_network().await.unwrap();
    assert!(identity.task.is_some());
    identity.network.as_ref().unwrap().fence();
    assert!(identity.start_network().await.is_err());
    identity.pause().await.unwrap();
    host.stop().await.unwrap();
}

async fn retry_retirement_after_live_link_tracking_changes(retain_receipt: bool) {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(1).unwrap();
    let host = open_host(&events).await;
    let secret = IdentitySecretKey::new([73; 64]);
    let material = prns_core::identity::PrivateIdentityMaterial::from(secret.clone());
    let scope = StorageScope {
        identity: core::LocalIdentityId {
            incarnation: [74; 16],
            sequence: 1,
        },
        public_hash: core::PublicIdentityHash(*material.identity_hash().as_bytes()),
    };
    let mut identity = MessagingIdentity::open(
        root.path(),
        scope,
        core::KeyReference([75; 32]),
        secret,
        true,
        host.client(),
        &events,
    )
    .unwrap();
    identity.register().await.unwrap();
    identity.fence().unwrap();
    let network = identity.network.as_ref().unwrap();
    network.wait_drained().await;
    assert!(network.outgoing_links().unwrap().is_empty());

    // Model an original owned-Link snapshot after that Link already closed.
    // This isolated host has no live Link for the ID: the regression concerns
    // retirement replay arguments, not successful on-wire Link teardown.
    let original = vec![personal_rns::routing::links::LinkId::new([76; 16])];
    identity.pause_outgoing = Some(original.clone());
    let pause = identity.endpoints[0].prepare_pause(original).await.unwrap();
    assert!(pause.is_drained());
    assert!(
        matches!(
            identity.endpoints[0].prepare_pause(Vec::new()).await,
            Err(personal_rns::runtime::ServiceLifecycleError::Conflict)
        ),
        "recomputing from the now-empty live tracker is an incompatible retry"
    );
    if retain_receipt {
        identity.prepared_pause = Some(pause);
    }

    let report = identity.pause().await.unwrap();
    assert!(identity.store.is_none());
    assert!(identity.network.is_none());
    assert!(identity.endpoints.is_empty());
    assert!(identity.pause_outgoing.is_none());
    assert!(identity.prepared_pause.is_none());
    assert_eq!(identity.pause().await.unwrap(), report);
    host.stop().await.unwrap();
}

#[tokio::test]
async fn pause_retries_retained_receipt_after_live_links_have_closed() {
    retry_retirement_after_live_link_tracking_changes(true).await;
}

#[tokio::test]
async fn pause_retries_original_snapshot_after_prepare_reply_is_lost() {
    retry_retirement_after_live_link_tracking_changes(false).await;
}

#[tokio::test]
async fn failed_storage_close_cannot_be_reinterpreted_as_successful_reactivation() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(1).unwrap();
    let host = open_host(&events).await;
    let secret = IdentitySecretKey::new([77; 64]);
    let material = prns_core::identity::PrivateIdentityMaterial::from(secret.clone());
    let scope = StorageScope {
        identity: core::LocalIdentityId {
            incarnation: [78; 16],
            sequence: 1,
        },
        public_hash: core::PublicIdentityHash(*material.identity_hash().as_bytes()),
    };
    let mut identity = MessagingIdentity::open(
        root.path(),
        scope,
        core::KeyReference([79; 32]),
        secret,
        true,
        host.client(),
        &events,
    )
    .unwrap();
    identity.pause().await.unwrap();
    // Reproduce the retained post-failure state: endpoints/store are gone, but
    // the previous owner's join could not establish successful close evidence.
    identity.storage_close_failed = true;
    assert!(identity.pause().await.is_err());
    assert!(identity.prepare().await.is_err());
    assert!(identity.store.is_none());
    assert!(identity.register().await.is_err());
    assert!(identity.endpoints.is_empty());
    host.stop().await.unwrap();
}
