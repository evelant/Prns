#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::contract::*;
use prns_groups_runner_native::{EnrollmentRequest, EnrollmentResult, RequestToken};
use prns_identities_core::{LocalIdentityId, PublicIdentityHash};

fn scope(seed: u8, sequence: u64) -> (StorageScope, IdentitySecretKey) {
    let secret = IdentitySecretKey::new([seed; 64]);
    let identity = PrivateIdentityMaterial::from(secret.clone());
    (
        StorageScope {
            identity: LocalIdentityId {
                incarnation: [41; 16],
                sequence,
            },
            public_hash: PublicIdentityHash(*identity.identity_hash().as_bytes()),
        },
        secret,
    )
}

async fn directory(owner: &DevelopmentStoreOwner, request: DirectoryRequest) -> DirectoryResponse {
    owner
        .admit_directory_async(request)
        .unwrap()
        .await
        .unwrap()
        .unwrap()
}

async fn mailbox(owner: &DevelopmentStoreOwner, request: MailboxRequest) -> MailboxReply {
    owner
        .admit_mailbox_async(request)
        .unwrap()
        .await
        .unwrap()
        .unwrap()
}

fn outgoing(secret: &IdentitySecretKey, text: &[u8]) -> prns_lxmf::mailbox::NewOutboundMessage {
    let identity = prns_lxmf::direct::LocalLxmfIdentity::from_secret_bytes(secret).unwrap();
    let mut wire = [0; prns_lxmf::wire::MAX_BASIC_LXMF_WIRE_BYTES];
    let message = prns_lxmf::wire::compose_basic_direct_lxmf(
        [61; 16],
        identity.destination(),
        1_700_000_000_000,
        b"",
        text,
        None,
        &identity,
        &mut wire,
    )
    .unwrap();
    prns_lxmf::mailbox::NewOutboundMessage {
        message_id: message.message_id(),
        source: identity.destination(),
        destination: [61; 16],
        timestamp_unix_ms: 1_700_000_000_000,
        title: vec![],
        content: text.to_vec(),
        exact_wire: wire[..usize::from(message.wire_len())].to_vec(),
    }
}

fn incoming(secret: &IdentitySecretKey) -> prns_lxmf::mailbox::NewInboundMessage {
    let local = prns_lxmf::direct::LocalLxmfIdentity::from_secret_bytes(secret).unwrap();
    let remote =
        prns_lxmf::direct::LocalLxmfIdentity::from_secret_bytes(&IdentitySecretKey::new([57; 64]))
            .unwrap();
    let mut wire = [0; prns_lxmf::wire::MAX_BASIC_LXMF_WIRE_BYTES];
    let message = prns_lxmf::wire::compose_basic_direct_lxmf(
        local.destination(),
        remote.destination(),
        1_700_000_000_000,
        b"",
        b"accepted callback",
        None,
        &remote,
        &mut wire,
    )
    .unwrap();
    prns_lxmf::mailbox::NewInboundMessage {
        message_id: message.message_id(),
        source: remote.destination(),
        destination: local.destination(),
        timestamp_unix_ms: 1_700_000_000_000,
        title: vec![],
        content: b"accepted callback".to_vec(),
        exact_wire: wire[..usize::from(message.wire_len())].to_vec(),
        verification: prns_lxmf::LxmfVerification::Verified,
    }
}

#[tokio::test]
async fn lifecycle_fences_clone_admission_but_drains_accepted_work_and_private_inspection() {
    use prns_lxmf::mailbox::{AttemptCompletion, AttemptKey};
    let root = tempfile::tempdir().unwrap();
    let (a, ka) = scope(51, 1);
    let (b, kb) = scope(52, 2);
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb, ScopedOpen::Create).unwrap();
    first
        .initialize_groups_blocking(prns_host::IdentityHash::new(a.public_hash.0))
        .unwrap();
    second
        .initialize_groups_blocking(prns_host::IdentityHash::new(b.public_hash.0))
        .unwrap();
    let external_group = first.group_client();
    let external_mailbox = first.mailbox_submitter();
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    first.admit_test_barrier(entered, wait).unwrap();
    entry.recv().unwrap();
    let create = EnrollmentRequest::Create {
        request_id: RequestToken::new(1_700_000_000, [61; 20])
            .unwrap()
            .into_bytes(),
        title: "Accepted before pause".into(),
        now: 1_700_000_000,
    };
    drop(external_group.admit_enrollment(create.clone()).unwrap());
    let accepted = first
        .admit_mailbox_async(MailboxRequest::InsertOutbound(outgoing(
            &ka,
            b"accepted before pause",
        )))
        .unwrap();
    first.fence_fresh_mutations();
    assert!(external_group.admit_enrollment(create.clone()).is_err());
    assert!(first
        .admit_directory_async(DirectoryRequest::CreateManual {
            destination: [8; 16],
            identity: None,
            alias: None
        })
        .is_err());
    assert!(external_mailbox
        .submit(MailboxRequest::InsertOutbound(outgoing(&ka, b"late")))
        .await
        .is_err());
    release.send(()).unwrap();
    let MailboxReply::OutboundInserted { message, .. } = accepted.await.unwrap().unwrap() else {
        panic!("accepted");
    };
    let completion = || MailboxRequest::CompleteAttempt {
        key: AttemptKey {
            local_record_id: message.local_record_id,
            generation: 1,
        },
        completion: AttemptCompletion::Delivered {
            delivered_at_millis: 1_700_000_001_000,
            rtt_millis: None,
        },
    };
    assert!(
        external_mailbox.submit(completion()).await.is_ok(),
        "already admitted attempt may settle while draining"
    );
    assert!(
        external_mailbox
            .submit(MailboxRequest::InsertInbound(incoming(&ka)))
            .await
            .is_ok(),
        "accepted network callbacks may finish inbox insertion while draining"
    );
    first.fence_all_mutations();
    first.drain().await.unwrap();
    assert!(external_mailbox.submit(completion()).await.is_err());
    assert!(external_mailbox
        .submit(MailboxRequest::InsertInbound(incoming(&ka)))
        .await
        .is_err());
    assert!(external_group.conversations().await.is_err());
    let inspection = first.inspection_group_client();
    assert_eq!(
        inspection.conversations().await.unwrap()[0].title,
        "Accepted before pause"
    );
    assert!(inspection.admit_enrollment(create).is_err());
    assert!(inspection
        .invitations(1_700_000_000)
        .await
        .unwrap()
        .is_empty());
    assert!(inspection
        .enrollment_work(1_700_000_000)
        .await
        .unwrap()
        .is_empty());
    assert!(second
        .group_client()
        .conversations()
        .await
        .unwrap()
        .is_empty());
    directory(
        &second,
        DirectoryRequest::CreateManual {
            destination: [8; 16],
            identity: None,
            alias: Some("Still active".into()),
        },
    )
    .await;
    first.close().unwrap();
    second.close().unwrap();
}

#[tokio::test]
async fn dependency_revisions_are_durable_identity_scoped_and_only_change_with_facts() {
    async fn revision(owner: &DevelopmentStoreOwner, digest: [u8; 32]) -> u64 {
        owner
            .admit_dependency_revision(digest)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
    }
    let root = tempfile::tempdir().unwrap();
    let (a, ka) = scope(61, 1);
    let (b, kb) = scope(62, 2);
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb, ScopedOpen::Create).unwrap();
    first.fence_all_mutations();
    assert_eq!(revision(&first, [1; 32]).await, 1);
    assert_eq!(revision(&first, [1; 32]).await, 1);
    drop(first.admit_dependency_revision([2; 32]).unwrap());
    first.drain().await.unwrap();
    first.close().unwrap();
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka, ScopedOpen::Existing).unwrap();
    assert_eq!(
        revision(&first, [2; 32]).await,
        2,
        "lost observation reply retains its committed revision"
    );
    assert_eq!(
        revision(&first, [1; 32]).await,
        3,
        "facts returning to an old value still advance revision"
    );
    assert_eq!(revision(&second, [1; 32]).await, 1);
    first.close().unwrap();
    second.close().unwrap();
}

#[tokio::test]
async fn contacts_mailbox_and_receive_state_keep_original_owner_through_drain_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let (a, ka) = scope(11, 1);
    let (b, kb) = scope(12, 2);
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb.clone(), ScopedOpen::Create).unwrap();
    assert_eq!(first.scope(), Some(a));
    assert_eq!(second.scope(), Some(b));
    let peer = [19; 16];
    for (owner, alias) in [(&first, "Work Alice"), (&second, "Private Alice")] {
        directory(
            owner,
            DirectoryRequest::CreateManual {
                destination: peer,
                identity: Some([20; 16]),
                alias: Some(alias.into()),
            },
        )
        .await;
    }
    directory(
        &first,
        DirectoryRequest::SetPinned {
            destination: peer,
            pinned: true,
        },
    )
    .await;
    let MailboxReply::OutboundInserted {
        message: original_a,
        ..
    } = mailbox(
        &first,
        MailboxRequest::InsertOutbound(outgoing(&ka, b"first owner")),
    )
    .await
    else {
        panic!("insert");
    };
    let MailboxReply::OutboundInserted {
        message: original_b,
        ..
    } = mailbox(
        &second,
        MailboxRequest::InsertOutbound(outgoing(&kb, b"second owner")),
    )
    .await
    else {
        panic!("insert");
    };
    assert_eq!(
        original_a.local_record_id, original_b.local_record_id,
        "record IDs are intentionally scoped"
    );
    assert!(second
        .admit_mailbox_async(MailboxRequest::InsertOutbound(outgoing(
            &ka,
            b"wrong owner"
        )))
        .unwrap()
        .await
        .unwrap()
        .is_err());
    let destination = [70; 16];
    first
        .admit_receive_state(destination, Some(zeroize::Zeroizing::new(vec![1, 2, 3])))
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert!(second
        .admit_receive_state(destination, None)
        .unwrap()
        .await
        .unwrap()
        .unwrap()
        .is_none());
    assert!(first.admit_bluetooth(None, |_| {}).is_err());
    let stale = first.mailbox_submitter();
    // The waiter's disappearance does not move or cancel accepted contact work.
    let accepted = first
        .admit_directory_async(DirectoryRequest::SetAlias {
            destination: peer,
            alias: Some("Retained work alias".into()),
        })
        .unwrap();
    drop(accepted);
    first.drain().await.unwrap();
    first.close().unwrap();
    assert!(stale.submit(MailboxRequest::ListQueued).await.is_err());
    let MailboxReply::Listed { messages, .. } = mailbox(&second, MailboxRequest::ListQueued).await
    else {
        panic!("list");
    };
    assert_eq!(messages, vec![original_b.clone()]);
    let DirectoryResponse::Lookup(ContactLookupOutcome::Found { contact }) =
        directory(&second, DirectoryRequest::Get { destination: peer }).await
    else {
        panic!("contact");
    };
    assert_eq!(contact.alias.as_deref(), Some("Private Alice"));
    assert!(!contact.pinned);
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka, ScopedOpen::Existing).unwrap();
    let MailboxReply::Listed { messages, .. } = mailbox(&first, MailboxRequest::ListQueued).await
    else {
        panic!("list");
    };
    assert_eq!(messages, vec![original_a]);
    let DirectoryResponse::Lookup(ContactLookupOutcome::Found { contact }) =
        directory(&first, DirectoryRequest::Get { destination: peer }).await
    else {
        panic!("contact");
    };
    assert_eq!(contact.alias.as_deref(), Some("Retained work alias"));
    assert!(contact.pinned);
    assert_eq!(
        first
            .admit_receive_state(destination, None)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .as_slice(),
        &[1, 2, 3]
    );
    first.close().unwrap();
    second.close().unwrap();
}

#[tokio::test]
async fn direct_delivery_settlement_with_equal_local_ids_changes_only_original_owner() {
    use prns_lxmf::mailbox::{
        AttemptCompletion, AttemptKey, CompletionTransition, DurableLxmfDeliveryState,
        MailboxListRequest,
    };
    let root = tempfile::tempdir().unwrap();
    let (a, ka) = scope(41, 1);
    let (b, kb) = scope(42, 2);
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb.clone(), ScopedOpen::Create).unwrap();
    let MailboxReply::OutboundInserted {
        message: a_message, ..
    } = mailbox(
        &first,
        MailboxRequest::InsertOutbound(outgoing(&ka, b"alpha")),
    )
    .await
    else {
        panic!("insert");
    };
    let MailboxReply::OutboundInserted {
        message: b_message, ..
    } = mailbox(
        &second,
        MailboxRequest::InsertOutbound(outgoing(&kb, b"beta")),
    )
    .await
    else {
        panic!("insert");
    };
    assert_eq!(a_message.local_record_id, b_message.local_record_id);
    let retained_client = first.mailbox_submitter();
    let reply = retained_client
        .submit(MailboxRequest::CompleteAttempt {
            key: AttemptKey {
                local_record_id: a_message.local_record_id,
                generation: 1,
            },
            completion: AttemptCompletion::Delivered {
                delivered_at_millis: 1_700_000_001_000,
                rtt_millis: Some(17),
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        reply,
        MailboxReply::Completion(CompletionTransition::Applied { .. })
    ));
    first.close().unwrap();
    second.close().unwrap();
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka, ScopedOpen::Existing).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb, ScopedOpen::Existing).unwrap();
    let list = || {
        MailboxRequest::List(MailboxListRequest {
            peer: None,
            direction: None,
            before: None,
            limit: 10,
        })
    };
    let MailboxReply::Listed {
        messages: a_messages,
        ..
    } = mailbox(&first, list()).await
    else {
        panic!("list");
    };
    let MailboxReply::Listed {
        messages: b_messages,
        ..
    } = mailbox(&second, list()).await
    else {
        panic!("list");
    };
    assert!(matches!(
        a_messages[0].delivery_state,
        DurableLxmfDeliveryState::Delivered { .. }
    ));
    assert_eq!(b_messages, vec![b_message]);
    assert!(retained_client.submit(list()).await.is_err());
    first.close().unwrap();
    second.close().unwrap();
}

#[tokio::test]
async fn group_creation_retries_and_stale_clients_are_identity_scoped() {
    let root = tempfile::tempdir().unwrap();
    let (a, ka) = scope(21, 1);
    let (b, kb) = scope(22, 2);
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb, ScopedOpen::Create).unwrap();
    first
        .initialize_groups_blocking(prns_host::IdentityHash::new(a.public_hash.0))
        .unwrap();
    second
        .initialize_groups_blocking(prns_host::IdentityHash::new(b.public_hash.0))
        .unwrap();
    let request_id = RequestToken::new(1_700_000_000, [51; 20])
        .unwrap()
        .into_bytes();
    let create = |title: &str| EnrollmentRequest::Create {
        request_id,
        title: title.into(),
        now: 1_700_000_000,
    };
    // A queued creation belongs to first even if its caller disappears.
    drop(
        first
            .group_client()
            .admit_enrollment(create("Work group"))
            .unwrap(),
    );
    first.drain().await.unwrap();
    let first_client = first.group_client();
    let EnrollmentResult::Created(group_a) = first_client
        .admit_enrollment(create("Work group"))
        .unwrap()
        .await
        .unwrap()
    else {
        panic!("created");
    };
    let EnrollmentResult::Created(group_b) = second
        .group_client()
        .admit_enrollment(create("Private group"))
        .unwrap()
        .await
        .unwrap()
    else {
        panic!("created");
    };
    assert_ne!(group_a, group_b);
    assert!(first_client
        .for_group(group_b)
        .network_config()
        .await
        .is_err());
    assert!(second
        .group_client()
        .for_group(group_a)
        .network_config()
        .await
        .is_err());
    assert_eq!(
        first_client.conversations().await.unwrap()[0].group,
        group_a
    );
    assert_eq!(
        second.group_client().conversations().await.unwrap()[0].group,
        group_b
    );
    assert!(first
        .initialize_groups_blocking(prns_host::IdentityHash::new(b.public_hash.0))
        .is_err());
    first.close().unwrap();
    assert!(first_client.conversations().await.is_err());
    assert_eq!(
        second.group_client().conversations().await.unwrap()[0].title,
        "Private group"
    );
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka, ScopedOpen::Existing).unwrap();
    first
        .initialize_groups_blocking(prns_host::IdentityHash::new(a.public_hash.0))
        .unwrap();
    let EnrollmentResult::Created(recovered) = first
        .group_client()
        .admit_enrollment(create("Work group"))
        .unwrap()
        .await
        .unwrap()
    else {
        panic!("created");
    };
    assert_eq!(recovered, group_a);
    assert_eq!(first.group_client().conversations().await.unwrap().len(), 1);
    first.close().unwrap();
    second.close().unwrap();
}

#[tokio::test]
async fn contact_cards_use_scoped_keys_and_mismatched_or_incomplete_stores_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let (a, ka) = scope(31, 1);
    let (b, kb) = scope(32, 2);
    assert!(
        DevelopmentStoreOwner::open_scoped(root.path(), a, kb.clone(), ScopedOpen::Create).is_err()
    );
    let first =
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).unwrap();
    let second =
        DevelopmentStoreOwner::open_scoped(root.path(), b, kb, ScopedOpen::Create).unwrap();
    let (tx, rx) = oneshot::channel();
    first
        .admit_messaging_name(Some("Work persona".into()), move |value| {
            let _ = tx.send(value);
        })
        .unwrap();
    rx.await.unwrap().unwrap();
    let request = crate::contact_cards::Request::Export(ExportContactCardInput {
        expected_local_identity: a.public_hash.0,
    });
    let crate::contact_cards::Reply::Export(ContactCardExportOutcome::Ready { card }) =
        first.admit_contact_card(request).unwrap().await.unwrap()
    else {
        panic!("card");
    };
    assert_eq!(card.identity, a.public_hash.0);
    assert_eq!(card.display_name, "Work persona");
    let request = crate::contact_cards::Request::Export(ExportContactCardInput {
        expected_local_identity: b.public_hash.0,
    });
    assert!(matches!(
        first.admit_contact_card(request).unwrap().await.unwrap(),
        crate::contact_cards::Reply::Export(ContactCardExportOutcome::IdentityChanged)
    ));
    let request = crate::contact_cards::Request::Import(ImportContactCardInput {
        uri: card.uri.clone(),
        expected_local_identity: a.public_hash.0,
        alias: None,
    });
    assert!(matches!(
        second.admit_contact_card(request).unwrap().await.unwrap(),
        crate::contact_cards::Reply::Import(ContactCardImportOutcome::IdentityChanged)
    ));
    let request = crate::contact_cards::Request::Import(ImportContactCardInput {
        uri: card.uri,
        expected_local_identity: b.public_hash.0,
        alias: None,
    });
    assert!(matches!(
        second.admit_contact_card(request).unwrap().await.unwrap(),
        crate::contact_cards::Reply::Import(ContactCardImportOutcome::Saved { .. })
    ));
    assert_eq!(
        directory(&first, DirectoryRequest::List).await,
        DirectoryResponse::List(ContactListOutcome::Listed { contacts: vec![] })
    );
    first.close().unwrap();
    second.close().unwrap();
    let path = a.path_under(&root.path().join("identity-data")).unwrap();
    let wrong_scope = StorageScope {
        public_hash: b.public_hash,
        ..a
    };
    assert!(ScopedDatabase::open(&path, wrong_scope).is_err());
    assert!(
        DevelopmentStoreOwner::open_scoped(root.path(), a, ka.clone(), ScopedOpen::Create).is_err()
    );
    // redb may update allocation/header housekeeping when opened and closed;
    // verify the immutable binding and committed application content, not raw
    // page equality (and never print an entire service database on failure).
    let reopened = ScopedDatabase::open(&path, a).unwrap();
    assert_eq!(reopened.scope(), a);
    assert_eq!(
        messaging_name(reopened.database(), None).unwrap(),
        "Work persona"
    );
    drop(reopened);
    let incomplete = StorageScope {
        identity: LocalIdentityId {
            sequence: 3,
            ..a.identity
        },
        ..a
    };
    drop(
        ScopedDatabase::create_new(
            &incomplete
                .path_under(&root.path().join("identity-data"))
                .unwrap(),
            incomplete,
        )
        .unwrap(),
    );
    assert!(
        DevelopmentStoreOwner::open_scoped(root.path(), incomplete, ka, ScopedOpen::Existing)
            .is_err()
    );
}
