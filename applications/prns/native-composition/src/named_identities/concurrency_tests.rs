#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use prns_host::{HostConfig, HostRole, IdentityConfig, PersistenceConfig, PrnsLimits};
use prns_host_native::{owner::OwnedSession, ApplicationEventDispatch, NativeEmbedding};
use std::time::Duration;
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
    match owner.catalog_request(Request::Catalog).await {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog,
        other => panic!("{other:?}"),
    }
}
async fn scoped<T: Send + 'static>(
    owner: &NamedRuntime,
    identity: NamedIdentityReference,
    class: core::AdmissionClass,
    action: impl for<'a> FnOnce(
            &'a mut MessagingIdentity,
        ) -> Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>
        + Send
        + 'static,
) -> Result<T, String> {
    let (tx, rx) = oneshot::channel();
    let work: Work = Box::new(move |services| {
        Box::pin(async move {
            let value = match services {
                Ok(s) => action(s).await,
                Err(e) => Err(e),
            };
            let _ = tx.send(value);
        })
    });
    owner
        .commands
        .send(Request::Service {
            identity,
            class,
            work,
        })
        .await
        .map_err(detail)?;
    rx.await.map_err(detail)?
}

async fn ready_activity(owner: &Arc<NamedRuntime>) -> Vec<NamedIdentityActivity> {
    let mut last = None;
    for _ in 0..50 {
        if let NamedIdentityActivityListOutcome::Ready { activities } = activity::list(owner).await
        {
            if activities
                .iter()
                .all(|entry| matches!(entry.activity, IdentityActivityOutcome::Ready { .. }))
            {
                return activities;
            }
            last = Some(activities);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("activity did not become ready: {last:?}");
}
fn unread(entry: &NamedIdentityActivity) -> u32 {
    match &entry.activity {
        IdentityActivityOutcome::Ready { summary } => summary.unread_direct,
        _ => panic!("unavailable"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_change_client_keeps_owner_after_selection_and_does_not_block_pause() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    let first = catalog(&owner).await.identities[0].reference.clone();
    let first_client = NamedIdentityClient {
        identity: first.clone(),
    };
    let initial = first_client
        .wait_lxmf_change_with_owner(owner.clone(), None)
        .await
        .unwrap();
    let first_service = first_client
        .call_with_owner(&owner, core::AdmissionClass::History, |services| {
            Box::pin(async move { services.direct().map_err(detail) })
        })
        .await
        .unwrap();
    let created = owner
        .catalog_request(|reply| {
            Request::Create(
                CreateNamedIdentityInput {
                    request_id: [152; 32],
                    label: "Second".into(),
                    published_name: None,
                },
                reply,
            )
        })
        .await;
    let NamedIdentityCatalogOutcome::Ready { catalog: created } = created else {
        panic!("create")
    };
    let second = created.identities[1].reference.clone();
    let second_client = NamedIdentityClient {
        identity: second.clone(),
    };
    let second_before = second_client
        .wait_lxmf_change_with_owner(owner.clone(), None)
        .await
        .unwrap();
    let mut waiting = Box::pin(
        first_client.wait_lxmf_change_with_owner(owner.clone(), Some(initial.projection_revision)),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut waiting)
            .await
            .is_err()
    );
    let selected = owner
        .catalog_request(|reply| {
            Request::Change(
                ChangeNamedIdentityInput {
                    identity: second,
                    change: NamedIdentityChange::Select,
                },
                reply,
            )
        })
        .await;
    assert!(
        matches!(selected, NamedIdentityCatalogOutcome::Ready { .. }),
        "an idle change wait must release its admitted job"
    );
    let remote = prns_lxmf::direct::LocalLxmfIdentity::from_secret_bytes(
        &prns_core::identity::vault::IdentitySecretKey::new([96; 64]),
    )
    .unwrap();
    let mut wire = [0; prns_lxmf::wire::MAX_BASIC_LXMF_WIRE_BYTES];
    let prepared = prns_lxmf::wire::compose_basic_direct_lxmf(
        first_service.local_destination(),
        remote.destination(),
        1_700_000_000_000,
        b"",
        b"immutable owner",
        None,
        &remote,
        &mut wire,
    )
    .unwrap();
    assert_eq!(
        first_service
            .callbacks()
            .on_host_link_delivery(&prns_host::LinkDelivery {
                link_id: prns_host::LinkId::new([13; 16]),
                local_destination: Some(prns_host::DestinationHash::new(
                    first_service.local_destination()
                )),
                arrived_at_millis: 1_700_000_000_005,
                source_interface: prns_host::InterfaceId::new([14; 8]),
                plaintext: wire[..usize::from(prepared.wire_len())].to_vec(),
            }),
        prns_lxmf::direct::CallbackOutcome::Enqueued
    );
    let received = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.mailbox_revision, 1);
    assert_eq!(
        second_client
            .wait_lxmf_change_with_owner(owner.clone(), None)
            .await
            .unwrap(),
        second_before,
        "changing UI selection must not retarget a captured client's stream"
    );
    let mut invalid = first.clone();
    invalid.public_hash = [0; 16];
    assert!(NamedIdentityClient { identity: invalid }
        .wait_lxmf_change_with_owner(owner.clone(), None)
        .await
        .is_none());

    let mut retiring = Box::pin(
        first_client.wait_lxmf_change_with_owner(owner.clone(), Some(received.projection_revision)),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut retiring)
            .await
            .is_err()
    );
    let paused = tokio::time::timeout(
        Duration::from_secs(2),
        owner.catalog_request(|reply| {
            Request::Change(
                ChangeNamedIdentityInput {
                    identity: first,
                    change: NamedIdentityChange::Pause,
                },
                reply,
            )
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(paused, NamedIdentityCatalogOutcome::Ready { .. }),
        "pause must not wait for the foreign change waiter"
    );
    let terminal = tokio::time::timeout(Duration::from_secs(2), retiring)
        .await
        .unwrap();
    assert!(terminal.is_none_or(|health| health.state == LxmfHealthState::Stopped));
    assert!(first_client
        .wait_lxmf_change_with_owner(owner.clone(), None)
        .await
        .is_none());
    owner.stop().await.unwrap();
    host.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_activity_selection_paused_seen_and_restart_are_scoped() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    let first = catalog(&owner).await.identities[0].reference.clone();
    let created = owner
        .catalog_request(|reply| {
            Request::Create(
                CreateNamedIdentityInput {
                    request_id: [151; 32],
                    label: "Second".into(),
                    published_name: None,
                },
                reply,
            )
        })
        .await;
    let NamedIdentityCatalogOutcome::Ready { catalog: created } = created else {
        panic!("create")
    };
    let second = created.identities[1].reference.clone();
    let empty = ready_activity(&owner).await;
    assert_eq!(empty.len(), 2);
    for entry in empty {
        let IdentityActivityOutcome::Ready { summary } = entry.activity else {
            panic!("new identity activity unavailable");
        };
        assert_eq!(summary.actionable_invitations, 0);
        assert_eq!(summary.unread_groups, 0);
        assert_eq!(summary.unread_direct, 0);
        assert!(summary.conversations.is_empty());
        assert!(!summary.partial);
    }
    for identity in [first.clone(), second.clone()] {
        let destination =
            crate::messaging_profile::messaging_destination(identity.public_hash).unwrap();
        scoped(
            &owner,
            identity,
            core::AdmissionClass::History,
            move |services| {
                Box::pin(async move {
                    services
                        .store()
                        .map_err(detail)?
                        .admit_mailbox_async(prns_lxmf::MailboxRequest::InsertInbound(
                            activity::tests::incoming([7; 16], destination, 1000),
                        ))
                        .map_err(detail)?
                        .await
                        .map_err(detail)?
                        .map_err(detail)?;
                    Ok(())
                })
            },
        )
        .await
        .unwrap();
    }
    let before = ready_activity(&owner).await;
    assert_eq!(before.iter().map(unread).collect::<Vec<_>>(), vec![1, 1]);
    owner
        .catalog_request(|reply| {
            Request::Change(
                ChangeNamedIdentityInput {
                    identity: second.clone(),
                    change: NamedIdentityChange::Select,
                },
                reply,
            )
        })
        .await;
    assert_eq!(
        ready_activity(&owner)
            .await
            .iter()
            .map(unread)
            .collect::<Vec<_>>(),
        vec![1, 1],
        "selection must not acknowledge any content"
    );
    owner
        .catalog_request(|reply| {
            Request::Change(
                ChangeNamedIdentityInput {
                    identity: first.clone(),
                    change: NamedIdentityChange::Pause,
                },
                reply,
            )
        })
        .await;
    let result = scoped(
        &owner,
        first.clone(),
        core::AdmissionClass::History,
        |services| {
            Box::pin(async move {
                assert!(!services.is_network_active());
                activity::operation(
                    services,
                    activity::Operation::Mark(MarkIdentityActivitySeenInput {
                        scope: IdentityActivityScope::Direct {
                            destination: [7; 16],
                        },
                        through_sequence: 1,
                    }),
                )
                .await
            })
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        activity::Reply::Mark(IdentityActivitySeenOutcome::Saved)
    ));
    assert_eq!(
        ready_activity(&owner)
            .await
            .iter()
            .map(unread)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let mut wrong = first.clone();
    wrong.public_hash = second.public_hash;
    assert!(
        scoped(&owner, wrong, core::AdmissionClass::History, |_| Box::pin(
            async { Ok(()) }
        ))
        .await
        .is_err()
    );
    owner.stop().await.unwrap();
    drop(owner);
    host.stop().await.unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let restarted_host = host_for_restart(&events).await;
    let owner = NamedRuntime::open(root.path(), restarted_host.client(), events)
        .await
        .unwrap();
    let after = ready_activity(&owner).await;
    assert_eq!(after.iter().map(unread).collect::<Vec<_>>(), vec![0, 1]);
    let restored = catalog(&owner).await;
    assert_eq!(restored.selected, Some(second));
    assert_eq!(restored.identities[0].state, NamedIdentityState::Paused);
    assert!(!restored.identities[0].ready);
    owner.stop().await.unwrap();
    restarted_host.stop().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unresolved_send_does_not_block_other_identity_and_pause_fences_before_drain() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    let first = catalog(&owner).await.identities[0].reference.clone();
    let created = owner
        .catalog_request(|r| {
            Request::Create(
                CreateNamedIdentityInput {
                    request_id: [99; 32],
                    label: "Other".into(),
                    published_name: None,
                },
                r,
            )
        })
        .await;
    let second = match created {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog.identities[1].reference.clone(),
        other => panic!("{other:?}"),
    };
    let (entered, started) = oneshot::channel();
    let sending = owner.clone();
    let sender = first.clone();
    let send = tokio::spawn(async move {
        scoped(&sending, sender, core::AdmissionClass::Network, move |s| {
            Box::pin(async move {
                let _ = entered.send(());
                direct::send(
                    s,
                    SendDirectTextInput {
                        destination: [199; 16],
                        title: String::new(),
                        content: "Unresolved peer".into(),
                    },
                )
                .await
            })
        })
        .await
    });
    started.await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !send.is_finished(),
        "the fixture must exercise an unresolved recipient"
    );
    let state = tokio::time::timeout(Duration::from_secs(1), catalog(&owner))
        .await
        .expect("catalog blocked behind another identity's send");
    assert_eq!(state.identities.len(), 2);
    let activity = tokio::time::timeout(Duration::from_secs(1), activity::list(&owner))
        .await
        .unwrap();
    let NamedIdentityActivityListOutcome::Ready { activities } = activity else {
        panic!("activity list unavailable");
    };
    assert!(
        matches!(
            activities[0].activity,
            IdentityActivityOutcome::Unavailable { .. }
        ),
        "busy activity must not queue behind an unresolved send or claim zero"
    );
    assert!(matches!(
        activities[1].activity,
        IdentityActivityOutcome::Ready { .. }
    ));
    assert!(state
        .identities
        .iter()
        .all(|identity| !identity.transitioning));
    let page = tokio::time::timeout(
        Duration::from_secs(1),
        scoped(&owner, second, core::AdmissionClass::History, |s| {
            Box::pin(async move { Ok(groups::conversations(s).await) })
        }),
    )
    .await
    .expect("history blocked behind another identity's resolver")
    .unwrap();
    assert!(matches!(page, GroupConversationListOutcome::Ready { .. }));
    let pause_owner = owner.clone();
    let pausing = first.clone();
    let pause = tokio::spawn(async move {
        pause_owner
            .catalog_request(|r| {
                Request::Change(
                    ChangeNamedIdentityInput {
                        identity: pausing,
                        change: NamedIdentityChange::Pause,
                    },
                    r,
                )
            })
            .await
    });
    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let current = catalog(&owner).await;
            let state = &current.identities[0];
            if state.state == NamedIdentityState::Paused {
                return state.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pause intent waited for the unresolved recipient");
    assert!(!observed.ready);
    assert!(
        observed.transitioning,
        "desired Pause must not claim completed drainage"
    );
    // A new publication on this owner must fail immediately while its old
    // admitted operation retains the lease and cancellation-safe result owner.
    let blocked = tokio::time::timeout(
        Duration::from_secs(1),
        scoped(
            &owner,
            first.clone(),
            core::AdmissionClass::ServiceMutation,
            |_| Box::pin(async { Ok(()) }),
        ),
    )
    .await
    .expect("fresh publication waited behind pause");
    assert!(blocked.is_err());
    let result = tokio::time::timeout(Duration::from_secs(20), send)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!matches!(result, SendDirectTextOutcome::Accepted { .. }));
    let paused = tokio::time::timeout(Duration::from_secs(2), pause)
        .await
        .unwrap()
        .unwrap();
    let NamedIdentityCatalogOutcome::Ready { catalog: paused } = paused else {
        panic!("{paused:?}");
    };
    assert_eq!(paused.identities[0].state, NamedIdentityState::Paused);
    assert!(!paused.identities[0].transitioning);
    owner.stop().await.unwrap();
    drop(owner);
    host.stop().await.unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host_for_restart(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    let restored = catalog(&owner).await;
    assert_eq!(restored.identities[0].reference, first);
    assert_eq!(restored.identities[0].state, NamedIdentityState::Paused);
    assert!(!restored.identities[0].transitioning);
    let resumed = owner
        .catalog_request(|r| {
            Request::Change(
                ChangeNamedIdentityInput {
                    identity: first,
                    change: NamedIdentityChange::Resume,
                },
                r,
            )
        })
        .await;
    assert!(
        matches!(resumed, NamedIdentityCatalogOutcome::Ready { .. }),
        "{resumed:?}"
    );
    owner.stop().await.unwrap();
    drop(owner);
    host.stop().await.unwrap();
}
async fn host_for_restart(events: &MessagingEvents) -> Arc<OwnedSession> {
    host(events).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_caller_keeps_admitted_work_owned_until_stop_drains_and_restart_recovers() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    let identity = catalog(&owner).await.identities[0].reference.clone();
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let calling_owner = owner.clone();
    let caller_identity = identity.clone();
    let caller = tokio::spawn(async move {
        scoped(
            &calling_owner,
            caller_identity,
            core::AdmissionClass::ServiceMutation,
            move |services| {
                Box::pin(async move {
                    entered.send(()).unwrap();
                    released.await.unwrap();
                    direct::mutation(
                        services,
                        crate::directory::DirectoryRequest::CreateManual {
                            destination: [47; 16],
                            identity: Some([48; 16]),
                            alias: Some("Committed after caller loss".into()),
                        },
                    )
                    .await
                })
            },
        )
        .await
    });
    started.await.unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let stopping_owner = owner.clone();
    let stopping = tokio::spawn(async move { stopping_owner.stop().await });
    // This observes the owner accepting Stop and closing its command receiver,
    // rather than relying on a scheduling sleep before checking the drain.
    tokio::time::timeout(Duration::from_secs(1), owner.commands.closed())
        .await
        .expect("Stop did not close fresh admission");
    assert!(!stopping.is_finished(), "Stop discarded its admitted job");
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), stopping)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(owner);
    host.stop().await.unwrap();

    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host_for_restart(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    assert_eq!(catalog(&owner).await.identities[0].reference, identity);
    let restored = scoped(
        &owner,
        identity,
        core::AdmissionClass::History,
        |services| {
            Box::pin(async move {
                direct::get(
                    services,
                    ContactDestinationInput {
                        destination: [47; 16],
                    },
                )
                .await
            })
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(restored, ContactLookupOutcome::Found { .. }),
        "{restored:?}"
    );
    owner.stop().await.unwrap();
    host.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_identity_history_requests_queue_behind_admitted_work() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let host = host(&events).await;
    let owner = NamedRuntime::open(root.path(), host.client(), events)
        .await
        .unwrap();
    let identity = catalog(&owner).await.identities[0].reference.clone();
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let running_owner = owner.clone();
    let running_identity = identity.clone();
    let running = tokio::spawn(async move {
        scoped(
            &running_owner,
            running_identity,
            core::AdmissionClass::History,
            move |_| {
                Box::pin(async move {
                    entered.send(()).unwrap();
                    released.await.unwrap();
                    Ok(())
                })
            },
        )
        .await
    });
    started.await.unwrap();
    let mut replies = Vec::new();
    for _ in 0..3 {
        let (reply, receive) = oneshot::channel();
        owner
            .commands
            .send(Request::Service {
                identity: identity.clone(),
                class: core::AdmissionClass::History,
                work: Box::new(move |services| {
                    Box::pin(async move {
                        let outcome = match services {
                            Ok(services) => direct::contacts(services).await,
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(outcome);
                    })
                }),
            })
            .await
            .unwrap();
        replies.push(receive);
    }
    // Catalog is behind these commands on the actor lane: its response proves
    // all three were handled while the original owner job still held its lock.
    tokio::time::timeout(Duration::from_secs(1), catalog(&owner))
        .await
        .unwrap();
    for reply in &mut replies {
        assert!(matches!(
            reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }
    release.send(()).unwrap();
    running.await.unwrap().unwrap();
    for reply in replies {
        let result = tokio::time::timeout(Duration::from_secs(2), reply)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(result, ContactListOutcome::Listed { .. }),
            "{result:?}"
        );
    }
    owner.stop().await.unwrap();
    host.stop().await.unwrap();
}
