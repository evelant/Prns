use alloc::vec::Vec;
use ed25519_dalek::{Signer, Verifier};
use prns_groups_mls::{LocalIdentity, MemberBinding};

use super::{Error, Result};

// RFC 8032 section 7.1, test 1; also the pinned RustCrypto provider's test_private_keys.json.
// This public test key is never persisted, enrolled, or used outside the local self-test.
const SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const PUBLIC: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

const SIGNATURE: [u8; 64] = [
    0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e, 0x82, 0x8a,
    0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65, 0x22, 0x49, 0x01, 0x55,
    0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e, 0x39, 0x70, 0x1c, 0xf9, 0xb4, 0x6b,
    0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24, 0x65, 0x51, 0x41, 0x43, 0x8e, 0x7a, 0x10, 0x0b,
];

pub(super) fn run() -> Result<()> {
    let signer = ed25519_dalek::SigningKey::from_bytes(&SEED);
    let public_matches = signer.verifying_key().as_bytes() == &PUBLIC;
    let signature_matches = signer.sign(b"").to_bytes() == SIGNATURE;
    let known_signature_valid = ed25519_dalek::VerifyingKey::from_bytes(&PUBLIC)
        .and_then(|key| key.verify(b"", &ed25519_dalek::Signature::from_bytes(&SIGNATURE)))
        .is_ok();
    log::info!("MLS crypto RFC8032-test1 public={public_matches} sign={signature_matches} verify={known_signature_valid}");
    let mut secret = Vec::with_capacity(64);
    secret.extend_from_slice(&SEED);
    secret.extend_from_slice(&PUBLIC);
    let known = LocalIdentity::from_parts(
        MemberBinding {
            endpoint: [0xfe; 32],
            incarnation: 1,
            signing_key: PUBLIC,
        },
        secret.into(),
    );
    log::info!(
        "MLS crypto before-mls-flash rfc8032-key-self-sign-verify={:?}",
        known.as_ref().map(|_| ())
    );
    let generated = LocalIdentity::generate([0xfd; 32], 1).and_then(|identity| {
        let (binding, secret) = identity.into_parts();
        LocalIdentity::from_parts(binding, secret)
    });
    log::info!(
        "MLS crypto before-mls-flash entropy-key-self-sign-verify={:?}",
        generated.as_ref().map(|_| ())
    );
    if !public_matches || !signature_matches || !known_signature_valid {
        return Err(Error::Policy(prns_groups_mls::PolicyError::Crypto));
    }
    known.map_err(Error::Policy)?;
    generated.map_err(Error::Policy)?;
    Ok(())
}
