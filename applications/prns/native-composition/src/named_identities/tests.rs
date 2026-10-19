#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use prns_host::{HostConfig, HostRole, IdentityConfig, PersistenceConfig, PrnsLimits};
use prns_host_native::{owner::OwnedSession, ApplicationEventDispatch, NativeEmbedding};

async fn host(events: &MessagingEvents) -> Arc<OwnedSession> {
    let e = events.clone();
    let a = events.clone();
    OwnedSession::open_with_embedding(
        HostConfig {
            identity: IdentityConfig::GenerateEphemeral,
            persistence: PersistenceConfig::Ephemeral,
            role: HostRole::Endpoint,
            destinations: vec![],
            required_capabilities: vec![],
            limits: PrnsLimits::balanced(),
        },
        NativeEmbedding {
            application_events: ApplicationEventDispatch::NativeCallback,
            on_event: Some(Box::new(move |event| e.on_event(&event))),
            accepted_announces: Some(Box::new(move |announce| a.on_announce(&announce))),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
async fn catalog(owner: &NamedRuntime) -> NamedIdentityCatalog {
    let outcome = owner.catalog_request(Request::Catalog).await;
    match outcome {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog,
        other => panic!("{other:?}"),
    }
}
async fn change(
    owner: &NamedRuntime,
    identity: &NamedIdentityReference,
    change: NamedIdentityChange,
) {
    let outcome = owner
        .catalog_request(|r| {
            Request::Change(
                ChangeNamedIdentityInput {
                    identity: identity.clone(),
                    change,
                },
                r,
            )
        })
        .await;
    assert!(
        matches!(outcome, NamedIdentityCatalogOutcome::Ready { .. }),
        "{outcome:?}"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bound_operations_pause_history_and_restart_remain_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(dir.path(), host.client(), events)
        .await
        .unwrap();
    install_global(Some(owner.clone()));
    let first = catalog(&owner).await.identities[0].reference.clone();
    let input = CreateNamedIdentityInput {
        request_id: [42; 32],
        label: "Work".into(),
        published_name: Some("Work radio".into()),
    };
    let outcome = owner
        .catalog_request(|r| Request::Create(input.clone(), r))
        .await;
    let second = match outcome {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog.identities[1].reference.clone(),
        other => panic!("{other:?}"),
    };
    let a = open_named_identity(first.clone());
    let b = open_named_identity(second.clone());
    change(&owner, &second, NamedIdentityChange::Select).await;
    // Calling the old screen's handle still writes to the original identity.
    assert!(matches!(
        a.create_manual_contact(CreateManualContactInput {
            destination: [4; 16],
            identity: Some([5; 16]),
            alias: Some("Only personal".into())
        })
        .await,
        ContactMutationOutcome::Saved { .. }
    ));
    assert!(matches!(
        b.get_contact(ContactDestinationInput {
            destination: [4; 16]
        })
        .await,
        ContactLookupOutcome::NotFound
    ));
    let token = prns_groups_runner_native::RequestToken::new(direct::now() / 1000, [8; 20])
        .unwrap()
        .into_bytes();
    let action_reference = GroupActionReference {
        owner_identity: first.public_hash,
        client_request_id: token,
    };
    assert!(matches!(
        a.prepare_group_action(PrepareGroupActionInput {
            owner_identity: first.public_hash,
            input: GroupActionInput::Create {
                input: CreateGroupInput {
                    client_request_id: token,
                    title: "Private group".into()
                }
            }
        })
        .await,
        GroupActionOutcome::Ready { .. }
    ));
    let completed = a.execute_group_action(action_reference.clone()).await;
    let group = match &completed {
        GroupActionOutcome::Ready {
            action:
                GroupAction {
                    state:
                        GroupActionState::Completed {
                            result: GroupActionResult::Created { group_id },
                        },
                    ..
                },
        } => *group_id,
        other => panic!("{other:?}"),
    };
    change(&owner, &first, NamedIdentityChange::Pause).await;
    assert_eq!(
        a.execute_group_action(action_reference.clone()).await,
        completed,
        "paused exact-result recovery must not require activation"
    );
    assert_eq!(
        a.acknowledge_group_action(action_reference).await,
        GroupActionAcknowledgeOutcome::Removed
    );

    assert!(
        matches!(a.list_group_conversations().await,GroupConversationListOutcome::Ready{conversations} if conversations.len()==1)
    );
    assert!(matches!(
        a.list_group_messages(ListGroupMessagesInput {
            group_id: group,
            before_sequence: None,
            limit: 20
        })
        .await,
        GroupMessageListOutcome::Ready { .. }
    ));
    assert!(matches!(
        a.set_messaging_name("Paused personal".into()).await,
        LocalMessagingProfileOutcome::Ready { .. }
    ));
    let updated_catalog = catalog(&owner).await;
    assert_eq!(
        updated_catalog.identities[0].published_name.as_deref(),
        Some("Paused personal")
    );
    assert_eq!(
        updated_catalog.identities[1].published_name.as_deref(),
        Some("Work radio")
    );
    assert!(matches!(
        a.create_group(CreateGroupInput {
            client_request_id: prns_groups_runner_native::RequestToken::new(
                direct::now() / 1000,
                [9; 20]
            )
            .unwrap()
            .into_bytes(),
            title: "Blocked while paused".into()
        })
        .await,
        CreateGroupOutcome::Unavailable { .. }
    ));
    let mut stale = first.clone();
    stale.public_hash = [0; 16];
    assert!(matches!(
        open_named_identity(stale).list_contacts().await,
        ContactListOutcome::DevelopmentUnavailable { .. }
    ));
    // A stopped identity stays paused across a full native-owner restart. The
    // already-created foreign handle resolves only this same durable reference.
    install_global(None);
    owner.stop().await.unwrap();
    drop(owner);
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    // This host's original event wiring cannot be replaced. Stop it and create
    // the next host with fresh synchronous event callbacks, as production does.
    host.stop().await.unwrap();
    let restarted = host_for_restart(&events).await;
    let owner = NamedRuntime::open(dir.path(), restarted.client(), events)
        .await
        .unwrap();
    install_global(Some(owner.clone()));
    let after = catalog(&owner).await;
    assert_eq!(after.selected, Some(second.clone()));
    assert_eq!(after.identities[0].state, NamedIdentityState::Paused);
    assert_eq!(
        after.identities[0].published_name.as_deref(),
        Some("Paused personal")
    );
    assert!(matches!(
        a.get_contact(ContactDestinationInput {
            destination: [4; 16]
        })
        .await,
        ContactLookupOutcome::Found { .. }
    ));
    assert!(matches!(
        b.get_contact(ContactDestinationInput {
            destination: [4; 16]
        })
        .await,
        ContactLookupOutcome::NotFound
    ));
    // More transitions than the core journal's capacity retain create retry
    // evidence while recycling only completed lifecycle operations.
    for _ in 0..35 {
        change(&owner, &first, NamedIdentityChange::Resume).await;
        change(&owner, &first, NamedIdentityChange::Pause).await;
    }
    let outcome = owner
        .catalog_request(|r| Request::Create(input.clone(), r))
        .await;
    assert!(
        matches!(outcome,NamedIdentityCatalogOutcome::Ready{catalog} if catalog.identities.len()==2)
    );
    let mut conflict = input;
    conflict.label = "Different".into();
    assert!(matches!(
        owner
            .catalog_request(|r| Request::Create(conflict, r))
            .await,
        NamedIdentityCatalogOutcome::Rejected { .. }
    ));
    install_global(None);
    owner.stop().await.unwrap();
    drop(owner);
    restarted.stop().await.unwrap();
}
async fn host_for_restart(events: &MessagingEvents) -> Arc<OwnedSession> {
    host(events).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_open_and_closed_command_lane_release_the_durable_owner() {
    let dir = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let (tx, rx) = mpsc::channel(1);
    let (ready, startup) = oneshot::channel();
    drop(startup);
    run(
        dir.path().to_owned(),
        host.client(),
        events.clone(),
        rx,
        ready,
    )
    .await;
    drop(tx);
    // Cancellation after startup admission must drain before releasing the lock.
    let disk = Disk::open(dir.path().join("named-identities")).unwrap();
    drop(disk);
    let owner = NamedRuntime::open(dir.path(), host.client(), events)
        .await
        .unwrap();
    drop(owner);
    let mut reopened = false;
    for _ in 0..100 {
        if let Ok(disk) = Disk::open(dir.path().join("named-identities")) {
            drop(disk);
            reopened = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        reopened,
        "dropping all command handles must drain the identity actor"
    );
    host.stop().await.unwrap();
}

#[test]
fn abrupt_exit_after_create_publication_and_pause_intent_is_recoverable() {
    for point in ["created-key", "pause-intent"] {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("named_identities::tests::crash_child")
            .arg("--nocapture")
            .env("PRNS_NAMED_CRASH_ROOT", dir.path())
            .env("PRNS_NAMED_CRASH_POINT", point)
            .status()
            .unwrap();
        assert_eq!(
            status.code(),
            Some(77),
            "child missed durable {point} checkpoint"
        );
        let mut disk = Disk::open(dir.path().join("named-identities")).unwrap();
        let mut before = Catalog::new([0; 16]);
        core::Storage::load(&mut disk, &mut before).unwrap();
        let original = *before.identities().next().unwrap();
        drop(disk);
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
                let host = host(&events).await;
                let owner = NamedRuntime::open(dir.path(), host.client(), events)
                    .await
                    .unwrap();
                let recovered = catalog(&owner).await;
                assert_eq!(
                    recovered.identities[0].reference,
                    NamedIdentityReference::from_record(&original)
                );
                assert_eq!(
                    recovered.identities[0].state,
                    if point == "pause-intent" {
                        NamedIdentityState::Paused
                    } else {
                        NamedIdentityState::Enabled
                    }
                );
                assert_eq!(recovered.identities[0].ready, point == "created-key");
                owner.stop().await.unwrap();
                drop(owner);
                host.stop().await.unwrap();
            });
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_child() {
    let Ok(root) = std::env::var("PRNS_NAMED_CRASH_ROOT") else {
        return;
    };
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(Path::new(&root), host.client(), events)
        .await
        .unwrap();
    let identity = catalog(&owner).await.identities[0].reference.clone();
    change(&owner, &identity, NamedIdentityChange::Pause).await;
    panic!("crash checkpoint was not reached");
}
