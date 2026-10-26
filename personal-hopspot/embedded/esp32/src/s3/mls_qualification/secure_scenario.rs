use alloc::{vec, vec::Vec};
use prns_groups_mls::{
    Admission, Event, GroupId, LocalIdentity, MemberBinding, OperationId, Output, Policy,
};
use prns_groups_store::{
    secure::SecureRequest, CommitOutcome, Coordinator, MaintenanceOutcome, OperationTicket,
    RecoveryOutcome, Revision, StoreError, StoreIncarnation,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{flash, telemetry, Error, GroupAdapter, Result};

const IDENTITY_KEY: &[u8] = b"qualification-identity-v1";
const FINALIZING_KEY: &[u8] = b"qualification-finalizing-v1";
const COMPLETE_KEY: &[u8] = b"qualification-complete-v1";
const GROUP: GroupId = GroupId([0x29; 32]);
// Closed fixture clock, not a source of UTC or a freshness/rollback guarantee.
const FIXTURE_TIME: u64 = 1_791_763_200;
const BODY: &[u8] = b"PRNS E290 secure MLS durable qualification";
const LAST_TICKET: u64 = 19;

fn ticket(incarnation: StoreIncarnation, sequence: u64) -> Result<OperationTicket> {
    OperationTicket::new(incarnation, sequence).map_err(|_| Error::Commit)
}
fn operation(incarnation: StoreIncarnation, sequence: u64) -> Result<OperationId> {
    Ok(ticket(incarnation, sequence)?.operation_id())
}

fn identity(
    store: &Coordinator<flash::Backend>,
    incarnation: StoreIncarnation,
    peer: u8,
) -> Result<LocalIdentity> {
    if let Some(bytes) = store.read(IDENTITY_KEY)? {
        let bytes = Zeroizing::new(bytes);
        if bytes.len() != 136 {
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
        return Ok(identity);
    }
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
    let mut bytes = Zeroizing::new(Vec::with_capacity(136));
    bytes.extend_from_slice(&binding.endpoint);
    bytes.extend_from_slice(&binding.incarnation.to_be_bytes());
    bytes.extend_from_slice(&binding.signing_key);
    bytes.extend_from_slice(secret.as_bytes());
    let transaction = store.begin(operation(incarnation, 1)?, store.head()?)?;
    transaction.put(IDENTITY_KEY, &bytes)?;
    if !matches!(transaction.commit()?, CommitOutcome::Committed(_)) {
        return Err(Error::Commit);
    }
    log::info!("MLS identity peer={peer} provisioned-durably");
    Ok(identity)
}

fn operation_output(
    adapter: &GroupAdapter,
    incarnation: StoreIncarnation,
    step: u64,
    label: &str,
    perform: impl FnOnce(
        OperationId,
    ) -> core::result::Result<
        prns_groups_mls::Committed,
        prns_groups_mls::Error<super::FlashError>,
    >,
) -> Result<Output> {
    let start = flash::begin_phase();
    let id = operation(incarnation, step)?;
    let recovered = adapter.retry(id)?;
    let replay = recovered.is_some();
    let output = match recovered {
        Some(output) => output,
        None => perform(id)?.output,
    };
    log::info!(
        "MLS operation step={step} retry={replay} duration_ms={} event={:?} wire_bytes={}",
        (embassy_time::Instant::now() - start).as_millis(),
        output.event,
        output.messages.iter().map(Vec::len).sum::<usize>()
    );
    flash::end_phase(label, start);
    Ok(output)
}

fn receive(
    adapter: &GroupAdapter,
    incarnation: StoreIncarnation,
    step: u64,
    wire: &[u8],
) -> Result<()> {
    let result = operation_output(adapter, incarnation, step, "receive", |id| {
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

fn output_digest(
    adapter: &GroupAdapter,
    incarnation: StoreIncarnation,
    sequence: u64,
) -> Result<[u8; 32]> {
    // Hash the complete durable output record, including its context and event,
    // rather than only wire bytes. The O namespace is the adapter's local key.
    let mut key = [0; 33];
    key[0] = b'O';
    key[1..].copy_from_slice(&operation(incarnation, sequence)?.0);
    let output = Zeroizing::new(adapter.coordinator().read(&key)?.ok_or(Error::Commit)?);
    Ok(Sha256::digest(&*output).into())
}

fn maintain(adapter: &GroupAdapter, request: SecureRequest, label: &str) -> Result<()> {
    let start = flash::begin_phase();
    match adapter.coordinator().maintain(request)? {
        MaintenanceOutcome::Complete(usage) => {
            log::info!("MLS secure maintenance phase={label} usage={usage:?}");
        }
        _ => return Err(Error::Maintenance),
    }
    flash::end_phase(label, start);
    Ok(())
}

fn compact_campaign(
    adapter: &GroupAdapter,
    incarnation: StoreIncarnation,
    cut: flash::Cut,
) -> Result<()> {
    if flash::campaign() < cut as u32 {
        flash::arm(cut, Some(output_digest(adapter, incarnation, 4)?))?;
        maintain(adapter, SecureRequest::Compact, "compaction")?;
        // A campaign without its expected reset is not evidence of recovery.
        return Err(Error::RecoveryDigest);
    }
    if flash::recovery_pending(cut) {
        let before = output_digest(adapter, incarnation, 4)?;
        flash::verify_output(before)?;
        maintain(adapter, SecureRequest::EraseRetired, "compaction-recovery")?;
        if output_digest(adapter, incarnation, 4)? != before {
            return Err(Error::RecoveryDigest);
        }
        flash::verify_single_capsule(0)?;
        flash::recovery_verified(cut);
    }
    Ok(())
}

fn marker(adapter: &GroupAdapter, key: &[u8]) -> Result<bool> {
    match adapter.coordinator().read(key)? {
        Some(value) if value == [1] => Ok(true),
        Some(_) => Err(Error::Application),
        None => Ok(false),
    }
}

fn write_marker(
    adapter: &GroupAdapter,
    incarnation: StoreIncarnation,
    sequence: u64,
    key: &[u8],
) -> Result<()> {
    if marker(adapter, key)? {
        return Ok(());
    }
    let tx = adapter.coordinator().begin(
        operation(incarnation, sequence)?,
        adapter.coordinator().head()?,
    )?;
    tx.put(key, &[1])?;
    if !matches!(tx.commit()?, CommitOutcome::Committed(_)) {
        return Err(Error::Commit);
    }
    Ok(())
}

fn finalize(adapter: &GroupAdapter, incarnation: StoreIncarnation, peer: usize) -> Result<()> {
    // Receipt retirement is an explicit fixture assertion: all application
    // messages were checked on their destination before writing FINALIZING_KEY.
    // Persisting this phase lets reset recovery skip already deleted outputs.
    let start = flash::begin_phase();
    if !marker(adapter, COMPLETE_KEY)? {
        write_marker(adapter, incarnation, 10, FINALIZING_KEY)?;
        for sequence in 2..=9 {
            let ack = operation(incarnation, sequence + 9)?;
            match adapter.coordinator().recover(ack)? {
                RecoveryOutcome::Committed(_) => {}
                RecoveryOutcome::NotCommitted { .. } => {
                    adapter.acknowledge(ack, operation(incarnation, sequence)?)?;
                }
                _ => return Err(Error::Maintenance),
            }
            if adapter.retry(operation(incarnation, sequence)?)?.is_some() {
                return Err(Error::RecoveryDigest);
            }
        }
        write_marker(adapter, incarnation, LAST_TICKET, COMPLETE_KEY)?;
    }
    flash::end_phase("acknowledge-results", start);
    maintain(
        adapter,
        SecureRequest::FinalizeThrough(ticket(incarnation, LAST_TICKET)?),
        "receipt-finalization",
    )?;
    maintain(adapter, SecureRequest::Compact, "final-compaction")?;
    flash::verify_single_capsule(peer)?;
    let old = operation(incarnation, 4)?;
    match adapter.coordinator().recover(old) {
        Ok(RecoveryOutcome::Retired {
            retired_through,
            revision,
        }) if retired_through == ticket(incarnation, LAST_TICKET)?
            && revision == Revision(LAST_TICKET) => {}
        _ => return Err(Error::Maintenance),
    }
    if !matches!(
        adapter
            .coordinator()
            .begin(old, adapter.coordinator().head()?),
        Err(StoreError::OutsideRecoveryWindow { .. })
    ) {
        return Err(Error::Maintenance);
    }
    log::info!("MLS secure retirement peer={peer} retired_through={LAST_TICKET} old-ticket=permanently-rejected");
    Ok(())
}

pub(super) fn run() -> Result<()> {
    super::crypto_probe::run()?;
    flash::verify_layout()?;
    let start = flash::begin_phase();
    // This expectation exists only across deliberately armed software resets;
    // once verified, subsequent commits are allowed to replace its ciphertext.
    let (alice_backend, alice_incarnation) = flash::mount(0)?;
    let (bob_backend, bob_incarnation) = flash::mount(1)?;
    flash::prepare_campaign(
        alice_backend.usage().revision.0 == 0,
        bob_backend.usage().revision.0 == 0,
    );
    flash::verify_storage()?;
    flash::verify_single_capsule(0)?;
    flash::verify_single_capsule(1)?;
    let limits = prns_groups_store::Limits {
        max_changes: 48,
        max_key_bytes: 64,
        max_value_bytes: 32 * 1024,
        max_staged_bytes: 60 * 1024,
    };
    let alice_store = Coordinator::new(alice_backend, limits)?;
    let bob_store = Coordinator::new(bob_backend, limits)?;
    flash::end_phase("mount-and-recover", start);
    let start = flash::begin_phase();
    let alice_identity = identity(&alice_store, alice_incarnation, 1)?;
    let bob_identity = identity(&bob_store, bob_incarnation, 2)?;
    flash::end_phase("durable-identities", start);
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
    let already_complete = marker(&alice, COMPLETE_KEY)? && marker(&bob, COMPLETE_KEY)?;
    let finalizing = marker(&alice, FINALIZING_KEY)? || marker(&bob, FINALIZING_KEY)?;
    if !finalizing && !already_complete {
        operation_output(&alice, alice_incarnation, 2, "create", |id| {
            alice.create(id, GROUP)
        })?;
        let package = operation_output(&bob, bob_incarnation, 2, "key-package", |id| {
            bob.key_package(id, FIXTURE_TIME)
        })?;
        let add = operation_output(&alice, alice_incarnation, 3, "add-member", |id| {
            alice.add(
                id,
                GROUP,
                &[package
                    .messages
                    .first()
                    .ok_or(prns_groups_mls::Error::CorruptState)?
                    .as_slice()],
                FIXTURE_TIME,
            )
        })?;
        operation_output(&bob, bob_incarnation, 3, "join", |id| {
            bob.join(
                id,
                GROUP,
                add.messages
                    .get(1)
                    .ok_or(prns_groups_mls::Error::CorruptState)?
                    .as_slice(),
                FIXTURE_TIME,
            )
        })?;
        drop((package, add));
        let sent = operation_output(&alice, alice_incarnation, 4, "send", |id| {
            alice.send(id, GROUP, BODY)
        })?;
        receive(
            &bob,
            bob_incarnation,
            4,
            sent.messages.first().ok_or(Error::Application)?,
        )?;
        drop(sent);
        let rekey = operation_output(&alice, alice_incarnation, 5, "rekey", |id| {
            alice.rekey(id, GROUP)
        })?;
        receive(
            &bob,
            bob_incarnation,
            5,
            rekey.messages.first().ok_or(Error::Application)?,
        )?;
        drop(rekey);
        compact_campaign(&alice, alice_incarnation, flash::Cut::CompactionPublished)?;
        compact_campaign(&alice, alice_incarnation, flash::Cut::OldArenaSectorErased)?;
        for (step, cut) in [
            (6, flash::Cut::SendPublished),
            (7, flash::Cut::OldCapsuleErased),
        ] {
            if flash::campaign() < cut as u32 {
                flash::arm(cut, None)?;
            }
            let sent =
                operation_output(&alice, alice_incarnation, step, "send-retirement", |id| {
                    alice.send(id, GROUP, BODY)
                })?;
            let digest = output_digest(&alice, alice_incarnation, step)?;
            let retry = alice
                .retry(operation(alice_incarnation, step)?)?
                .ok_or(Error::Commit)?;
            if sent.messages != retry.messages
                || digest != output_digest(&alice, alice_incarnation, step)?
            {
                return Err(Error::RecoveryDigest);
            }
            if flash::recovery_pending(cut) {
                // Captured after publication but before releasing the result:
                // immutable ciphertext and the full current capsule were checked
                // against the pre-reset snapshot after mount, then this outbox
                // was resolved twice without repeating the MLS send.
                flash::verify_storage()?;
                flash::verify_single_capsule(0)?;
                flash::recovery_verified(cut);
            } else if flash::campaign() < cut as u32 {
                return Err(Error::RecoveryDigest);
            }
            receive(
                &bob,
                bob_incarnation,
                step,
                retry.messages.first().ok_or(Error::Application)?,
            )?;
        }
        let sent = operation_output(&alice, alice_incarnation, 8, "post-recovery-send", |id| {
            alice.send(id, GROUP, BODY)
        })?;
        receive(
            &bob,
            bob_incarnation,
            8,
            sent.messages.first().ok_or(Error::Application)?,
        )?;
        let reply = operation_output(&bob, bob_incarnation, 9, "reply", |id| {
            bob.send(id, GROUP, BODY)
        })?;
        receive(
            &alice,
            alice_incarnation,
            9,
            reply.messages.first().ok_or(Error::Application)?,
        )?;
        write_marker(&alice, alice_incarnation, 10, FINALIZING_KEY)?;
        write_marker(&bob, bob_incarnation, 10, FINALIZING_KEY)?;
    }
    finalize(&alice, alice_incarnation, 0)?;
    finalize(&bob, bob_incarnation, 1)?;
    telemetry::sample("secure-complete");
    if already_complete {
        log::info!("MLS secure qualification already-complete alice_revision={} bob_revision={} retired_through={LAST_TICKET}",
            alice.coordinator().head()?.0, bob.coordinator().head()?.0);
    } else {
        log::info!("MLS secure qualification complete alice_revision={} bob_revision={} retired_through={LAST_TICKET} campaigns=4",
            alice.coordinator().head()?.0, bob.coordinator().head()?.0);
    }
    Ok(())
}
