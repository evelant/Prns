//! Live dynamic host registrations plus durable catalog/application owners.
//! Transport over independent nodes is covered by the group carrier wire tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::drop_non_drop,
    clippy::too_many_arguments
)]

use super::*;
use prns_groups_runner_native::{EnrollmentRequest, EnrollmentResult, RequestToken};
use prns_host::{HostConfig, HostRole, IdentityConfig, PersistenceConfig, PrnsLimits};
use prns_host_native::{owner::OwnedSession, ApplicationEventDispatch, NativeEmbedding};
use prns_identities_core as core;
use prns_identities_native::NativeStore;
use prns_identities_runtime as runtime;
use prns_lxmf::mailbox::{DurableLxmfMessage, NewOutboundMessage};

mod dependencies;
mod pause_retry;

type Disk = NativeStore<2, 16, 1>;
type Catalog = core::Catalog<2, 16, 1>;
type Coordinator<'a> = core::Coordinator<'a, 2, 16, 1>;
type Controller<'a> = runtime::Controller<'a, 2, 1, 4>;

fn budget() -> runtime::Budget {
    runtime::Budget {
        capacity: runtime::Resources {
            held_keys: 7,
            destinations: 7,
            request_handlers: 5,
            links: 65,
            queue_slots: 33,
        },
        infrastructure_reserve: runtime::Resources {
            held_keys: 1,
            destinations: 1,
            request_handlers: 1,
            links: 1,
            queue_slots: 1,
        },
    }
}

fn create(disk: &mut Disk, c: &mut Coordinator<'_>, name: &str, tag: u8) -> core::LocalIdentityId {
    let command = core::Command::Create(core::CreateRequest {
        source: core::KeySource::Generate,
        metadata: core::Metadata::new(name, None).unwrap(),
        services: core::ServiceSet::new(&[MESSAGING_SERVICE]).unwrap(),
    });
    let token = c
        .reserve(disk, command, core::CommandDigest([tag; 32]))
        .unwrap();
    c.begin(disk, token, &command).unwrap();
    let ticket = c.action(token).unwrap().unwrap().ticket();
    let verified = disk.publish_key(c, ticket, None).unwrap();
    let core::OperationResult::Created { identity } = c.ack_key(disk, ticket, verified).unwrap()
    else {
        panic!("created")
    };
    identity
}

fn enable(
    disk: &mut Disk,
    c: &mut Coordinator<'_>,
    id: core::LocalIdentityId,
    tag: u8,
    dependency: ServiceDependency,
) -> core::Action<1> {
    let report = core::DependencyReport::new(&[dependency], 0).unwrap();
    let command = core::Command::Transition(
        c.prepare(id, core::TransitionTarget::Enable, report)
            .unwrap(),
    );
    let token = c
        .reserve(disk, command, core::CommandDigest([tag; 32]))
        .unwrap();
    c.begin(disk, token, &command).unwrap();
    c.action(token).unwrap().unwrap()
}

async fn open_host(events: &MessagingEvents) -> Arc<OwnedSession> {
    let event_handler = events.clone();
    let announce_handler = events.clone();
    OwnedSession::open_with_embedding(
        HostConfig {
            identity: IdentityConfig::GenerateEphemeral,
            persistence: PersistenceConfig::Ephemeral,
            role: HostRole::Endpoint,
            destinations: Vec::new(),
            required_capabilities: Vec::new(),
            limits: PrnsLimits::balanced(),
        },
        NativeEmbedding {
            on_event: Some(Box::new(move |event| event_handler.on_event(&event))),
            accepted_announces: Some(Box::new(move |announce| {
                announce_handler.on_announce(&announce)
            })),
            application_events: ApplicationEventDispatch::NativeCallback,
            ..NativeEmbedding::default()
        },
    )
    .await
    .unwrap()
}

fn install(
    driver: &mut MessagingDriver,
    disk: &mut Disk,
    c: &Coordinator<'_>,
    action: core::Action<1>,
    root: &Path,
    host: &OwnedSession,
    events: &MessagingEvents,
    create: bool,
) {
    let record = c.identity(action.ticket().identity).unwrap();
    let secret = disk.activate_key(c, action.ticket()).unwrap();
    driver
        .install(
            MessagingIdentity::open(
                root,
                StorageScope {
                    identity: record.id,
                    public_hash: record.public_hash,
                },
                record.key_reference,
                secret,
                create,
                host.client(),
                events,
            )
            .unwrap(),
        )
        .unwrap();
}

macro_rules! finish_step {
    ($expression:expr, $driver:expr, $authority:expr) => {
        loop {
            match $expression.unwrap() {
                runtime::Progress::Complete(value) => break value,
                runtime::Progress::Pending => $driver.advance(&$authority).await.unwrap(),
            }
        }
    };
}

async fn activate(
    disk: &mut Disk,
    c: &mut Coordinator<'_>,
    controller: &mut Controller<'_>,
    driver: &mut MessagingDriver,
    action: core::Action<1>,
    start: bool,
) -> runtime::NetworkPermit {
    let owner = controller
        .begin_activation(&disk.runtime_authority(c), action, MessagingDriver::plan())
        .unwrap();
    activate_owner(disk, c, controller, driver, action, owner, start).await
}

async fn activate_owner(
    disk: &mut Disk,
    c: &mut Coordinator<'_>,
    controller: &mut Controller<'_>,
    driver: &mut MessagingDriver,
    action: core::Action<1>,
    owner: runtime::OwnerToken,
    start: bool,
) -> runtime::NetworkPermit {
    let receipt = {
        let authority = disk.runtime_authority(c);
        finish_step!(
            controller.fence_previous_workers(&authority, driver, owner),
            driver,
            authority
        );
        finish_step!(
            controller.preflight(&authority, driver, owner),
            driver,
            authority
        );
        finish_step!(
            controller.prepare_service(&authority, driver, owner, MESSAGING_SERVICE),
            driver,
            authority
        );
        finish_step!(
            controller.activate_service(&authority, driver, owner, MESSAGING_SERVICE),
            driver,
            authority
        );
        controller.finish_activation(&authority, owner).unwrap()
    };
    c.ack_activated(disk, receipt.ticket()).unwrap();
    let id = action.ticket().identity;
    let guard = c
        .admit(
            id,
            c.identity(id).unwrap().public_hash,
            core::AdmissionClass::Network,
        )
        .unwrap();
    let authority = disk.runtime_authority(c);
    controller
        .confirm_activation(&authority, receipt, guard)
        .unwrap();
    let permit = controller
        .network_permit(&authority, guard, MESSAGING_SERVICE)
        .unwrap();
    if start {
        driver
            .start_network(&authority, controller, permit)
            .await
            .unwrap();
    }
    permit
}

async fn pause(
    disk: &mut Disk,
    c: &mut Coordinator<'_>,
    controller: &mut Controller<'_>,
    driver: &mut MessagingDriver,
    id: core::LocalIdentityId,
    tag: u8,
) {
    let report = core::DependencyReport::new(&[driver.dependencies(id).await.unwrap()], 0).unwrap();
    let review = c
        .prepare(id, core::TransitionTarget::Pause, report)
        .unwrap();
    let token = c
        .apply_stop(disk, review, core::CommandDigest([tag; 32]))
        .unwrap();
    let receipt = {
        let authority = disk.runtime_authority(c);
        let owner = controller
            .begin_quiescence(&authority, c.action(token).unwrap().unwrap())
            .unwrap();
        finish_step!(
            controller.quiesce_service(&authority, driver, owner, MESSAGING_SERVICE),
            driver,
            authority
        );
        finish_step!(
            controller.finish_quiescence(&authority, driver, owner),
            driver,
            authority
        )
    };
    c.ack_quiesced(disk, receipt.ticket(), receipt.report())
        .unwrap();
    controller
        .confirm_quiescence(&disk.runtime_authority(c), receipt)
        .unwrap();
}

fn group_create(tag: u8, title: &str, now: u64) -> EnrollmentRequest {
    EnrollmentRequest::Create {
        request_id: RequestToken::new(now, [tag; 20]).unwrap().into_bytes(),
        title: title.into(),
        now,
    }
}

async fn assert_registered(driver: &MessagingDriver, id: core::LocalIdentityId, expected: bool) {
    let entry = driver.entry(id).unwrap().services.lock().await;
    let local = LocalLxmfIdentity::from_secret_bytes(&entry.secret).unwrap();
    let identity = IdentityHash::new(entry.storage_scope.public_hash.0);
    // A live announce resolves the actual destination registry. HostSnapshot's
    // destination_identities is the learned-remote-identity cache, not this registry.
    for destination in [
        DestinationHash::new(local.destination()),
        prns_groups_reticulum::destination_for(identity).unwrap(),
        prns_groups_reticulum::enrollment::destination_for(identity).unwrap(),
    ] {
        let result = entry
            .host
            .execute(prns_host::HostCommand::Announce {
                destination,
                interface: None,
            })
            .await
            .unwrap();
        assert_eq!(
            matches!(result, Ok(prns_host::CommandOutcome::Announced)),
            expected,
            "actual local destination registration: {result:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_registration_is_reconciled_under_new_durable_authority() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let root = tempfile::tempdir().unwrap();
        let services_path = root.path().join("services");
        std::fs::create_dir(&services_path).unwrap();
        let mut disk = Disk::create_new(root.path().join("catalog")).unwrap();
        let mut working = Catalog::new([0; 16]);
        let mut staging = working.clone();
        let mut c = Coordinator::open(&mut disk, &mut working, &mut staging).unwrap();
        let id = create(&mut disk, &mut c, "Retained worker", 11);
        let action = enable(
            &mut disk,
            &mut c,
            id,
            12,
            ServiceDependency::clear(MESSAGING_SERVICE, 0),
        );
        let events = MessagingEvents::new(1).unwrap();
        let host = open_host(&events).await;
        let mut driver = MessagingDriver::new(1).unwrap();
        install(
            &mut driver,
            &mut disk,
            &c,
            action,
            &services_path,
            &host,
            &events,
            true,
        );
        let mut state = runtime::RuntimeState::new([43; 16], budget()).unwrap();
        let mut controller = Controller::new(&mut state);
        let owner = {
            let authority = disk.runtime_authority(&c);
            let owner = controller
                .begin_activation(&authority, action, MessagingDriver::plan())
                .unwrap();
            finish_step!(
                controller.fence_previous_workers(&authority, &mut driver, owner),
                driver,
                authority
            );
            finish_step!(
                controller.preflight(&authority, &mut driver, owner),
                driver,
                authority
            );
            finish_step!(
                controller.prepare_service(&authority, &mut driver, owner, MESSAGING_SERVICE),
                driver,
                authority
            );
            assert_eq!(
                controller
                    .activate_service(&authority, &mut driver, owner, MESSAGING_SERVICE)
                    .unwrap(),
                runtime::Progress::Pending
            );
            let entry = driver.entry(id).unwrap().services.clone();
            let held = entry.lock().await;
            assert!(
                tokio::time::timeout(Duration::from_millis(20), driver.advance(&authority))
                    .await
                    .is_err()
            );
            drop(held);
            owner
        };
        let reserved = controller.state().reserved_resources();
        drop(c);
        let mut c = Coordinator::open(&mut disk, &mut working, &mut staging).unwrap();
        let recovered = c.recovery_action(id).unwrap().unwrap();
        assert!(recovered.ticket().session > action.ticket().session);
        assert!(driver.advance(&disk.runtime_authority(&c)).await.is_err());
        driver
            .recover_pending(&disk.runtime_authority(&c), recovered)
            .await
            .unwrap();
        assert_eq!(
            controller.state().reserved_resources(),
            reserved,
            "recovery does not reset resource or mutation ownership"
        );
        assert_registered(&driver, id, false).await;
        let next_owner = controller
            .reconcile_activation(
                &disk.runtime_authority(&c),
                recovered,
                MessagingDriver::plan(),
            )
            .unwrap();
        assert_eq!(next_owner, owner);
        activate_owner(
            &mut disk,
            &mut c,
            &mut controller,
            &mut driver,
            recovered,
            next_owner,
            true,
        )
        .await;
        assert_registered(&driver, id, true).await;
        pause(&mut disk, &mut c, &mut controller, &mut driver, id, 13).await;
        host.stop().await.unwrap();
    })
    .await
    .expect("retained native work recovery is bounded");
}

async fn queued(driver: &MessagingDriver, id: core::LocalIdentityId) -> Vec<DurableLxmfMessage> {
    let entry = driver.entry(id).unwrap().services.lock().await;
    let MailboxReply::Listed { messages, .. } = entry
        .store
        .as_ref()
        .unwrap()
        .mailbox_submitter()
        .submit(MailboxRequest::ListQueued)
        .await
        .unwrap()
    else {
        panic!("queued")
    };
    messages
}

async fn insert_pending(
    driver: &MessagingDriver,
    id: core::LocalIdentityId,
    now: u64,
) -> NewOutboundMessage {
    let entry = driver.entry(id).unwrap().services.lock().await;
    let signer = LocalLxmfIdentity::from_secret_bytes(&entry.secret).unwrap();
    let mut wire = [0; prns_lxmf::wire::MAX_BASIC_LXMF_WIRE_BYTES];
    let message = prns_lxmf::wire::compose_basic_direct_lxmf(
        [87; 16],
        signer.destination(),
        now * 1000,
        b"",
        b"retained while offline",
        None,
        &signer,
        &mut wire,
    )
    .unwrap();
    let outbound = NewOutboundMessage {
        message_id: message.message_id(),
        source: signer.destination(),
        destination: [87; 16],
        timestamp_unix_ms: now * 1000,
        title: Vec::new(),
        content: b"retained while offline".to_vec(),
        exact_wire: wire[..usize::from(message.wire_len())].to_vec(),
    };
    entry
        .store
        .as_ref()
        .unwrap()
        .mailbox_submitter()
        .submit(MailboxRequest::InsertOutbound(outbound.clone()))
        .await
        .unwrap();
    outbound
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_host_named_identities_pause_resume_and_reopen_without_crossing_storage() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("catalog");
        let services_path = root.path().join("services");
        std::fs::create_dir(&services_path).unwrap();
        let mut disk = Disk::create_new(&catalog_path).unwrap();
        let mut working = Catalog::new([0; 16]);
        let mut staging = working.clone();
        let mut c = Coordinator::open(&mut disk, &mut working, &mut staging).unwrap();
        let a = create(&mut disk, &mut c, "Personal", 1);
        let b = create(&mut disk, &mut c, "Fieldwork", 2);
        let hash_a = c.identity(a).unwrap().public_hash;
        let hash_b = c.identity(b).unwrap().public_hash;
        let events = MessagingEvents::new(2).unwrap();
        let host = open_host(&events).await;
        assert_ne!(host.client().identity_hash().unwrap().as_bytes(), &hash_a.0);
        let mut driver = MessagingDriver::new(2).unwrap();
        let mut state = runtime::RuntimeState::new([41; 16], budget()).unwrap();
        let mut controller = Controller::new(&mut state);
        let action_a = enable(
            &mut disk,
            &mut c,
            a,
            3,
            ServiceDependency::clear(MESSAGING_SERVICE, 0),
        );
        install(
            &mut driver,
            &mut disk,
            &c,
            action_a,
            &services_path,
            &host,
            &events,
            true,
        );
        let permit_a = activate(
            &mut disk,
            &mut c,
            &mut controller,
            &mut driver,
            action_a,
            true,
        )
        .await;
        let action_b = enable(
            &mut disk,
            &mut c,
            b,
            4,
            ServiceDependency::clear(MESSAGING_SERVICE, 0),
        );
        install(
            &mut driver,
            &mut disk,
            &c,
            action_b,
            &services_path,
            &host,
            &events,
            true,
        );
        let permit_b = activate(
            &mut disk,
            &mut c,
            &mut controller,
            &mut driver,
            action_b,
            true,
        )
        .await;
        assert_registered(&driver, a, true).await;
        assert_registered(&driver, b, true).await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let request_a = group_create(31, "Personal group", now);
        let groups_a = driver.groups(a).await.unwrap();
        let EnrollmentResult::Created(group_a) = groups_a
            .admit_enrollment(request_a.clone())
            .unwrap()
            .await
            .unwrap()
        else {
            panic!("group")
        };
        let groups_b = driver.groups(b).await.unwrap();
        let request_b = group_create(31, "Field group", now); // Same token, different local owner.
        let EnrollmentResult::Created(group_b) = groups_b
            .admit_enrollment(request_b.clone())
            .unwrap()
            .await
            .unwrap()
        else {
            panic!("group")
        };
        assert_ne!(group_a, group_b);
        assert_eq!(groups_a.conversations().await.unwrap().len(), 1);
        assert_eq!(
            groups_b.conversations().await.unwrap()[0].title,
            "Field group"
        );
        let retained = insert_pending(&driver, a, now).await;
        assert!(queued(&driver, b).await.is_empty());
        let old_scope = driver
            .entry(a)
            .unwrap()
            .services
            .lock()
            .await
            .network
            .clone()
            .unwrap();

        pause(&mut disk, &mut c, &mut controller, &mut driver, a, 5).await;
        assert!(groups_a.admit_enrollment(request_a.clone()).is_err());
        assert!(old_scope.check_open().is_err());
        assert!(controller
            .validate_network(&disk.runtime_authority(&c), permit_a)
            .is_err());
        controller
            .validate_network(&disk.runtime_authority(&c), permit_b)
            .unwrap();
        let snapshot = host.client().snapshot().await.unwrap();
        assert!(snapshot.runtime.running);
        assert_registered(&driver, a, false).await;
        assert_registered(&driver, b, true).await;
        let EnrollmentResult::Created(retried_b) =
            groups_b.admit_enrollment(request_b).unwrap().await.unwrap()
        else {
            panic!("retry")
        };
        assert_eq!(retried_b, group_b);

        let action_a = enable(
            &mut disk,
            &mut c,
            a,
            6,
            driver.dependencies(a).await.unwrap(),
        );
        activate(
            &mut disk,
            &mut c,
            &mut controller,
            &mut driver,
            action_a,
            false,
        )
        .await;
        let resumed = driver.groups(a).await.unwrap();
        let EnrollmentResult::Created(retried_a) = resumed
            .admit_enrollment(request_a.clone())
            .unwrap()
            .await
            .unwrap()
        else {
            panic!("retry")
        };
        assert_eq!(retried_a, group_a);
        let pending = queued(&driver, a).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, retained.message_id);
        assert_eq!(pending[0].exact_wire, retained.exact_wire);
        assert!(
            groups_a.conversations().await.is_err(),
            "old client cannot follow the new owner"
        );
        pause(&mut disk, &mut c, &mut controller, &mut driver, a, 7).await;
        pause(&mut disk, &mut c, &mut controller, &mut driver, b, 8).await;
        assert_registered(&driver, a, false).await;
        assert_registered(&driver, b, false).await;
        assert_eq!(
            controller.state().reserved_resources(),
            runtime::Resources::ZERO
        );
        assert_eq!(controller.state().budget(), budget());
        host.stop().await.unwrap();
        drop(driver);
        drop(controller);
        drop(c);
        drop(disk);

        let mut disk = Disk::open(&catalog_path).unwrap();
        let mut c = Coordinator::open(&mut disk, &mut working, &mut staging).unwrap();
        assert_eq!(c.identity(a).unwrap().public_hash, hash_a);
        assert_eq!(c.identity(b).unwrap().public_hash, hash_b);
        let events = MessagingEvents::new(2).unwrap();
        let reopened_host = open_host(&events).await;
        let mut driver = MessagingDriver::new(2).unwrap();
        let mut state = runtime::RuntimeState::new([42; 16], budget()).unwrap();
        let mut controller = Controller::new(&mut state);
        let action = enable(
            &mut disk,
            &mut c,
            a,
            9,
            ServiceDependency::clear(MESSAGING_SERVICE, 0),
        );
        install(
            &mut driver,
            &mut disk,
            &c,
            action,
            &services_path,
            &reopened_host,
            &events,
            false,
        );
        activate(
            &mut disk,
            &mut c,
            &mut controller,
            &mut driver,
            action,
            false,
        )
        .await;
        let reopened = driver.groups(a).await.unwrap();
        let EnrollmentResult::Created(replayed) =
            reopened.admit_enrollment(request_a).unwrap().await.unwrap()
        else {
            panic!("reopen retry")
        };
        assert_eq!(replayed, group_a);
        assert_eq!(
            reopened.conversations().await.unwrap()[0].title,
            "Personal group"
        );
        let pending = queued(&driver, a).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].exact_wire, retained.exact_wire);
        assert!(controller
            .validate_network(&disk.runtime_authority(&c), permit_b)
            .is_err());
        pause(&mut disk, &mut c, &mut controller, &mut driver, a, 10).await;
        reopened_host.stop().await.unwrap();
    })
    .await
    .expect("bounded service lifecycle test");
}
