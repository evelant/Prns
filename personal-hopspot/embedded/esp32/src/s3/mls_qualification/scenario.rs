use alloc::{vec, vec::Vec};
use prns_groups_mls::{
    Admission, Event, GroupId, LocalIdentity, MemberBinding, OperationId, Output, Policy,
};
use prns_groups_store::{CommitOutcome, Coordinator, Revision};
use zeroize::Zeroizing;

use super::{flash, telemetry, Error, GroupAdapter, Result};

const IDENTITY_RECORD_BYTES: usize = 72 + 64;
const IDENTITY_KEY: &[u8] = b"qualification-identity-v1";
const GROUP: GroupId = GroupId([0x29; 32]);
// This closed local fixture has an explicit synthetic validation clock. It is not a device UTC service.
const FIXTURE_TIME: u64 = 1_791_763_200;
const BODY: &[u8] = b"PRNS E290 MLS durable qualification";

fn operation(value: u8) -> OperationId {
    OperationId([value; 32])
}

fn identity(store: &Coordinator<flash::Backend>, peer: u8) -> Result<LocalIdentity> {
    if let Some(bytes) = store.read(IDENTITY_KEY)? {
        let bytes = Zeroizing::new(bytes);
        if bytes.len() != IDENTITY_RECORD_BYTES {
            return Err(Error::IdentityRecord);
        }
        let binding = MemberBinding {
            endpoint: bytes[..32].try_into().map_err(|_| Error::IdentityRecord)?,
            incarnation: u64::from_be_bytes(
                bytes[32..40]
                    .try_into()
                    .map_err(|_| Error::IdentityRecord)?,
            ),
            signing_key: bytes[40..72]
                .try_into()
                .map_err(|_| Error::IdentityRecord)?,
        };
        if binding.endpoint != [peer; 32] || binding.incarnation != 1 {
            return Err(Error::IdentityRecord);
        }
        let identity = LocalIdentity::from_parts(binding, bytes[72..].to_vec().into())
            .map_err(Error::Policy)?;
        log::info!("MLS identity peer={peer} loaded signing-self-test=passed");
        telemetry::sample("identity-loaded");
        return Ok(identity);
    }
    // Missing identity beside any other durable state is corruption, never implicit replacement.
    if store.head()? != Revision(0) {
        return Err(Error::IdentityRecord);
    }
    let identity = LocalIdentity::generate([peer; 32], 1).map_err(Error::Policy)?;
    let (binding, secret) = identity.clone().into_parts();
    if secret.as_bytes().len() != 64 {
        return Err(Error::IdentityRecord);
    }
    let _validated =
        LocalIdentity::from_parts(binding.clone(), secret.clone()).map_err(Error::Policy)?;
    log::info!("MLS identity peer={peer} generated signing-self-test=passed");
    let mut bytes = Zeroizing::new(Vec::with_capacity(IDENTITY_RECORD_BYTES));
    bytes.extend_from_slice(&binding.endpoint);
    bytes.extend_from_slice(&binding.incarnation.to_be_bytes());
    bytes.extend_from_slice(&binding.signing_key);
    bytes.extend_from_slice(secret.as_bytes());
    let transaction = store.begin(operation(1), store.head()?)?;
    transaction.put(IDENTITY_KEY, &bytes)?;
    if !matches!(transaction.commit()?, CommitOutcome::Committed(_)) {
        return Err(Error::Commit);
    }
    log::info!("MLS identity peer={peer} provisioned-durably");
    Ok(identity)
}

fn operation_output(
    adapter: &GroupAdapter,
    step: u8,
    perform: impl FnOnce(
        OperationId,
    ) -> core::result::Result<
        prns_groups_mls::Committed,
        prns_groups_mls::Error<super::FlashError>,
    >,
) -> Result<Output> {
    let start = embassy_time::Instant::now().as_millis();
    let recovered = adapter.retry(operation(step))?;
    let replay = recovered.is_some();
    let output = match recovered {
        Some(output) => output,
        None => perform(operation(step))?.output,
    };
    log::info!(
        "MLS operation step={step} retry={replay} duration_ms={} event={:?} wire_bytes={}",
        embassy_time::Instant::now().as_millis() - start,
        output.event,
        output.messages.iter().map(Vec::len).sum::<usize>()
    );
    telemetry::sample("operation");
    Ok(output)
}

fn receive(adapter: &GroupAdapter, step: u8, wire: &[u8]) -> Result<()> {
    let result = operation_output(adapter, step, |id| {
        adapter.receive(id, GROUP, wire, |_| Admission::Accept)
    })?;
    match (&result.event, step) {
        (Event::Application(body), 4 | 6..=9) if body == BODY => {
            log::info!("MLS application step={step} plaintext=verified");
        }
        (Event::Control, 5) => {}
        _ => return Err(Error::Application),
    }
    Ok(())
}

pub(super) fn run() -> Result<()> {
    super::crypto_probe::run()?;
    flash::verify_layout()?;
    let heap_probe = vec![0u8; 1024];
    log::info!(
        "MLS global-heap-probe address={:p} bytes={}",
        heap_probe.as_ptr(),
        heap_probe.len()
    );
    drop(heap_probe);
    let limits = prns_groups_store::Limits {
        max_changes: 48,
        max_key_bytes: 64,
        max_value_bytes: 32 * 1024,
        max_staged_bytes: 60 * 1024,
    };
    let alice_store = Coordinator::new(flash::mount(0)?, limits)?;
    let bob_store = Coordinator::new(flash::mount(1)?, limits)?;
    flash::prepare_campaign(alice_store.head()?, bob_store.head()?)?;
    let alice_identity = identity(&alice_store, 1)?;
    telemetry::sample("identity-alice");
    let bob_identity = identity(&bob_store, 2)?;
    telemetry::sample("identity-bob");
    let owner = alice_identity.binding();
    let policy =
        Policy::new(owner.clone(), vec![owner, bob_identity.binding()]).map_err(Error::Policy)?;
    let limits = prns_groups_mls::Limits {
        max_members: 4,
        max_groups: 1,
        max_wire_bytes: 16 * 1024,
        max_plaintext_bytes: 1024,
        max_snapshot_bytes: 32 * 1024,
        max_key_packages: 2,
        max_retained_epochs: 1,
        max_forward_distance: 8,
        max_skipped_per_ratchet: 8,
        max_skipped_per_epoch: 16,
        max_device_skipped_keys: 32,
        max_pending_proposals: 4,
        max_pending_outputs: 24,
    };
    let alice = GroupAdapter::new(alice_store, alice_identity, policy.clone(), limits)?;
    let bob = GroupAdapter::new(bob_store, bob_identity, policy, limits)?;
    operation_output(&alice, 2, |id| alice.create(id, GROUP))?;
    let package = operation_output(&bob, 2, |id| bob.key_package(id, FIXTURE_TIME))?;
    let package_wire = package
        .messages
        .first()
        .ok_or(Error::Application)?
        .as_slice();
    let add = operation_output(&alice, 3, |id| {
        alice.add(id, GROUP, &[package_wire], FIXTURE_TIME)
    })?;
    let welcome = add.messages.get(1).ok_or(Error::Application)?.as_slice();
    operation_output(&bob, 3, |id| bob.join(id, GROUP, welcome, FIXTURE_TIME))?;
    drop((package, add));
    let sent = operation_output(&alice, 4, |id| alice.send(id, GROUP, BODY))?;
    receive(
        &bob,
        4,
        sent.messages.first().ok_or(Error::Application)?.as_slice(),
    )?;
    drop(sent);
    let rekey = operation_output(&alice, 5, |id| alice.rekey(id, GROUP))?;
    receive(
        &bob,
        5,
        rekey.messages.first().ok_or(Error::Application)?.as_slice(),
    )?;
    drop(rekey);
    for (step, cut) in [
        (6, flash::Cut::BeforePublish),
        (7, flash::Cut::TornBody),
        (8, flash::Cut::AfterPublish),
    ] {
        flash::arm(cut);
        let sent = operation_output(&alice, step, |id| alice.send(id, GROUP, BODY))?;
        // A dispatch resolves its exact persisted outbox again, including after a fresh mount.
        let retry = alice.retry(operation(step))?.ok_or(Error::Commit)?;
        if retry.messages != sent.messages {
            return Err(Error::RecoveryDigest);
        }
        receive(
            &bob,
            step,
            retry.messages.first().ok_or(Error::Application)?.as_slice(),
        )?;
        log::info!("MLS recovery step={step} exact-outbox=verified");
    }
    let reply = operation_output(&bob, 9, |id| bob.send(id, GROUP, BODY))?;
    receive(
        &alice,
        9,
        reply.messages.first().ok_or(Error::Application)?.as_slice(),
    )?;
    log::info!(
        "MLS qualification complete alice_revision={} bob_revision={}",
        alice.coordinator().head()?.0,
        bob.coordinator().head()?.0
    );
    Ok(())
}
