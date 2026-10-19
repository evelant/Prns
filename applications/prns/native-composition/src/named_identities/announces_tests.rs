#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use prns_host::{HostConfig, HostRole, IdentityConfig, PersistenceConfig, PrnsLimits};
use prns_host_native::{owner::OwnedSession, ApplicationEventDispatch, NativeEmbedding};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_owner_waits_for_interfaces_clears_paused_work_and_keeps_budget_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(MAX_IDENTITIES).unwrap();
    let event_handler = events.clone();
    let announce_handler = events.clone();
    let host = OwnedSession::open_with_embedding(
        HostConfig {
            identity: IdentityConfig::GenerateEphemeral,
            persistence: PersistenceConfig::Ephemeral,
            role: HostRole::Endpoint,
            destinations: vec![],
            required_capabilities: vec![],
            limits: PrnsLimits::balanced(),
        },
        NativeEmbedding {
            on_event: Some(Box::new(move |event| event_handler.on_event(&event))),
            accepted_announces: Some(Box::new(move |announce| {
                announce_handler.on_announce(&announce)
            })),
            application_events: ApplicationEventDispatch::NativeCallback,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut catalog = Box::new(Catalog::new([0; 16]));
    let mut scratch = Box::new(Catalog::new([0; 16]));
    let mut state = runtime::RuntimeState::new([5; 16], budget()).unwrap();
    let mut disk = Disk::create_new(root.path().join("named-identities")).unwrap();
    let coordinator = Coordinator::open(&mut disk, &mut catalog, &mut scratch).unwrap();
    let mut actor = Actor {
        root: root.path().to_owned(),
        disk,
        coordinator,
        controller: Controller::new(&mut state),
        driver: MessagingDriver::new(MAX_IDENTITIES).unwrap(),
        host: host.client(),
        events,
        selected: None,
        mutation_sequence: 0,
        create_receipts: Vec::new(),
        announces: SharedAnnounces::open(root.path()).unwrap(),
        jobs: jobs::Jobs::new(),
    };
    for (request, label) in [(2, "Personal"), (3, "Work")] {
        actor
            .create(CreateNamedIdentityInput {
                request_id: [request; 32],
                label: label.into(),
                published_name: None,
            })
            .await
            .unwrap();
    }
    actor.restore_announces().await.unwrap();
    let references: Vec<_> = actor
        .coordinator
        .snapshot()
        .identities()
        .map(NamedIdentityReference::from_record)
        .collect();
    let first = &references[0];
    let second = &references[1];
    assert!(actor.drive_announce().await.unwrap().is_none());
    assert!(matches!(
        actor
            .manual_announce_for(first, GROUP_SERVICE)
            .await
            .unwrap(),
        ManualOutcome::Queued
    ));
    assert_eq!(
        actor.announces.owner.snapshot().last_sequence,
        0,
        "no real interface must not spend startup budget"
    );
    for reference in &references {
        let settings = actor
            .saved_announce_settings(
                reference,
                Some(GroupAnnounceSettings {
                    interval_minutes: None,
                }),
            )
            .await
            .unwrap();
        actor
            .configure_announces(reference.id(), &settings)
            .unwrap();
    }
    actor
        .change(ChangeNamedIdentityInput {
            identity: first.clone(),
            change: NamedIdentityChange::Pause,
        })
        .await
        .unwrap();
    actor.reconcile_announces().await.unwrap();
    assert!(actor
        .manual_announce_for(first, GROUP_SERVICE)
        .await
        .is_err());
    actor
        .change(ChangeNamedIdentityInput {
            identity: first.clone(),
            change: NamedIdentityChange::Resume,
        })
        .await
        .unwrap();
    actor.reconcile_announces().await.unwrap();
    let now = actor.announces.now();
    assert!(
        actor
            .announces
            .owner
            .reserve_next(now, None, true, 0)
            .unwrap()
            .is_none(),
        "pause must clear the old manual intent even with immediate resume"
    );

    // Force only the scheduler's eligibility input for this host test. Native
    // dispatch still uses registered scoped destinations; no physical TX claim.
    actor
        .announces
        .owner
        .request_manual(key(first.id(), GROUP_SERVICE), now, None)
        .unwrap();
    let reservation = actor
        .announces
        .owner
        .reserve_next(now, None, true, 0)
        .unwrap()
        .unwrap();
    let accepted = actor.dispatch_announce(reservation).await.unwrap();
    assert_eq!(accepted, GROUP_CHANNELS);
    actor
        .announces
        .owner
        .complete(reservation, accepted, 0)
        .unwrap();
    let next_now = actor.announces.now();
    assert!(matches!(
        actor
            .announces
            .owner
            .request_manual(key(second.id(), DIRECT_SERVICE), next_now, None)
            .unwrap(),
        ManualRequest::Throttled { .. }
    ));
    actor
        .change(ChangeNamedIdentityInput {
            identity: first.clone(),
            change: NamedIdentityChange::Rename {
                label: "Personal renamed".into(),
            },
        })
        .await
        .unwrap();
    actor
        .change(ChangeNamedIdentityInput {
            identity: second.clone(),
            change: NamedIdentityChange::Select,
        })
        .await
        .unwrap();
    actor.reconcile_announces().await.unwrap();
    assert!(actor
        .announces
        .owner
        .reserve_next(actor.announces.now(), None, true, 0)
        .unwrap()
        .is_none());
    assert_eq!(actor.announces.owner.snapshot().last_sequence, 1);
    actor.close().await.unwrap();
    drop(actor);
    let mut restored =
        NativeAnnounces::<SLOTS>::open(root.path().join("named-announces"), 0, None).unwrap();
    assert!(matches!(
        restored
            .request_manual(key(second.id(), DIRECT_SERVICE), 0, None)
            .unwrap(),
        ManualRequest::Throttled {
            retry_after_ms: 60_000
        }
    ));
    assert!(restored
        .reserve_next(59_999, None, true, 0)
        .unwrap()
        .is_none());
    assert_eq!(
        restored
            .reserve_next(60_000, None, true, 0)
            .unwrap()
            .unwrap()
            .key(),
        key(second.id(), DIRECT_SERVICE)
    );
    host.stop().await.unwrap();
}
