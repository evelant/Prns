//! Exercise the production node composition, rather than the legacy unit fixture.
#![cfg(feature = "host-test")]
#![allow(clippy::expect_used, clippy::panic)]
use prns_app::{contract::*, host_test as app, named_identities as identities};
use std::path::Path;

fn catalog(outcome: NamedIdentityCatalogOutcome) -> NamedIdentityCatalog {
    match outcome {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog,
        other => panic!("catalog unavailable: {other:?}"),
    }
}
async fn change(
    identity: &NamedIdentityReference,
    change: NamedIdentityChange,
) -> NamedIdentityCatalog {
    catalog(
        identities::change_named_identity(ChangeNamedIdentityInput {
            identity: identity.clone(),
            change,
        })
        .await,
    )
}
async fn start(root: &Path) -> DevelopmentNodeSnapshot {
    let root = root.to_owned();
    match tokio::task::spawn_blocking(move || {
        app::start_configured(
            &root,
            DevelopmentNodeStartInput {
                development_tcp_target: None,
            },
        )
    })
    .await
    .expect("start thread")
    {
        DevelopmentNodeStartOutcome::Started { snapshot } => snapshot,
        other => panic!("node did not start: {other:?}"),
    }
}
async fn stop() {
    assert_eq!(
        tokio::task::spawn_blocking(app::stop)
            .await
            .expect("stop thread"),
        DevelopmentNodeStopOutcome::Stopped
    );
}
async fn contacts(client: &identities::NamedIdentityClient) -> Vec<Contact> {
    match client.list_contacts().await {
        ContactListOutcome::Listed { contacts } => contacts,
        other => panic!("contacts unavailable: {other:?}"),
    }
}
async fn groups(client: &identities::NamedIdentityClient) -> Vec<GroupConversation> {
    match client.list_group_conversations().await {
        GroupConversationListOutcome::Ready { conversations } => conversations,
        other => panic!("groups unavailable: {other:?}"),
    }
}
async fn card(
    client: &identities::NamedIdentityClient,
    owner: &NamedIdentityReference,
) -> ContactCard {
    match client
        .export_contact_card(ExportContactCardInput {
            expected_local_identity: owner.public_hash,
        })
        .await
    {
        ContactCardExportOutcome::Ready { card } => card,
        other => panic!("card unavailable: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_node_keeps_captured_owners_paused_history_and_restart_state() {
    let directory = tempfile::tempdir().expect("temporary app");
    let root = directory.path().join("prns/development");
    let create_root = root.clone();
    assert!(matches!(
        tokio::task::spawn_blocking(move || app::create_generated_identity(&create_root))
            .await
            .expect("bootstrap thread"),
        IdentityCreationOutcome::Created { .. }
    ));
    let first_node = start(&root).await;
    let first = catalog(identities::list_named_identities().await);
    assert_eq!(first.identities.len(), 1);
    let personal = first.identities[0].reference.clone();
    assert_eq!(first.selected, Some(personal.clone()));
    assert_eq!(first.identities[0].state, NamedIdentityState::Enabled);
    let personal_client = identities::open_named_identity(personal.clone());
    let original_card = card(&personal_client, &personal).await;
    assert!(
        original_card.display_name.is_empty(),
        "private labels are never published automatically"
    );

    let create = CreateNamedIdentityInput {
        request_id: [0x31; 32],
        label: "Work private".into(),
        published_name: None,
    };
    let second = catalog(identities::create_named_identity(create.clone()).await);
    let work = second
        .identities
        .iter()
        .find(|entry| entry.reference != personal)
        .expect("second identity")
        .reference
        .clone();
    let repeated = catalog(identities::create_named_identity(create).await);
    assert_eq!(
        repeated.identities.len(),
        2,
        "create retry must reuse its exact key"
    );
    let work_client = identities::open_named_identity(work.clone());
    let work_card = card(&work_client, &work).await;
    assert_ne!(original_card.identity, work_card.identity);
    assert_ne!(original_card.destination, work_card.destination);
    assert!(work_card.display_name.is_empty());
    change(&work, NamedIdentityChange::Select).await;

    // A client captured before selection changed must still mutate Personal.
    assert!(matches!(
        personal_client
            .import_contact_card(ImportContactCardInput {
                uri: work_card.uri.clone(),
                expected_local_identity: personal.public_hash,
                alias: Some("Work seen only by Personal".into()),
            })
            .await,
        ContactCardImportOutcome::Saved { .. }
    ));
    assert_eq!(contacts(&personal_client).await.len(), 1);
    assert!(contacts(&work_client).await.is_empty());
    let group = match personal_client
        .create_group(CreateGroupInput {
            client_request_id: prns_groups_runner_native::create_request_id()
                .expect("fresh group request"),
            title: "Personal history".into(),
        })
        .await
    {
        CreateGroupOutcome::Accepted { group_id } => group_id,
        other => panic!("group create failed: {other:?}"),
    };
    assert_eq!(
        groups(&personal_client).await[0].owner_identity,
        Some(personal.public_hash)
    );
    assert!(groups(&work_client).await.is_empty());

    let paused = change(&personal, NamedIdentityChange::Pause).await;
    assert_eq!(paused.selected, Some(work.clone()));
    assert_eq!(
        paused
            .identities
            .iter()
            .find(|entry| entry.reference == personal)
            .expect("Personal")
            .state,
        NamedIdentityState::Paused
    );
    assert!(
        paused
            .identities
            .iter()
            .find(|entry| entry.reference == work)
            .expect("Work")
            .ready
    );
    assert_eq!(groups(&personal_client).await[0].group_id, group);
    assert!(matches!(
        personal_client
            .list_group_messages(ListGroupMessagesInput {
                group_id: group,
                before_sequence: None,
                limit: 20
            })
            .await,
        GroupMessageListOutcome::Ready { .. }
    ));
    assert!(matches!(
        personal_client
            .set_contact_alias(SetContactAliasInput {
                destination: work_card.destination,
                alias: Some("Edited while paused".into())
            })
            .await,
        ContactMutationOutcome::Updated { .. }
    ));
    assert!(matches!(
        personal_client
            .set_messaging_name("Published name".into())
            .await,
        LocalMessagingProfileOutcome::Ready { .. }
    ));
    assert_eq!(
        card(&personal_client, &personal).await.display_name,
        "Published name"
    );
    assert!(matches!(
        personal_client
            .create_group(CreateGroupInput {
                client_request_id: prns_groups_runner_native::create_request_id()
                    .expect("fresh rejected request"),
                title: "Forbidden while paused".into()
            })
            .await,
        CreateGroupOutcome::Unavailable { .. }
    ));
    assert!(matches!(
        personal_client
            .send_direct_text(SendDirectTextInput {
                destination: work_card.destination,
                title: String::new(),
                content: "Forbidden while paused".into()
            })
            .await,
        SendDirectTextOutcome::DevelopmentUnavailable { .. }
    ));
    assert!(matches!(
        personal_client
            .list_lxmf_messages(ListLxmfMessagesInput {
                peer: None,
                before: None,
                limit: 20
            })
            .await,
        LxmfMessageListOutcome::Listed { .. }
    ));
    let mut incorrect = personal.clone();
    incorrect.public_hash[0] ^= 1;
    assert!(matches!(
        identities::open_named_identity(incorrect)
            .list_contacts()
            .await,
        ContactListOutcome::DevelopmentUnavailable { .. }
    ));
    stop().await;
    assert!(matches!(
        identities::list_named_identities().await,
        NamedIdentityCatalogOutcome::Unavailable { .. }
    ));
    let restarted_node = start(&root).await;
    assert_eq!(
        restarted_node.controller_identity_fingerprint, first_node.controller_identity_fingerprint,
        "persona lifecycle preserves installation management identity"
    );
    let restored = catalog(identities::list_named_identities().await);
    assert_eq!(restored.selected, Some(work.clone()));
    assert_eq!(
        restored
            .identities
            .iter()
            .find(|entry| entry.reference == personal)
            .expect("retained Personal")
            .state,
        NamedIdentityState::Paused
    );
    assert_eq!(restored.identities.len(), 2);
    // Existing foreign handles revalidate the same immutable owner on restart.
    assert_eq!(
        contacts(&personal_client).await[0].alias.as_deref(),
        Some("Edited while paused")
    );
    assert!(contacts(&work_client).await.is_empty());
    assert_eq!(groups(&personal_client).await[0].group_id, group);
    assert!(groups(&work_client).await.is_empty());
    assert_eq!(
        card(&personal_client, &personal).await.identity,
        personal.public_hash
    );
    let enabled = change(&personal, NamedIdentityChange::Resume).await;
    assert!(enabled.identities.iter().all(|entry| entry.ready));
    assert_eq!(enabled.selected, Some(work));
    assert_eq!(groups(&personal_client).await.len(), 1);
    stop().await;
}
