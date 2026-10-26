use core::convert::Infallible;
use mls_rs::{
    client_builder::MlsConfig,
    error::MlsError,
    group::{
        mls_rules::{
            CommitDirection, CommitOptions, CommitSource, DefaultMlsRules, EncryptionOptions,
            MlsRules,
        },
        GroupContext, ReceiveRatchetLimitError, ReceiveRatchetLimits, ReceiveRatchetUsage, Roster,
    },
    identity::{
        basic::{BasicCredential, BasicIdentityProvider},
        SigningIdentity,
    },
    mls_rules::ProposalBundle,
    storage_provider::in_memory::InMemoryGroupStateStorage,
    CipherSuite, CipherSuiteProvider, Client, CryptoProvider,
};
use mls_rs_core::{
    error::IntoAnyError,
    group::{EpochRecord, GroupState, GroupStateStorage},
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;

const ROOMY: ReceiveRatchetLimits = ReceiveRatchetLimits {
    max_forward_distance: 32,
    max_skipped_keys_per_ratchet: 32,
    max_skipped_keys_per_epoch: 64,
};

#[derive(Clone, Copy)]
enum IncomingPath {
    Required,
    Standard,
}
#[derive(Clone)]
struct Rules {
    defaults: DefaultMlsRules,
    incoming_path: IncomingPath,
}
impl MlsRules for Rules {
    type Error = Infallible;
    fn filter_proposals(
        &self,
        direction: CommitDirection,
        source: CommitSource,
        roster: &Roster,
        context: &GroupContext,
        proposals: ProposalBundle,
    ) -> Result<ProposalBundle, Self::Error> {
        self.defaults
            .filter_proposals(direction, source, roster, context, proposals)
    }
    fn commit_options(
        &self,
        roster: &Roster,
        context: &GroupContext,
        proposals: &ProposalBundle,
    ) -> Result<CommitOptions, Self::Error> {
        self.defaults.commit_options(roster, context, proposals)
    }
    fn encryption_options(
        &self,
        roster: &Roster,
        context: &GroupContext,
    ) -> Result<EncryptionOptions, Self::Error> {
        self.defaults.encryption_options(roster, context)
    }
    fn require_receive_update_path(&self) -> bool {
        matches!(self.incoming_path, IncomingPath::Required)
    }
}
fn client(name: &[u8], incoming_path: IncomingPath) -> Result<Client<impl MlsConfig>, MlsError> {
    let crypto =
        RustCryptoProvider::with_enabled_cipher_suites(vec![CipherSuite::CURVE25519_AES128]);
    let suite = crypto
        .cipher_suite_provider(CipherSuite::CURVE25519_AES128)
        .unwrap();
    let (secret, public) = suite
        .signature_key_generate()
        .map_err(|error| MlsError::CryptoProviderError(error.into_any_error()))?;
    let signing = SigningIdentity::new(
        BasicCredential::new(name.to_vec()).into_credential(),
        public,
    );
    Ok(Client::builder()
        .identity_provider(BasicIdentityProvider)
        .crypto_provider(crypto)
        .group_state_storage(InMemoryGroupStateStorage::new())
        .mls_rules(Rules {
            defaults: DefaultMlsRules::new()
                .with_encryption_options(EncryptionOptions::new(true, Default::default())),
            incoming_path,
        })
        .signing_identity(signing, secret, CipherSuite::CURVE25519_AES128)
        .build())
}
macro_rules! pair {
    ($alice:ident, $a:ident, $bob:ident, $b:ident) => {
        let $alice = client(b"alice", IncomingPath::Standard)?;
        let $bob = client(b"bob", IncomingPath::Standard)?;
        let mut $a = $alice.group_builder()?.build()?;
        let kp = $bob.generate_key_package_message(Default::default(), Default::default(), None)?;
        let commit = $a.commit_builder().add_member(kp)?.build()?;
        $a.apply_pending_commit()?;
        let (mut $b, _) = $bob.join_group(None, &commit.welcome_messages[0], None)?;
        $a.set_receive_ratchet_limits(ROOMY)?;
        $b.set_receive_ratchet_limits(ROOMY)?;
    };
}
fn assert_limit<T>(result: Result<T, MlsError>, expected: ReceiveRatchetLimitError) {
    match result {
        Err(MlsError::ReceiveRatchetLimitExceeded(actual)) => assert_eq!(actual, expected),
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("expected receive limit rejection"),
    }
}

#[test]
fn forward_work_rejection_preserves_lower_generation() -> Result<(), MlsError> {
    pair!(alice, a, bob, b);
    b.set_receive_ratchet_limits(ReceiveRatchetLimits {
        max_forward_distance: 1,
        ..ROOMY
    })?;
    let mut messages = Vec::new();
    for _ in 0..3 {
        messages.push(a.encrypt_application_message(b"x", Default::default())?);
    }
    assert_limit(
        b.process_incoming_message(messages[2].clone()),
        ReceiveRatchetLimitError::ForwardDistance {
            requested: 2,
            limit: 1,
        },
    );
    assert_eq!(
        b.receive_ratchet_usage()?,
        ReceiveRatchetUsage {
            skipped_keys: 0,
            max_skipped_keys_in_ratchet: 0
        }
    );
    b.process_incoming_message(messages[0].clone())?;
    Ok(())
}

#[test]
fn per_ratchet_rejection_recovers_after_a_skipped_key_is_consumed() -> Result<(), MlsError> {
    pair!(alice, a, bob, b);
    b.set_receive_ratchet_limits(ReceiveRatchetLimits {
        max_skipped_keys_per_ratchet: 2,
        ..ROOMY
    })?;
    let mut messages = Vec::new();
    for _ in 0..5 {
        messages.push(a.encrypt_application_message(b"x", Default::default())?);
    }
    b.process_incoming_message(messages[2].clone())?;
    assert_limit(
        b.process_incoming_message(messages[4].clone()),
        ReceiveRatchetLimitError::PerRatchetSkippedKeys {
            requested: 3,
            limit: 2,
        },
    );
    assert_eq!(
        b.receive_ratchet_usage()?,
        ReceiveRatchetUsage {
            skipped_keys: 2,
            max_skipped_keys_in_ratchet: 2
        }
    );
    b.process_incoming_message(messages[0].clone())?;
    b.process_incoming_message(messages[4].clone())?;
    assert_eq!(
        b.receive_ratchet_usage()?,
        ReceiveRatchetUsage {
            skipped_keys: 2,
            max_skipped_keys_in_ratchet: 2
        }
    );
    Ok(())
}

#[test]
fn epoch_budget_spans_multiple_senders() -> Result<(), MlsError> {
    let alice = client(b"alice", IncomingPath::Standard)?;
    let bob = client(b"bob", IncomingPath::Standard)?;
    let charlie = client(b"charlie", IncomingPath::Standard)?;
    let mut a = alice.group_builder()?.build()?;
    let bk = bob.generate_key_package_message(Default::default(), Default::default(), None)?;
    let ck = charlie.generate_key_package_message(Default::default(), Default::default(), None)?;
    let commit = a.commit_builder().add_member(bk)?.add_member(ck)?.build()?;
    a.apply_pending_commit()?;
    let (mut b, _) = bob.join_group(None, &commit.welcome_messages[0], None)?;
    let (mut c, _) = charlie.join_group(None, &commit.welcome_messages[0], None)?;
    b.set_receive_ratchet_limits(ReceiveRatchetLimits {
        max_skipped_keys_per_epoch: 2,
        ..ROOMY
    })?;
    let mut am = Vec::new();
    let mut cm = Vec::new();
    for _ in 0..4 {
        am.push(a.encrypt_application_message(b"a", Default::default())?);
        cm.push(c.encrypt_application_message(b"c", Default::default())?);
    }
    b.process_incoming_message(am[1].clone())?;
    b.process_incoming_message(cm[1].clone())?;
    assert_limit(
        b.process_incoming_message(am[3].clone()),
        ReceiveRatchetLimitError::PerEpochSkippedKeys {
            requested: 3,
            limit: 2,
        },
    );
    assert_eq!(
        b.receive_ratchet_usage()?,
        ReceiveRatchetUsage {
            skipped_keys: 2,
            max_skipped_keys_in_ratchet: 1
        }
    );
    b.process_incoming_message(cm[0].clone())?;
    b.process_incoming_message(am[3].clone())?;
    Ok(())
}

#[test]
fn epoch_budget_spans_application_and_handshake_keys() -> Result<(), MlsError> {
    pair!(alice, a, bob, b);
    b.set_receive_ratchet_limits(ReceiveRatchetLimits {
        max_skipped_keys_per_epoch: 2,
        ..ROOMY
    })?;
    let early_application = a.encrypt_application_message(b"early", Default::default())?;
    let later_application = a.encrypt_application_message(b"later", Default::default())?;
    let mut proposals = Vec::new();
    for _ in 0..3 {
        proposals.push(a.propose_update(Default::default())?);
    }
    assert_eq!(b.cached_proposal_count(), 0);
    b.process_incoming_message(proposals[2].clone())?;
    assert_eq!(b.cached_proposal_count(), 1);
    assert_limit(
        b.process_incoming_message(later_application.clone()),
        ReceiveRatchetLimitError::PerEpochSkippedKeys {
            requested: 3,
            limit: 2,
        },
    );
    assert_eq!(
        b.receive_ratchet_usage()?,
        ReceiveRatchetUsage {
            skipped_keys: 2,
            max_skipped_keys_in_ratchet: 2
        }
    );
    b.process_incoming_message(proposals[0].clone())?;
    b.process_incoming_message(later_application)?;
    assert_eq!(
        b.receive_ratchet_usage()?,
        ReceiveRatchetUsage {
            skipped_keys: 2,
            max_skipped_keys_in_ratchet: 1
        }
    );
    b.process_incoming_message(early_application)?;
    Ok(())
}

#[test]
fn imported_current_and_prior_epochs_are_revalidated() -> Result<(), MlsError> {
    pair!(alice, a, bob, b);
    let mut messages = Vec::new();
    for _ in 0..3 {
        messages.push(a.encrypt_application_message(b"x", Default::default())?);
    }
    b.process_incoming_message(messages[2].clone())?;
    b.write_to_storage()?;
    drop(b);
    let mut b = bob.load_group(a.group_id())?;
    let low = ReceiveRatchetLimits {
        max_skipped_keys_per_ratchet: 1,
        ..ROOMY
    };
    assert_limit(
        b.set_receive_ratchet_limits(low),
        ReceiveRatchetLimitError::PerRatchetSkippedKeys {
            requested: 2,
            limit: 1,
        },
    );
    assert_eq!(b.receive_ratchet_limits(), None);
    b.set_receive_ratchet_limits(ROOMY)?;
    assert_limit(
        b.set_receive_ratchet_limits(low),
        ReceiveRatchetLimitError::PerRatchetSkippedKeys {
            requested: 2,
            limit: 1,
        },
    );
    assert_eq!(b.receive_ratchet_limits(), Some(ROOMY));
    let old_epoch = b.current_epoch();
    let commit = a.commit_builder().build()?;
    a.apply_pending_commit()?;
    b.process_incoming_message(commit.commit_message)?;
    b.write_to_storage()?;
    drop(b);
    let mut b = bob.load_group(a.group_id())?;
    b.set_receive_ratchet_limits(low)?;
    assert_limit(
        b.prior_epoch_receive_ratchet_usage(old_epoch),
        ReceiveRatchetLimitError::PerRatchetSkippedKeys {
            requested: 2,
            limit: 1,
        },
    );
    assert_limit(
        b.process_incoming_message(messages[0].clone()),
        ReceiveRatchetLimitError::PerRatchetSkippedKeys {
            requested: 2,
            limit: 1,
        },
    );
    b.set_receive_ratchet_limits(ROOMY)?;
    b.process_incoming_message(messages[0].clone())?;
    assert_eq!(
        b.prior_epoch_receive_ratchet_usage(old_epoch)?,
        Some(ReceiveRatchetUsage {
            skipped_keys: 1,
            max_skipped_keys_in_ratchet: 1
        })
    );
    Ok(())
}

#[test]
fn incoming_add_only_commit_obeys_path_rule() -> Result<(), MlsError> {
    let alice = client(b"alice", IncomingPath::Standard)?;
    let bob = client(b"bob", IncomingPath::Required)?;
    let charlie = client(b"charlie", IncomingPath::Standard)?;
    let mut a = alice.group_builder()?.build()?;
    let kp = bob.generate_key_package_message(Default::default(), Default::default(), None)?;
    let commit = a.commit_builder().add_member(kp)?.build()?;
    a.apply_pending_commit()?;
    let (mut b, _) = bob.join_group(None, &commit.welcome_messages[0], None)?;
    b.set_receive_ratchet_limits(ROOMY)?;
    let kp = charlie.generate_key_package_message(Default::default(), Default::default(), None)?;
    let commit = a.commit_builder().add_member(kp)?.build()?;
    assert!(matches!(
        b.process_incoming_message(commit.commit_message),
        Err(MlsError::CommitMissingPath)
    ));
    Ok(())
}

#[test]
fn generation_arithmetic_never_wraps() {
    assert_eq!(ROOMY.forward_distance(u32::MAX - 1, u32::MAX - 1), Ok(0));
    assert_eq!(
        ROOMY.forward_distance(u32::MAX - 1, u32::MAX),
        Err(ReceiveRatchetLimitError::GenerationExhausted)
    );
    assert_eq!(ROOMY.forward_distance(u32::MAX, 0), Ok(0));
    assert_eq!(
        ROOMY.forward_distance(0, 33),
        Err(ReceiveRatchetLimitError::ForwardDistance {
            requested: 33,
            limit: 32
        })
    );
}

#[test]
fn imported_snapshot_rejects_trailing_bytes_and_wrong_group_id() -> Result<(), MlsError> {
    pair!(alice, a, bob, b);
    b.write_to_storage()?;
    let id = b.group_id().to_vec();
    let mut store = bob.group_state_storage();
    let original = store.state(&id).unwrap().unwrap();
    let mut trailing = original.clone();
    trailing.push(0);
    store
        .write(
            GroupState {
                id: id.clone(),
                data: trailing,
            },
            vec![],
            vec![],
        )
        .unwrap();
    assert!(matches!(
        bob.load_group(&id),
        Err(MlsError::SerializationError(_))
    ));
    let wrong_id = b"misfiled group".to_vec();
    store
        .write(
            GroupState {
                id: wrong_id.clone(),
                data: original.clone(),
            },
            vec![],
            vec![],
        )
        .unwrap();
    assert!(matches!(
        bob.load_group(&wrong_id),
        Err(MlsError::GroupIdMismatch)
    ));
    store
        .write(
            GroupState {
                id: id.clone(),
                data: original,
            },
            vec![],
            vec![],
        )
        .unwrap();
    bob.load_group(&id)?;
    Ok(())
}

#[test]
fn imported_prior_epoch_rejects_trailing_bytes_and_misfiled_records_before_caching(
) -> Result<(), MlsError> {
    pair!(alice, a, bob, b);
    let old_epoch = b.current_epoch();
    for _ in 0..2 {
        let commit = a.commit_builder().build()?;
        a.apply_pending_commit()?;
        b.process_incoming_message(commit.commit_message)?;
    }
    b.write_to_storage()?;
    let id = b.group_id().to_vec();
    let mut store = bob.group_state_storage();
    let current = store.state(&id).unwrap().unwrap();
    let original = store.epoch(&id, old_epoch).unwrap().unwrap();
    let wrong_epoch = store.epoch(&id, old_epoch + 1).unwrap().unwrap();
    let mut other = bob.group_builder()?.build()?;
    other.commit_builder().build()?;
    other.apply_pending_commit()?;
    other.write_to_storage()?;
    let wrong_group = store.epoch(other.group_id(), 0).unwrap().unwrap();
    let mut trailing = original.clone();
    trailing.push(0);
    enum Expected {
        TrailingBytes,
        Epoch,
        Group,
    }
    for (data, expected) in [
        (trailing, Expected::TrailingBytes),
        (wrong_epoch, Expected::Epoch),
        (wrong_group, Expected::Group),
    ] {
        store
            .write(
                GroupState {
                    id: id.clone(),
                    data: current.clone(),
                },
                vec![],
                vec![EpochRecord::new(old_epoch, data)],
            )
            .unwrap();
        let mut loaded = bob.load_group(&id)?;
        loaded.set_receive_ratchet_limits(ROOMY)?;
        let rejected = loaded.prior_epoch_receive_ratchet_usage(old_epoch);
        match expected {
            Expected::TrailingBytes => {
                assert!(matches!(rejected, Err(MlsError::SerializationError(_))))
            }
            Expected::Epoch => assert!(matches!(rejected, Err(MlsError::InvalidEpoch))),
            Expected::Group => assert!(matches!(rejected, Err(MlsError::GroupIdMismatch))),
        }
        store
            .write(
                GroupState {
                    id: id.clone(),
                    data: current.clone(),
                },
                vec![],
                vec![EpochRecord::new(old_epoch, original.clone())],
            )
            .unwrap();
        assert!(loaded
            .prior_epoch_receive_ratchet_usage(old_epoch)?
            .is_some());
    }
    Ok(())
}
