#![cfg(feature = "host-test")]
#![allow(clippy::expect_used, clippy::panic)]

use personal_rns::identity::{PrivateIdentityMaterial, IDENTITY_SECRET_KEY_LEN};
use personal_rns::remote_control::{
    RemoteControlControllerAuthority, RemoteControlControllerCard, RemoteControlControllerIdentity,
    RemoteControlRequestKind, RemoteControlRequestSet, RemoteControlTargetAccess,
    RemoteControlTargetAccessCard, RemoteControlTargetIdentity,
};
use prns_app::contract::{
    ControllerSetupCard, ControllerSetupCardOutcome, DevelopmentNodeStartInput,
    DevelopmentNodeStartOutcome, DevelopmentNodeStopOutcome, IdentityCreationOutcome,
    ImportTargetAccessCardInput, TargetAccessCardImportOutcome, TargetAccessCardPreviewOutcome,
};
use prns_app::host_test as app;

fn target(fill: u8) -> RemoteControlTargetIdentity {
    RemoteControlTargetIdentity::new(
        PrivateIdentityMaterial::from_bytes([fill; IDENTITY_SECRET_KEY_LEN])
            .public()
            .public_keys(),
    )
}

fn card(
    recipient: RemoteControlControllerIdentity,
    target: &RemoteControlTargetIdentity,
    request: RemoteControlRequestKind,
) -> String {
    let access = RemoteControlTargetAccess::new(
        RemoteControlTargetIdentity::new(*target.public_keys()),
        RemoteControlControllerAuthority::Operator,
        RemoteControlRequestSet::only(request),
    )
    .expect("valid explicit Operator grant");
    let card = RemoteControlTargetAccessCard::new(recipient, access).expect("valid public card");
    let mut output = [0; RemoteControlTargetAccessCard::MAX_TEXT_LEN];
    let len = card.encode_text(&mut output).expect("encode card");
    String::from_utf8(output[..len].to_vec()).expect("ASCII card")
}

async fn exported() -> ControllerSetupCard {
    match app::export_controller_setup_card().await {
        ControllerSetupCardOutcome::Ready { card } => card,
        outcome => panic!("expected controller setup card: {outcome:?}"),
    }
}

async fn start(root: &std::path::Path) {
    let root = root.to_owned();
    let outcome = tokio::task::spawn_blocking(move || {
        app::start_configured(
            &root,
            DevelopmentNodeStartInput {
                development_tcp_target: None,
            },
        )
    })
    .await
    .expect("start worker");
    assert!(
        matches!(outcome, DevelopmentNodeStartOutcome::Started { .. }),
        "{outcome:?}"
    );
}

async fn stop() {
    assert_eq!(
        tokio::task::spawn_blocking(app::stop)
            .await
            .expect("stop worker"),
        DevelopmentNodeStopOutcome::Stopped
    );
}

#[tokio::test]
async fn public_setup_cards_require_trust_match_the_installation_and_survive_restart() {
    let temporary = tempfile::tempdir().expect("app storage");
    let root = temporary.path().join("prns/development");
    let create_root = root.clone();
    assert!(matches!(
        tokio::task::spawn_blocking(move || app::create_generated_identity(&create_root))
            .await
            .expect("create identity"),
        IdentityCreationOutcome::Created { .. }
    ));
    start(&root).await;
    let exported = exported().await;
    let controller = *RemoteControlControllerCard::decode_text(&exported.text)
        .expect("exported public card")
        .controller();
    assert_eq!(
        controller.identity_hash().as_bytes(),
        &exported.controller_identity_fingerprint
    );
    assert_eq!(exported.public_key_hex.len(), 128);
    assert_eq!(
        app::snapshot()
            .await
            .controller_identity_fingerprint
            .as_deref(),
        Some(exported.controller_identity_fingerprint.as_slice())
    );
    let target = target(0x71);
    let text = card(controller, &target, RemoteControlRequestKind::Describe);
    let preview = app::preview_target_access_card(text.clone());
    let TargetAccessCardPreviewOutcome::Valid { preview } = preview else {
        panic!("valid card preview")
    };
    assert_eq!(
        preview.target_identity_fingerprint,
        *target.identity_hash().as_bytes()
    );
    assert!(app::snapshot().await.paired_targets.is_empty());
    assert!(matches!(
        app::preview_target_access_card(format!("{text}00")),
        TargetAccessCardPreviewOutcome::Invalid { .. }
    ));
    let input = ImportTargetAccessCardInput {
        card: text,
        expected_controller_identity: exported.controller_identity_fingerprint,
        trusted_source_confirmed: false,
    };
    assert_eq!(
        app::import_target_access_card(input.clone()).await,
        TargetAccessCardImportOutcome::TrustRequired
    );
    let input = ImportTargetAccessCardInput {
        trusted_source_confirmed: true,
        ..input
    };
    assert_eq!(
        app::import_target_access_card(ImportTargetAccessCardInput {
            expected_controller_identity: [0; 16],
            ..input.clone()
        })
        .await,
        TargetAccessCardImportOutcome::ControllerChanged
    );
    let other_controller = RemoteControlControllerIdentity::new(target.public_keys().to_owned());
    assert_eq!(
        app::import_target_access_card(ImportTargetAccessCardInput {
            card: card(
                other_controller,
                &target,
                RemoteControlRequestKind::Describe
            ),
            ..input.clone()
        })
        .await,
        TargetAccessCardImportOutcome::RecipientMismatch
    );
    assert!(app::snapshot().await.paired_targets.is_empty());
    assert_eq!(
        app::import_target_access_card(input.clone()).await,
        TargetAccessCardImportOutcome::Saved {
            target_identity_fingerprint: *target.identity_hash().as_bytes()
        }
    );
    let saved = app::snapshot().await.paired_targets;
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0].permitted_requests,
        vec![prns_app::contract::RemoteControlRequestKind::Describe]
    );
    // Import does not need a reachable board and must not claim a remote operation succeeded.
    assert!(app::snapshot().await.active_operation.is_none());
    assert_eq!(
        app::import_target_access_card(ImportTargetAccessCardInput {
            card: card(controller, &target, RemoteControlRequestKind::AnnounceSelf),
            ..input.clone()
        })
        .await,
        TargetAccessCardImportOutcome::Conflict
    );
    assert_eq!(app::snapshot().await.paired_targets, saved);
    stop().await;
    start(&root).await;
    assert_eq!(exported, self::exported().await);
    assert_eq!(app::snapshot().await.paired_targets, saved);
    assert_eq!(
        app::import_target_access_card(input).await,
        TargetAccessCardImportOutcome::Existing {
            target_identity_fingerprint: *target.identity_hash().as_bytes()
        }
    );
    stop().await;
}
