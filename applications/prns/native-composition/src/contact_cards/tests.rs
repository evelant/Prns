use super::*;
use crate::development_store::DevelopmentStoreOwner;
use crate::directory::DirectoryRequest;

fn owner(root: &Path) -> DevelopmentStoreOwner {
    DevelopmentStoreOwner::open(root, &root.join("application.redb")).unwrap()
}

fn identity(root: &Path, byte: u8) -> [u8; 16] {
    let mut vault = FileVault::new(root.join("identities"));
    vault
        .store(&IdentityLabel::new("primary").unwrap(), &[byte; 64])
        .unwrap();
    *PrivateIdentityMaterial::from_bytes([byte; 64])
        .identity_hash()
        .as_bytes()
}

fn signed_uri(byte: u8, service: [u8; 10]) -> String {
    let key = PrivateIdentityMaterial::from_bytes([byte; 64]);
    let mut wire = [0; MAX_CARD_BYTES];
    let length = prns_contact_card::encode_signed(
        CardFields {
            public_identity: key.public().as_bytes(),
            service_name_hash: &service,
            display_name: "Other maintainer",
        },
        &Signer(&key),
        &mut wire,
    )
    .unwrap();
    let mut uri = [0; MAX_URI_BYTES];
    let length = prns_contact_card::encode_uri(&wire[..length], &mut uri).unwrap();
    std::str::from_utf8(&uri[..length]).unwrap().to_owned()
}

fn import_input(expected: [u8; 16]) -> ImportContactCardInput {
    ImportContactCardInput {
        uri: signed_uri(19, messaging_service_hash()),
        expected_local_identity: expected,
        alias: None,
    }
}

fn imported(
    owner: &DevelopmentStoreOwner,
    input: ImportContactCardInput,
) -> ContactCardImportOutcome {
    match owner
        .admit_contact_card(Request::Import(input))
        .unwrap()
        .blocking_recv()
        .unwrap()
    {
        Reply::Import(outcome) => outcome,
        _ => panic!("wrong reply"),
    }
}

fn directory(owner: &DevelopmentStoreOwner, request: DirectoryRequest) -> DirectoryResponse {
    owner
        .admit_directory_async(request)
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap()
}

#[test]
fn offline_export_uses_persisted_profile_and_reticulum_destination() {
    let temp = tempfile::tempdir().unwrap();
    let local = identity(temp.path(), 7);
    let owner = owner(temp.path());
    owner
        .admit_messaging_name(Some("My phone".into()), |result| {
            result.unwrap();
        })
        .unwrap();
    let reply = owner
        .admit_contact_card(Request::Export(ExportContactCardInput {
            expected_local_identity: local,
        }))
        .unwrap()
        .blocking_recv()
        .unwrap();
    let Reply::Export(ContactCardExportOutcome::Ready { card }) = reply else {
        panic!("export failed")
    };
    assert_eq!(card.display_name, "My phone");
    assert_eq!(card.identity, local);
    assert_eq!(
        Some(card.destination),
        crate::messaging_profile::messaging_destination(local)
    );
    assert_eq!(
        card.service_name_hash,
        *prns_core::routing::announce::expand_name("lxmf", &["delivery"])
            .unwrap()
            .as_bytes()
    );
    assert_eq!(
        preview(&card.uri),
        ContactCardPreviewOutcome::Valid { card }
    );
    assert_eq!(
        directory(&owner, DirectoryRequest::List),
        DirectoryResponse::List(ContactListOutcome::Listed { contacts: vec![] })
    );
}

#[test]
fn preview_and_import_reject_tampering_and_unsupported_services_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let local = identity(temp.path(), 7);
    let owner = owner(temp.path());
    let mut input = import_input(local);
    let mut wire = [0; MAX_CARD_BYTES];
    let length = prns_contact_card::decode_uri(&input.uri, &mut wire).unwrap();
    wire[84] ^= 1;
    let mut uri = [0; MAX_URI_BYTES];
    let length = prns_contact_card::encode_uri(&wire[..length], &mut uri).unwrap();
    input.uri = std::str::from_utf8(&uri[..length]).unwrap().to_owned();
    assert!(matches!(
        preview(&input.uri),
        ContactCardPreviewOutcome::Invalid { .. }
    ));
    assert!(matches!(
        imported(&owner, input.clone()),
        ContactCardImportOutcome::Invalid { .. }
    ));
    input.uri = signed_uri(19, prns_contact_card::service_name_hash("telemetry.events"));
    assert_eq!(
        preview(&input.uri),
        ContactCardPreviewOutcome::UnsupportedService
    );
    assert_eq!(
        imported(&owner, input),
        ContactCardImportOutcome::UnsupportedService
    );
    assert!(matches!(
        preview(&"x".repeat(MAX_URI_BYTES + 1)),
        ContactCardPreviewOutcome::Invalid { .. }
    ));
    assert_eq!(
        directory(&owner, DirectoryRequest::List),
        DirectoryResponse::List(ContactListOutcome::Listed { contacts: vec![] })
    );
}

#[test]
fn duplicate_import_preserves_alias_pin_and_observed_name_across_restart() {
    let temp = tempfile::tempdir().unwrap();
    let local = identity(temp.path(), 7);
    let first_owner = owner(temp.path());
    let input = import_input(local);
    let ContactCardImportOutcome::Saved { contact } = imported(&first_owner, input.clone()) else {
        panic!("not saved")
    };
    assert_eq!(contact.alias.as_deref(), Some("Other maintainer"));
    assert_eq!(contact.announced_name, None);
    assert!(!contact.pinned);
    assert!(contact.is_messaging);
    let destination = contact.destination;
    directory(
        &first_owner,
        DirectoryRequest::SetAlias {
            destination,
            alias: Some("Private nickname".into()),
        },
    );
    directory(
        &first_owner,
        DirectoryRequest::SetPinned {
            destination,
            pinned: true,
        },
    );
    let DirectoryResponse::Mutation(ContactMutationOutcome::Updated { contact: expected }) =
        directory(
            &first_owner,
            DirectoryRequest::SaveDiscovered {
                destination,
                identity: contact.identity.unwrap(),
                announced_name: Some("Live name".into()),
            },
        )
    else {
        panic!("not updated")
    };
    first_owner.close().unwrap();
    let reopened = owner(temp.path());
    let mut input = input;
    input.alias = Some("Overwrite attempt".into());
    assert_eq!(
        imported(&reopened, input),
        ContactCardImportOutcome::Existing { contact: expected }
    );
}

#[test]
fn imports_fill_unknown_identity_but_never_replace_a_conflicting_identity() {
    for previous_identity in [None, Some([42; 16])] {
        let temp = tempfile::tempdir().unwrap();
        let local = identity(temp.path(), 7);
        let first_owner = owner(temp.path());
        let input = import_input(local);
        let ContactCardPreviewOutcome::Valid { card } = preview(&input.uri) else {
            panic!("invalid fixture")
        };
        directory(
            &first_owner,
            DirectoryRequest::CreateManual {
                destination: card.destination,
                identity: previous_identity,
                alias: Some("Keep me".into()),
            },
        );
        let outcome = imported(&first_owner, input.clone());
        if let Some(existing) = previous_identity {
            assert_eq!(
                outcome,
                ContactCardImportOutcome::IdentityConflict {
                    existing,
                    attempted: card.identity
                }
            );
        } else {
            let ContactCardImportOutcome::Updated { contact } = outcome else {
                panic!("not updated")
            };
            assert_eq!(contact.identity, Some(card.identity));
            assert_eq!(contact.alias.as_deref(), Some("Keep me"));
            assert!(contact.is_messaging);
        }
        first_owner.close().unwrap();
        let reopened = owner(temp.path());
        if let Some(existing) = previous_identity {
            assert_eq!(
                imported(&reopened, input),
                ContactCardImportOutcome::IdentityConflict {
                    existing,
                    attempted: card.identity
                }
            );
        }
    }
}

#[test]
fn execution_rechecks_persisted_identity_and_dropped_waiters_do_not_cancel_imports() {
    let temp = tempfile::tempdir().unwrap();
    let local = identity(temp.path(), 7);
    let first_owner = owner(temp.path());
    let (entered, wait_entered) = std::sync::mpsc::sync_channel(1);
    let (release, wait_release) = std::sync::mpsc::sync_channel(1);
    first_owner
        .admit_test_barrier(entered, wait_release)
        .unwrap();
    wait_entered.recv().unwrap();
    let receive = first_owner
        .admit_contact_card(Request::Import(import_input(local)))
        .unwrap();
    // Simulate a stale snapshot/external identity replacement before execution.
    let changed = identity(temp.path(), 8);
    release.send(()).unwrap();
    assert!(matches!(
        receive.blocking_recv().unwrap(),
        Reply::Import(ContactCardImportOutcome::IdentityChanged)
    ));
    assert_eq!(
        directory(&first_owner, DirectoryRequest::List),
        DirectoryResponse::List(ContactListOutcome::Listed { contacts: vec![] })
    );
    drop(
        first_owner
            .admit_contact_card(Request::Import(import_input(changed)))
            .unwrap(),
    );
    first_owner.close().unwrap();
    let reopened = owner(temp.path());
    assert!(matches!(
        imported(&reopened, import_input(changed)),
        ContactCardImportOutcome::Existing { .. }
    ));
}
