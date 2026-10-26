use super::*;

fn enroll(a: &mut Device<'_>, b: &mut Device<'_>, group: GroupId, token: u8) -> [u8; 32] {
    let id = offer(a, b, group, token);
    b.engine
        .respond(&mut b.owner, request(token.wrapping_add(1)), id, true, NOW)
        .unwrap();
    let join = b.work("join");
    let reply = a.receive(b.identity, &join, NOW);
    b.complete(&join, &reply, NOW);
    let joined = b.work("joined");
    let reply = a.receive(b.identity, &joined, NOW);
    b.complete(&joined, &reply, NOW);
    id
}
fn remove(a: &mut Device<'_>, b: &mut Device<'_>, group: GroupId, token: u8) {
    let member = b.descriptor(group).identity.binding();
    let head = a.descriptor(group).control.head.0;
    a.engine
        .remove_member(
            &mut a.owner,
            request(token),
            group,
            Member::new(b.identity, member).endpoint().unwrap(),
            head,
            NOW,
        )
        .unwrap();
    let sync = b.work("sync");
    let reply = a.receive(b.identity, &sync, NOW);
    b.complete(&sync, &reply, NOW);
}

#[test]
fn removal_is_owner_ordered_and_former_member_receives_terminal_proof() {
    let directory = tempfile::tempdir().unwrap();
    let dbs = ["a", "b", "c"].map(|name| Database::create(directory.path().join(name)).unwrap());
    let (mut a, mut b, mut c) = (
        Device::new(&dbs[0], 1),
        Device::new(&dbs[1], 2),
        Device::new(&dbs[2], 3),
    );
    let group = a
        .engine
        .create(&mut a.owner, request(1), "management", NOW)
        .unwrap();
    enroll(&mut a, &mut b, group, 2);
    enroll(&mut a, &mut c, group, 4);
    let old_head = a.descriptor(group).control.head.0;
    let bob = Member::new(b.identity, b.descriptor(group).identity.binding())
        .endpoint()
        .unwrap();
    assert!(matches!(
        b.engine
            .remove_member(&mut b.owner, request(40), group, bob, old_head, NOW),
        Err(EnrollmentError::NotOwner)
    ));
    assert!(matches!(
        a.engine
            .remove_member(&mut a.owner, request(41), group, bob, [0; 32], NOW),
        Err(EnrollmentError::StaleMembership)
    ));
    remove(&mut a, &mut b, group, 42);
    assert_eq!(b.descriptor(group).lifecycle, GroupLifecycle::Removed);
    assert_eq!(a.descriptor(group).readers.len(), 2);
    let sync = c.work("sync");
    let reply = a.receive(c.identity, &sync, NOW);
    c.complete(&sync, &reply, NOW);
    assert_eq!(c.descriptor(group).readers.len(), 2);
    assert_eq!(c.descriptor(group).control, a.descriptor(group).control);
    a.engine
        .remove_member(&mut a.owner, request(42), group, bob, old_head, NOW)
        .unwrap();
    assert_eq!(a.descriptor(group).readers.len(), 2);
}

#[test]
fn authenticated_leave_cannot_substitute_another_members_endpoint() {
    let directory = tempfile::tempdir().unwrap();
    let dbs = ["a", "b", "c"].map(|name| Database::create(directory.path().join(name)).unwrap());
    let (mut a, mut b, mut c) = (
        Device::new(&dbs[0], 1),
        Device::new(&dbs[1], 2),
        Device::new(&dbs[2], 3),
    );
    let group = a
        .engine
        .create(&mut a.owner, request(1), "leave auth", NOW)
        .unwrap();
    enroll(&mut a, &mut b, group, 2);
    enroll(&mut a, &mut c, group, 4);
    let ac = a.engine.read().unwrap();
    let bc = b.engine.read().unwrap();
    let cc = c.engine.read().unwrap();
    let mut target = cc.entries[0].local.clone();
    target.rns = b.identity.into_bytes();
    let departure = Departure {
        id: [9; 32],
        group: group.0,
        owner: ac.entries[0].owner.clone(),
        member: target,
        known_head: a.descriptor(group).control.head.0,
    };
    assert!(matches!(
        a.engine.handle(
            &mut a.owner,
            b.identity,
            &frame(&Frame::Leave(departure)).unwrap(),
            NOW
        ),
        Err(EnrollmentError::Authentication)
    ));
    assert_eq!(a.descriptor(group).readers.len(), 3);
    assert_eq!(bc.entries[0].local.rns, b.identity.into_bytes());
}

#[test]
fn leave_outbox_survives_local_delete_restart_and_lost_owner_reply() {
    let directory = tempfile::tempdir().unwrap();
    let ap = directory.path().join("a");
    let bp = directory.path().join("b");
    let adb = Database::create(&ap).unwrap();
    let bdb = Database::create(&bp).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "leave then delete", NOW)
        .unwrap();
    enroll(&mut a, &mut b, group, 2);
    b.engine
        .leave(&mut b.owner, request(40), group, NOW)
        .unwrap();
    assert_eq!(b.descriptor(group).lifecycle, GroupLifecycle::Leaving);
    let leave = b.work("leave");
    b.engine
        .delete_local(&mut b.owner, request(41), group, NOW)
        .unwrap();
    assert!(b.engine.groups(&mut b.owner).unwrap().is_empty());
    assert!(b.engine.read().unwrap().retired.iter().all(|r| r.purged));
    drop(b);
    drop(bdb);
    let bdb = Database::open(&bp).unwrap();
    let mut b = Device::new(&bdb, 2);
    let retry = b.work("leave");
    assert_eq!(retry.frame, leave.frame);
    let lost = a.receive(b.identity, &retry, NOW);
    assert_eq!(a.descriptor(group).readers.len(), 1);
    let repeated = a.receive(b.identity, &retry, NOW);
    assert_eq!(lost, repeated);
    b.complete(&retry, &repeated, NOW);
    assert!(b.engine.read().unwrap().departures.is_empty());
    assert!(b.engine.groups(&mut b.owner).unwrap().is_empty());
    // A deliberate new invitation creates a fresh membership; the old Leave
    // remains a terminal query and cannot remove the new incarnation.
    enroll(&mut a, &mut b, group, 50);
    let new_member = b.descriptor(group).identity.binding();
    assert_ne!(
        new_member.endpoint,
        decode::<Frame>(&leave.frame)
            .ok()
            .and_then(|f| if let Frame::Leave(d) = f {
                Some(d.member.endpoint)
            } else {
                None
            })
            .unwrap()
    );
    let old = a.receive(b.identity, &leave, NOW);
    assert_eq!(old, lost);
    assert_eq!(a.descriptor(group).readers.len(), 2);
}

#[test]
fn cancellation_suppresses_late_offer_and_allows_fresh_invitation() {
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "cancel", NOW)
        .unwrap();
    let id = a
        .engine
        .invite(&mut a.owner, request(2), group, b.identity, NOW)
        .unwrap();
    let delayed = a.work("offer");
    a.engine
        .cancel_invitation(&mut a.owner, request(3), id, NOW)
        .unwrap();
    let cancel = a.work("cancel");
    let ack = b.receive(a.identity, &cancel, NOW);
    a.complete(&cancel, &ack, NOW);
    b.receive(a.identity, &delayed, NOW);
    assert!(b.engine.invitations(&mut b.owner, NOW).unwrap().is_empty());
    let fresh = offer(&mut a, &mut b, group, 4);
    b.engine
        .respond(&mut b.owner, request(5), fresh, true, NOW)
        .unwrap();
    let pending = b.work("join");
    a.engine
        .cancel_invitation(&mut a.owner, request(6), fresh, NOW)
        .unwrap();
    let reply = a.receive(b.identity, &pending, NOW);
    assert!(matches!(decode::<Reply>(&reply).unwrap(), Reply::Cancelled));
    b.complete(&pending, &reply, NOW);
    enroll(&mut a, &mut b, group, 7);
    assert_eq!(b.descriptor(group).lifecycle, GroupLifecycle::Active);
    let joined = a
        .engine
        .invitations(&mut a.owner, NOW)
        .unwrap()
        .into_iter()
        .find(|i| i.state == InvitationState::Joined)
        .unwrap();
    assert!(!joined.can_cancel);
    assert!(joined.can_dismiss);
    assert!(matches!(
        a.engine
            .cancel_invitation(&mut a.owner, request(20), joined.id, NOW),
        Err(EnrollmentError::AlreadyJoined)
    ));
}

#[test]
fn owner_close_is_terminal_and_notification_survives_local_purge() {
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "close", NOW)
        .unwrap();
    enroll(&mut a, &mut b, group, 2);
    assert!(matches!(
        a.engine.delete_local(&mut a.owner, request(40), group, NOW),
        Err(EnrollmentError::OwnerMustClose)
    ));
    let head = a.descriptor(group).control.head.0;
    a.engine
        .close(&mut a.owner, request(41), group, head, NOW)
        .unwrap();
    assert_eq!(a.descriptor(group).lifecycle, GroupLifecycle::Closed);
    a.engine
        .delete_local(&mut a.owner, request(42), group, NOW)
        .unwrap();
    let terminal = a.work("terminal");
    let ack = b.receive(a.identity, &terminal, NOW);
    a.complete(&terminal, &ack, NOW);
    assert_eq!(b.descriptor(group).lifecycle, GroupLifecycle::Closed);
    b.engine
        .delete_local(&mut b.owner, request(43), group, NOW)
        .unwrap();
    assert!(b
        .engine
        .read()
        .unwrap()
        .closed_groups
        .iter()
        .any(|p| p.statement.group == group.0));
    assert!(a.engine.groups(&mut a.owner).unwrap().is_empty());
    assert!(a.engine.read().unwrap().entries.is_empty());
    // Retained Create replay reports its prior result without recreating state.
    assert_eq!(
        a.engine
            .create(&mut a.owner, request(1), "close", NOW)
            .unwrap(),
        group
    );
    assert!(a.engine.groups(&mut a.owner).unwrap().is_empty());
}

#[test]
fn repeated_readmission_preserves_old_namespaces_and_passes_seven_controls() {
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "churn", NOW)
        .unwrap();
    for i in 0..6u8 {
        enroll(&mut a, &mut b, group, 2 + i * 3);
        remove(&mut a, &mut b, group, 4 + i * 3);
    }
    let catalog = a.engine.read().unwrap();
    let controls = a
        .engine
        .adapter(&mut a.owner, &catalog.entries[0], None, |_, s| {
            Ok(s.controls.len())
        })
        .unwrap();
    assert_eq!(controls, 12);
    assert_eq!(b.engine.read().unwrap().archives.len(), 5);
    assert_eq!(a.descriptor(group).readers.len(), 1);
}

#[test]
fn ordinary_catalog_capacity_preserves_remove_close_and_delete_requests() {
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "reserved", NOW)
        .unwrap();
    enroll(&mut a, &mut b, group, 2);
    let mut catalog = a.engine.read().unwrap();
    while catalog.requests.len() < ORDINARY_REQUESTS {
        let n = catalog.requests.len() as u8 + 50;
        catalog.requests.push(Request {
            id: request(n),
            fingerprint: [n; 32],
            result: group.0,
        });
    }
    a.engine.write(&catalog).unwrap();
    assert!(matches!(
        a.engine.invite(
            &mut a.owner,
            request(200),
            group,
            IdentityHash::new([3; 16]),
            NOW
        ),
        Err(EnrollmentError::Capacity)
    ));
    remove(&mut a, &mut b, group, 201);
    let head = a.descriptor(group).control.head.0;
    a.engine
        .close(&mut a.owner, request(202), group, head, NOW)
        .unwrap();
    a.engine
        .delete_local(&mut a.owner, request(203), group, NOW)
        .unwrap();
    assert!(a.engine.groups(&mut a.owner).unwrap().is_empty());
    assert_eq!(
        a.engine.read().unwrap().requests.len(),
        ORDINARY_REQUESTS + 3
    );
}

#[test]
fn unsolicited_cancellation_quota_cannot_consume_known_invitation_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let attacker = IdentityHash::new([9; 16]);
    for n in 0..MAX_INCOMING_PER_PEER {
        let frame = frame(&Frame::Cancel {
            invitation: [n as u8; 32],
            group: [42; 32],
            expires: NOW + INVITATION_LIFETIME_SECS,
        })
        .unwrap();
        b.engine
            .handle(&mut b.owner, attacker, &frame, NOW)
            .unwrap();
    }
    let frame = frame(&Frame::Cancel {
        invitation: [88; 32],
        group: [42; 32],
        expires: NOW + INVITATION_LIFETIME_SECS,
    })
    .unwrap();
    assert!(matches!(
        b.engine.handle(&mut b.owner, attacker, &frame, NOW),
        Err(EnrollmentError::Capacity)
    ));
    let group = a
        .engine
        .create(&mut a.owner, request(1), "real", NOW)
        .unwrap();
    let id = offer(&mut a, &mut b, group, 2);
    a.engine
        .cancel_invitation(&mut a.owner, request(3), id, NOW)
        .unwrap();
    let cancel = a.work("cancel");
    b.receive(a.identity, &cancel, NOW);
    assert_eq!(
        b.engine.invitations(&mut b.owner, NOW).unwrap()[0].state,
        InvitationState::Cancelled
    );
}

#[test]
fn close_reaches_a_pending_acceptance_after_owner_purge() {
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let mut a = Device::new(&adb, 1);
    let mut b = Device::new(&bdb, 2);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "pending close", NOW)
        .unwrap();
    let id = offer(&mut a, &mut b, group, 2);
    b.engine
        .respond(&mut b.owner, request(3), id, true, NOW)
        .unwrap();
    let join = b.work("join");
    let head = a.descriptor(group).control.head.0;
    a.engine
        .close(&mut a.owner, request(4), group, head, NOW)
        .unwrap();
    a.engine
        .delete_local(&mut a.owner, request(5), group, NOW)
        .unwrap();
    let response = a.receive(b.identity, &join, NOW);
    assert!(matches!(
        decode::<Reply>(&response).unwrap(),
        Reply::Terminal(_)
    ));
    b.complete(&join, &response, NOW);
    assert!(b.engine.groups(&mut b.owner).unwrap().is_empty());
    assert_eq!(
        b.engine.invitations(&mut b.owner, NOW).unwrap()[0].state,
        InvitationState::Cancelled
    );
}

#[test]
fn owner_closed_pending_invitation_is_dismissible_and_no_longer_offered() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::create(directory.path().join("owner")).unwrap();
    let mut a = Device::new(&db, 1);
    let group = a
        .engine
        .create(&mut a.owner, request(1), "close offer", NOW)
        .unwrap();
    let id = a
        .engine
        .invite(
            &mut a.owner,
            request(2),
            group,
            IdentityHash::new([2; 16]),
            NOW,
        )
        .unwrap();
    let head = a.descriptor(group).control.head.0;
    a.engine
        .close(&mut a.owner, request(3), group, head, NOW)
        .unwrap();
    let invites = a.engine.invitations(&mut a.owner, NOW).unwrap();
    assert_eq!(invites[0].state, InvitationState::Cancelled);
    assert!(!invites[0].can_cancel);
    assert!(invites[0].can_dismiss);
    assert!(!a
        .engine
        .pending(&mut a.owner, NOW)
        .unwrap()
        .iter()
        .any(|w| matches!(decode::<Frame>(&w.frame).unwrap(), Frame::Offer(_))));
    a.engine
        .dismiss_invitation(&mut a.owner, request(4), id, NOW)
        .unwrap();
    assert!(a.engine.invitations(&mut a.owner, NOW).unwrap().is_empty());
}

fn managed_command(
    database: &Database,
    identity: IdentityHash,
    group: GroupId,
    control: Option<prns_groups::ControlReference>,
    command: crate::client::Command,
) -> Result<crate::client::Reply, crate::Error> {
    let mut manager = crate::GroupManager::open(database, identity, None).unwrap();
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    manager.execute(crate::Job {
        command,
        group: Some(group),
        control,
        reply,
        wake: std::sync::Arc::new(tokio::sync::Notify::new()),
    });
    receiver.try_recv().unwrap()
}

#[test]
fn committed_send_replays_from_terminal_and_archived_membership_without_republishing() {
    use crate::client::{Command, Reply};
    let directory = tempfile::tempdir().unwrap();
    let adb = Database::create(directory.path().join("a")).unwrap();
    let bdb = Database::create(directory.path().join("b")).unwrap();
    let (mut a, mut b) = (Device::new(&adb, 1), Device::new(&bdb, 2));
    let group = a
        .engine
        .create(&mut a.owner, request(1), "history retry", NOW)
        .unwrap();
    let invitation = enroll(&mut a, &mut b, group, 2);
    let prior = b.descriptor(group);
    let clock = prns_groups::ClockReading::Trusted(
        prns_groups::ClockInterval::new(Timestamp(NOW), Timestamp(NOW)).unwrap(),
    );
    let request = crate::SendText {
        group,
        client_request_id: request(30),
        text: "Retained result".into(),
        clock,
    };
    let Reply::Sent(original) = managed_command(
        &bdb,
        b.identity,
        group,
        Some(prior.control),
        Command::SendText(request.clone()),
    )
    .unwrap() else {
        panic!("missing publication result")
    };
    remove(&mut a, &mut b, group, 40);
    assert!(
        matches!(managed_command(&bdb, b.identity, group, Some(prior.control),
        Command::SendText(request.clone())).unwrap(), Reply::Sent(id) if id == original)
    );
    b.engine
        .dismiss_invitation(&mut b.owner, super::request(41), invitation, NOW)
        .unwrap();
    assert_eq!(
        crate::GroupManager::open(&bdb, b.identity, None)
            .unwrap()
            .invitation_group(invitation)
            .unwrap(),
        Some(group)
    );
    enroll(&mut a, &mut b, group, 42);
    let current = b.descriptor(group);
    assert_ne!(prior.identity.binding(), current.identity.binding());
    assert!(matches!(managed_command(&bdb, b.identity, group, None,
        Command::SendText(request.clone())).unwrap(), Reply::Sent(id) if id == original));
    let mut changed = request.clone();
    changed.text = "Changed immutable intent".into();
    assert!(matches!(
        managed_command(&bdb, b.identity, group, None, Command::SendText(changed)),
        Err(crate::Error::IdempotencyConflict)
    ));
    let mut unsent = request;
    unsent.client_request_id = super::request(31);
    assert!(matches!(
        managed_command(
            &bdb,
            b.identity,
            group,
            Some(prior.control),
            Command::SendText(unsent.clone())
        ),
        Err(crate::Error::Configuration)
    ));
    let Reply::Messages(history) = managed_command(
        &bdb,
        b.identity,
        group,
        None,
        Command::Messages {
            group,
            before_sequence: None,
            limit: 100,
            clock,
        },
    )
    .unwrap() else {
        panic!("missing history")
    };
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].id, original);
    assert!(
        matches!(managed_command(&bdb, b.identity, group, Some(current.control),
        Command::SendText(unsent)).unwrap(), Reply::Sent(id) if id != original)
    );
    b.engine
        .delete_local(&mut b.owner, super::request(49), group, NOW)
        .unwrap();
    assert_eq!(
        crate::GroupManager::open(&bdb, b.identity, None)
            .unwrap()
            .invitation_group(invitation)
            .unwrap(),
        Some(group)
    );
}
