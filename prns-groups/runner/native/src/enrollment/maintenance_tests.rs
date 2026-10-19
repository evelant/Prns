use super::*;
use prns_groups::retry::RetryError;

#[test]
fn receipt_reserve_keeps_last_slot_for_cleanup_that_releases_the_oldest_output() {
    assert!(Enrollment::ordinary_receipt_available(4093, 4096));
    assert!(!Enrollment::ordinary_receipt_available(4094, 4096));
    assert!(Enrollment::cleanup_receipt_available(4094, 4096, false));
    assert!(!Enrollment::cleanup_receipt_available(4095, 4096, false));
    assert!(Enrollment::cleanup_receipt_available(4095, 4096, true));
    assert!(!Enrollment::cleanup_receipt_available(4096, 4096, true));
    assert!(!Enrollment::ordinary_receipt_available(0, 1));
}

#[test]
fn deletion_and_retry_fence_publish_together_across_abort_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let db = Database::create(&path).unwrap();
    let mut a = Device::new(&db, 1);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "durable", NOW)
        .unwrap();
    let later = NOW + RETRY_HORIZON_SECS;
    let mut planned = a.engine.read().unwrap();
    let result = a
        .engine
        .maintenance_plan(&mut a.owner, &mut planned, later)
        .unwrap();
    assert_eq!(result.retired_requests, 1);
    assert_eq!(planned.request_retired_before, NOW);
    // Stage the exact maintenance catalog, then abandon its write transaction.
    // A crash before commit has the same old-catalog visibility boundary.
    {
        let tx = db.begin_write().unwrap();
        {
            let bytes = Zeroizing::new(encode(&planned).unwrap());
            tx.open_table(storage::CATALOG)
                .unwrap()
                .insert(0, bytes.as_slice())
                .unwrap();
        }
    }
    drop(a);
    drop(db);
    let db = Database::open(&path).unwrap();
    let mut a = Device::new(&db, 1);
    let prior = a.engine.read().unwrap();
    assert_eq!(prior.requests.len(), 1);
    assert_eq!(prior.request_retired_before, 0);
    assert_eq!(
        a.engine
            .create(&mut a.owner, request(1), "durable", later)
            .unwrap(),
        group
    );
    a.engine.maintain(&mut a.owner, later).unwrap();
    drop(a);
    drop(db);
    let db = Database::open(&path).unwrap();
    let mut a = Device::new(&db, 1);
    assert!(a.engine.read().unwrap().requests.is_empty());
    assert!(matches!(
        a.engine.create(&mut a.owner, request(1), "durable", NOW),
        Err(EnrollmentError::Retry(RetryError::Expired))
    ));
    assert_eq!(a.engine.groups(&mut a.owner).unwrap(), vec![group]);
}

#[test]
fn opaque_legacy_receipts_have_grace_but_create_recovery_does_not_depend_on_them() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::create(dir.path().join("db")).unwrap();
    let mut a = Device::new(&db, 1);
    let group = [7; 32];
    let mut catalog = a.engine.read().unwrap();
    catalog
        .entries
        .push(a.engine.entry(group, "legacy", None, NOW).unwrap());
    Enrollment::remember(
        &mut catalog,
        [1; 32],
        hash(&encode(&("create", "legacy")).unwrap()),
        group,
    )
    .unwrap();
    a.engine.write(&catalog).unwrap();
    a.engine.maintain(&mut a.owner, NOW).unwrap();
    assert_eq!(a.engine.read().unwrap().requests.len(), 1);
    assert_eq!(
        Enrollment::replay(
            &a.engine.read().unwrap(),
            [1; 32],
            hash(&encode(&("create", "legacy")).unwrap())
        )
        .unwrap(),
        Some(group)
    );
    a.engine
        .maintain(&mut a.owner, NOW + RETRY_HORIZON_SECS)
        .unwrap();
    assert!(a.engine.read().unwrap().requests.is_empty());
    // Catalog admission survived, although no MLS creation or API retry occurred.
    assert_eq!(a.engine.groups(&mut a.owner).unwrap(), vec![GroupId(group)]);
    assert!(matches!(
        a.engine.create(&mut a.owner, [1; 32], "legacy", NOW),
        Err(EnrollmentError::Retry(RetryError::InvalidToken))
    ));
}

#[test]
fn unsolicited_inbox_has_separate_peer_and_total_quotas_and_cannot_replay_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::create(dir.path().join("db")).unwrap();
    let mut a = Device::new(&db, 1);
    let group_a = a
        .engine
        .create(&mut a.owner, request(1), "local a", NOW)
        .unwrap();
    let group_b = a
        .engine
        .create(&mut a.owner, request(2), "local b", NOW)
        .unwrap();
    let make_offer = |peer: u8, id: u8| {
        let entry = a.engine.entry([id; 32], "incoming", None, NOW).unwrap();
        let mut member = entry.local.clone();
        member.rns = [peer; 16];
        Offer {
            id: [id; 32],
            group: [id; 32],
            title: "incoming".into(),
            owner: member,
            recipient: a.identity.into_bytes(),
            created: NOW,
            expires: NOW + INVITATION_LIFETIME_SECS,
        }
    };
    let offers = [
        make_offer(2, 10),
        make_offer(2, 11),
        make_offer(2, 12),
        make_offer(3, 13),
        make_offer(3, 14),
        make_offer(4, 15),
    ];
    for index in [0, 1] {
        let offer = &offers[index];
        a.engine
            .handle(
                &mut a.owner,
                IdentityHash::new(offer.owner.rns),
                &frame(&Frame::Offer(offer.clone())).unwrap(),
                NOW,
            )
            .unwrap();
    }
    // The third offer from one peer is rejected even while total space remains.
    {
        let offer = &offers[2];
        assert!(matches!(
            a.engine.handle(
                &mut a.owner,
                IdentityHash::new(offer.owner.rns),
                &frame(&Frame::Offer(offer.clone())).unwrap(),
                NOW
            ),
            Err(EnrollmentError::Capacity)
        ));
    }
    for index in [3, 4] {
        let offer = &offers[index];
        a.engine
            .handle(
                &mut a.owner,
                IdentityHash::new(offer.owner.rns),
                &frame(&Frame::Offer(offer.clone())).unwrap(),
                NOW,
            )
            .unwrap();
    }
    {
        let offer = &offers[5];
        assert!(matches!(
            a.engine.handle(
                &mut a.owner,
                IdentityHash::new(offer.owner.rns),
                &frame(&Frame::Offer(offer.clone())).unwrap(),
                NOW
            ),
            Err(EnrollmentError::Capacity)
        ));
    }
    // Exact duplicate does not charge another quota slot.
    let old = frame(&Frame::Offer(offers[0].clone())).unwrap();
    a.engine
        .handle(&mut a.owner, IdentityHash::new([2; 16]), &old, NOW)
        .unwrap();
    for id in 20..32 {
        let group = if id < 26 { group_a } else { group_b };
        a.engine
            .invite(
                &mut a.owner,
                request(id),
                group,
                IdentityHash::new([id; 16]),
                NOW,
            )
            .unwrap();
    }
    assert_eq!(a.engine.read().unwrap().invites.len(), MAX_INVITATIONS);
    let retired = a
        .engine
        .maintain(&mut a.owner, NOW + INVITATION_LIFETIME_SECS)
        .unwrap();
    assert_eq!(retired.retired_invitations, MAX_INVITATIONS);
    assert!(matches!(
        a.engine
            .handle(&mut a.owner, IdentityHash::new([2; 16]), &old, NOW),
        Err(EnrollmentError::Expired)
    ));
    assert!(a.engine.read().unwrap().invites.is_empty());
}

#[test]
fn enrollment_output_cleanup_preserves_service_facts_and_unowned_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::create(dir.path().join("db")).unwrap();
    let mut a = Device::new(&db, 1);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "ownership", NOW)
        .unwrap();
    let entry = a.engine.read().unwrap().entries.remove(0);
    // A service operation and its application fact occupy the same namespace.
    let operation = a
        .engine
        .adapter(&mut a.owner, &entry, None, |adapter, _| {
            let operation = Enrollment::next(adapter, &entry)?;
            let mut key = [0; 33];
            key[0] = b'o';
            key[1..].copy_from_slice(&operation.0);
            adapter
                .compose(
                    operation,
                    Command::Send {
                        group,
                        body: b"application",
                    },
                    |_, _, _| Ok::<_, EnrollmentError>(Admission::Reject(1)),
                    |tx, _| {
                        tx.put(&key, b"service recovery fact")
                            .map_err(|_| EnrollmentError::Corrupt)
                    },
                )
                .unwrap();
            Ok(operation)
        })
        .unwrap();
    assert_eq!(
        a.engine
            .maintain(&mut a.owner, NOW)
            .unwrap()
            .retired_outputs,
        1
    );
    a.engine
        .adapter(&mut a.owner, &entry, None, |adapter, snapshot| {
            assert_eq!(adapter.pending_operations().unwrap(), vec![operation]);
            // Prefix finalization discharged Create's receipt, but the service
            // operation and the later cleanup transaction remain behind its fence.
            assert_eq!(
                Enrollment::finalize_prefix(adapter, &entry, &[operation]).unwrap(),
                (2, Some(operation))
            );
            let output = adapter.retry(operation).unwrap().unwrap();
            assert_eq!(output.event, prns_groups_mls::Event::Sent);
            // Defense in depth: a missing service fact is not permission to erase
            // ciphertext. Neither Sent nor an unsupported control counts as proof.
            assert!(!Enrollment::output_replay_is_preserved(&entry, &snapshot, &output).unwrap());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        a.engine
            .maintain(&mut a.owner, NOW + 1)
            .unwrap()
            .retired_outputs,
        0
    );
}

#[test]
fn repeated_invitation_cycles_retire_more_than_the_api_receipt_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::create(dir.path().join("db")).unwrap();
    let mut a = Device::new(&db, 1);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "cycles", NOW)
        .unwrap();
    for cycle in 0..12 {
        let now = NOW + cycle * (RETRY_HORIZON_SECS + 1);
        for id in 2..9 {
            a.engine
                .invite(
                    &mut a.owner,
                    request_at(id, now),
                    group,
                    IdentityHash::new([id; 16]),
                    now,
                )
                .unwrap();
        }
        let catalog = a.engine.read().unwrap();
        assert_eq!(catalog.invites.len(), 7);
        assert!(catalog.requests.len() <= 8);
    }
    let now = NOW + 12 * (RETRY_HORIZON_SECS + 1);
    a.engine.maintain(&mut a.owner, now).unwrap();
    assert!(a.engine.read().unwrap().requests.is_empty());
    assert!(a.engine.read().unwrap().invites.is_empty());
    assert_eq!(a.engine.groups(&mut a.owner).unwrap(), vec![group]);
}

#[test]
fn unfinished_welcome_survives_retirement_and_finished_join_retries_after_catalog_pruning() {
    let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let dbs = [
        Database::create(dirs[0].path().join("a")).unwrap(),
        Database::create(dirs[1].path().join("b")).unwrap(),
    ];
    let (mut a, mut b) = (Device::new(&dbs[0], 1), Device::new(&dbs[1], 2));
    let group = a
        .engine
        .create(&mut a.owner, request(1), "late", NOW)
        .unwrap();
    let invitation = offer(&mut a, &mut b, group, 2);
    b.engine
        .respond(&mut b.owner, request(3), invitation, true, NOW)
        .unwrap();
    let join = b.work("join");
    let exact = a.receive(b.identity, &join, NOW);
    let later = NOW + RETRY_HORIZON_SECS + 1;
    a.engine.maintain(&mut a.owner, later).unwrap();
    b.engine.maintain(&mut b.owner, later).unwrap();
    assert_eq!(a.engine.read().unwrap().invites.len(), 1);
    assert_eq!(b.engine.read().unwrap().invites.len(), 1);
    assert_eq!(a.receive(b.identity, &join, later), exact);
    b.complete(&join, &exact, later);
    let joined = b.work("joined");
    let ack = a.receive(b.identity, &joined, later);
    b.complete(&joined, &ack, later);
    let prune = later + RETRY_HORIZON_SECS;
    a.engine.maintain(&mut a.owner, prune).unwrap();
    b.engine.maintain(&mut b.owner, prune).unwrap();
    assert!(a.engine.read().unwrap().invites.is_empty());
    assert!(b.engine.read().unwrap().invites.is_empty());
    assert_eq!(a.receive(b.identity, &join, prune), exact);
    assert_eq!(a.receive(b.identity, &joined, prune), ack);
    assert_eq!(b.descriptor(group).readers.len(), 2);
    // Enrollment outputs are redundant once Snapshot owns the exact retry data.
    for device in [&mut a, &mut b] {
        let entry = device.engine.read().unwrap().entries.remove(0);
        device
            .engine
            .adapter(&mut device.owner, &entry, None, |adapter, _| {
                assert!(adapter.pending_operations().unwrap().is_empty());
                Ok(())
            })
            .unwrap();
    }
}
