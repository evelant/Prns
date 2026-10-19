//! Generic and text consumers share the same durable owner and carrier.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
#[path = "../examples/support/telemetry_projection.rs"]
mod telemetry_projection;
use prns_groups::{NamespaceId, SchemaId};
use prns_groups_runner_native::{fixture, Client, Error, Job, PublishRecord, StoreSession, Submit};
use prns_groups_service::{ChangeKind, ClockReading, Timestamp};
use prns_host::IdentityHash;
use std::sync::{Arc, Mutex};
use support::{clock, Worker, NOW};
use telemetry_projection::{Projection, Reading, Refresh};

fn request(group: prns_groups::GroupId, targets: Vec<prns_groups::Endpoint>) -> PublishRecord {
    PublishRecord {
        group,
        client_request_id: prns_groups::retry::RequestToken::new(NOW, [19; 20])
            .unwrap()
            .into_bytes(),
        namespace: NamespaceId(0x74656c65),
        schema: SchemaId(1),
        targets,
        // Application-defined sensor ID + temperature in centi-degrees.
        body: [3u32.to_be_bytes(), 2150i32.to_be_bytes()].concat(),
        clock: clock(),
    }
}
struct Captured(Mutex<Option<Job>>);
impl Submit for Captured {
    fn submit(&self, job: Job) -> Result<(), Error> {
        let mut pending = self.0.lock().unwrap();
        if pending.is_some() {
            return Err(Error::Busy);
        }
        *pending = Some(job);
        Ok(())
    }
}

fn expired_clock() -> ClockReading {
    let later = Timestamp(NOW + 15 * 86400);
    ClockReading::Trusted(prns_groups::ClockInterval::new(later, later).unwrap())
}

#[tokio::test]
async fn caught_up_projection_rebuilds_after_lost_maintenance_result_and_reopen() {
    let identities = [IdentityHash::new([31; 16]), IdentityHash::new([32; 16])];
    let fixtures = fixture::generate("Projection recovery", &identities, Timestamp(NOW)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.redb");
    let worker = Worker::open(path.clone(), fixtures[0].clone(), identities[0]);
    let cfg = worker.client.network_config().await.unwrap();
    let group = cfg.group;
    let request = request(group, cfg.peers.iter().map(|peer| peer.endpoint).collect());
    let id = worker.client.publish(request).await.unwrap();
    let mut projection = Projection::new(256).unwrap();
    assert_eq!(
        projection
            .refresh(&worker.client, group, clock())
            .await
            .unwrap(),
        Refresh::Current
    );
    assert_eq!(projection.readings.len(), 1);
    let caught_up = projection.cursor.unwrap();
    assert_eq!(
        caught_up,
        worker.client.changes(group, None, 10).await.unwrap().head
    );
    worker.stop();

    // The admitted maintenance owns its transaction even when the waiting
    // consumer disappears. Reopen the actual database before observing results.
    let database = redb::Database::create(&path).unwrap();
    let cfg = fixture::load_and_install(&database, &fixtures[0], identities[0]).unwrap();
    let mut session = StoreSession::open(&database, cfg).unwrap();
    let lane = Arc::new(Captured(Mutex::new(None)));
    let client = Client::new(lane.clone());
    let mut waiter = Box::pin(client.maintain(expired_clock()));
    assert!(futures_util::poll!(waiter.as_mut()).is_pending());
    drop(waiter);
    lane.0.lock().unwrap().take().unwrap().execute(&mut session);
    drop(session);
    drop(database);

    let worker = Worker::open(path, fixtures[0].clone(), identities[0]);
    let changes = worker
        .client
        .changes(group, Some(caught_up), 10)
        .await
        .unwrap();
    assert!(changes.changes.is_empty());
    assert!(
        changes.history_pruned,
        "a caught-up consumer must learn of deletion"
    );
    assert!(changes.head.position > caught_up.position);
    assert_eq!(changes.next, changes.head);
    assert!(worker
        .client
        .record(group, id, expired_clock())
        .await
        .unwrap()
        .is_none());
    assert!(worker
        .client
        .delivery_progress(group, id, expired_clock())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        projection
            .refresh(&worker.client, group, expired_clock())
            .await
            .unwrap(),
        Refresh::Current
    );
    assert!(
        projection.readings.is_empty(),
        "rebuild replaces the old projection"
    );
    assert_eq!(projection.cursor, Some(changes.head));
    let rebuilt = projection.clone();
    assert_eq!(
        projection
            .refresh(&worker.client, group, expired_clock())
            .await
            .unwrap(),
        Refresh::Current
    );
    assert_eq!(projection, rebuilt);
    worker.stop();
}

#[tokio::test]
async fn bounded_projection_keeps_rows_and_cursor_on_incomplete_refresh_or_wrong_store() {
    let identities = [IdentityHash::new([33; 16]), IdentityHash::new([34; 16])];
    let fixtures = fixture::generate("Projection bounds", &identities, Timestamp(NOW)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        directory.path().join("local.redb"),
        fixtures[0].clone(),
        identities[0],
    );
    let other = Worker::open(
        directory.path().join("peer.redb"),
        fixtures[1].clone(),
        identities[1],
    );
    let cfg = worker.client.network_config().await.unwrap();
    let group = cfg.group;
    let mut request = request(group, cfg.peers.iter().map(|peer| peer.endpoint).collect());
    worker.client.publish(request.clone()).await.unwrap();
    let mut projection = Projection::new(1).unwrap();
    assert_eq!(
        projection
            .refresh(&worker.client, group, clock())
            .await
            .unwrap(),
        Refresh::Current
    );
    let saved = projection.clone();
    // An actual other endpoint's cursor scope, not only a fabricated token.
    assert!(matches!(
        projection.refresh(&other.client, group, clock()).await,
        Err(Error::InvalidInput)
    ));
    assert_eq!(projection, saved);

    request.client_request_id = prns_groups::retry::RequestToken::new(NOW, [23; 20])
        .unwrap()
        .into_bytes();
    request.body = Reading {
        sensor: 4,
        centi_celsius: 2275,
    }
    .encode()
    .to_vec();
    worker.client.publish(request).await.unwrap();
    assert_eq!(
        projection
            .refresh(&worker.client, group, clock())
            .await
            .unwrap(),
        Refresh::Incomplete
    );
    assert_eq!(
        projection, saved,
        "a partial rebuild must not advance the durable cursor"
    );
    let mut larger = Projection::new(2).unwrap();
    assert_eq!(
        larger
            .refresh(&worker.client, group, clock())
            .await
            .unwrap(),
        Refresh::Current
    );
    assert_eq!(larger.readings.len(), 2);
    worker.stop();
    other.stop();
}

#[tokio::test]
async fn projection_replays_insertions_and_restarts_after_retirement_during_paging() {
    // Interleave writes at a deterministic query boundary using the real native
    // owner and store. The third query follows the first record page and is the
    // replay from the pre-page anchor; neither race relies on timing or sleeps.
    for retire in [false, true] {
        let identities = [IdentityHash::new([35; 16]), IdentityHash::new([36; 16])];
        let fixtures =
            fixture::generate("Projection paging race", &identities, Timestamp(NOW)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let database = redb::Database::create(directory.path().join("race.redb")).unwrap();
        let cfg = fixture::load_and_install(&database, &fixtures[0], identities[0]).unwrap();
        let group = cfg.group;
        let mut request = request(group, cfg.peers.iter().map(|peer| peer.endpoint).collect());
        let mut session = StoreSession::open(&database, cfg).unwrap();
        let lane = Arc::new(Captured(Mutex::new(None)));
        let client = Client::new(lane.clone());
        let first = client.admit_publish(request.clone()).unwrap();
        lane.0.lock().unwrap().take().unwrap().execute(&mut session);
        let first = first.await.unwrap();
        let mut projection = Projection::new(256).unwrap();
        let mut refresh = Box::pin(projection.refresh(&client, group, clock()));
        let mut queries = 0;
        let result = loop {
            if let std::task::Poll::Ready(result) = futures_util::poll!(refresh.as_mut()) {
                break result.unwrap();
            }
            queries += 1;
            assert!(queries < 20, "projection refresh must be bounded");
            let query = lane.0.lock().unwrap().take().unwrap();
            if queries == 3 {
                let writer = Arc::new(Captured(Mutex::new(None)));
                let writer_client = Client::new(writer.clone());
                if retire {
                    let mut maintenance = Box::pin(writer_client.maintain(expired_clock()));
                    assert!(futures_util::poll!(maintenance.as_mut()).is_pending());
                    writer
                        .0
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .execute(&mut session);
                    maintenance.await.unwrap();
                } else {
                    request.client_request_id =
                        prns_groups::retry::RequestToken::new(NOW, [24; 20])
                            .unwrap()
                            .into_bytes();
                    request.body = Reading {
                        sensor: 4,
                        centi_celsius: 2275,
                    }
                    .encode()
                    .to_vec();
                    let second = writer_client.admit_publish(request.clone()).unwrap();
                    writer
                        .0
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .execute(&mut session);
                    second.await.unwrap();
                }
            }
            query.execute(&mut session);
        };
        drop(refresh);
        assert_eq!(result, Refresh::Current);
        if retire {
            assert!(
                projection.readings.is_empty(),
                "invalidation must discard the stale page"
            );
            assert!(queries >= 6, "retirement must restart the paged rebuild");
        } else {
            assert_eq!(
                projection.readings.len(),
                2,
                "pre-page anchor must recover concurrent insertion"
            );
            assert!(projection.readings.contains_key(&first));
        }
    }
}

#[tokio::test]
async fn projection_reports_incomplete_when_retirement_repeatedly_overtakes_rebuild() {
    let identities = [IdentityHash::new([37; 16]), IdentityHash::new([38; 16])];
    let fixtures = fixture::generate("Projection churn", &identities, Timestamp(NOW)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let database = redb::Database::create(directory.path().join("churn.redb")).unwrap();
    let cfg = fixture::load_and_install(&database, &fixtures[0], identities[0]).unwrap();
    let group = cfg.group;
    let mut request = request(group, cfg.peers.iter().map(|peer| peer.endpoint).collect());
    let mut session = StoreSession::open(&database, cfg).unwrap();
    let lane = Arc::new(Captured(Mutex::new(None)));
    let client = Client::new(lane.clone());
    let first = client.admit_publish(request.clone()).unwrap();
    lane.0.lock().unwrap().take().unwrap().execute(&mut session);
    first.await.unwrap();
    let mut projection = Projection::new(256).unwrap();
    let saved = projection.clone();
    let mut refresh = Box::pin(projection.refresh(&client, group, clock()));
    let mut queries = 0;
    let result = loop {
        if let std::task::Poll::Ready(result) = futures_util::poll!(refresh.as_mut()) {
            break result.unwrap();
        }
        queries += 1;
        assert!(
            queries <= 9,
            "three rebuild attempts must exhaust the retry budget"
        );
        let query = lane.0.lock().unwrap().take().unwrap();
        if queries % 3 == 0 {
            let writer = Arc::new(Captured(Mutex::new(None)));
            let writer_client = Client::new(writer.clone());
            let cycle = queries / 3;
            let now = NOW + (cycle - 1) as u64 * 15 * 86400;
            if queries > 3 {
                request.client_request_id =
                    prns_groups::retry::RequestToken::new(now, [queries as u8; 20])
                        .unwrap()
                        .into_bytes();
                request.clock = ClockReading::Trusted(
                    prns_groups::ClockInterval::new(Timestamp(now), Timestamp(now)).unwrap(),
                );
                let publication = writer_client.admit_publish(request.clone()).unwrap();
                writer
                    .0
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .execute(&mut session);
                publication.await.unwrap();
            }
            let expired = Timestamp(now + 15 * 86400);
            let expired =
                ClockReading::Trusted(prns_groups::ClockInterval::new(expired, expired).unwrap());
            let mut maintenance = Box::pin(writer_client.maintain(expired));
            assert!(futures_util::poll!(maintenance.as_mut()).is_pending());
            writer
                .0
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .execute(&mut session);
            maintenance.await.unwrap();
        }
        query.execute(&mut session);
    };
    drop(refresh);
    assert_eq!(result, Refresh::Incomplete);
    assert_eq!(queries, 9);
    assert_eq!(
        projection, saved,
        "a repeatedly invalidated rebuild must not publish partial state"
    );
}

#[tokio::test]
async fn lost_generic_reply_reopens_exact_record_and_scoped_change_cursor() {
    let identities = [IdentityHash::new([11; 16]), IdentityHash::new([22; 16])];
    let fixtures = fixture::generate("Generic recovery", &identities, Timestamp(NOW)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("generic.redb");
    let database = redb::Database::create(&path).unwrap();
    let cfg = fixture::load_and_install(&database, &fixtures[0], identities[0]).unwrap();
    let group = cfg.group;
    let request = request(group, cfg.peers.iter().map(|p| p.endpoint).collect());
    let mut session = StoreSession::open(&database, cfg).unwrap();
    let lane = Arc::new(Captured(Mutex::new(None)));
    let client = Client::new(lane.clone());
    let mut oversized = request.clone();
    oversized.body = vec![0; prns_groups_runner_native::MAX_PUBLICATION_BODY_BYTES + 1];
    assert!(matches!(
        client.admit_publish(oversized),
        Err(Error::InvalidInput)
    ));
    let mut duplicate = request.clone();
    duplicate.targets.push(duplicate.targets[0]);
    assert!(matches!(
        client.admit_publish(duplicate),
        Err(Error::InvalidInput)
    ));
    let mut too_many = request.clone();
    too_many.targets =
        vec![too_many.targets[0]; prns_groups_runner_native::MAX_PUBLICATION_TARGETS + 1];
    assert!(matches!(
        client.admit_publish(too_many),
        Err(Error::InvalidInput)
    ));
    let mut unsorted = request.clone();
    let mut another = unsorted.targets[0];
    another.id.0[0] ^= 1;
    unsorted.targets.push(another);
    unsorted.targets.sort();
    unsorted.targets.reverse();
    assert!(matches!(
        client.admit_publish(unsorted),
        Err(Error::InvalidInput)
    ));
    assert!(
        lane.0.lock().unwrap().is_none(),
        "invalid inputs must not occupy the lane"
    );
    let waiter = client.admit_publish(request.clone()).unwrap();
    drop(waiter);
    lane.0.lock().unwrap().take().unwrap().execute(&mut session);
    drop(session);
    drop(database);
    let worker = Worker::open(path, fixtures[0].clone(), identities[0]);
    let page = worker
        .client
        .records(group, None, 10, clock())
        .await
        .unwrap();
    assert_eq!(page.records.len(), 1);
    let id = page.records[0].record.metadata.record;
    assert_eq!(page.records[0].record.body.as_slice(), request.body);
    assert_eq!(page.records[0].progress.summary.pending, 1);
    assert!(worker
        .client
        .messages(group, None, 10, clock())
        .await
        .unwrap()
        .messages
        .is_empty());
    let mut replay = request.clone();
    replay.clock = ClockReading::Unknown;
    assert_eq!(worker.client.publish(replay).await.unwrap(), id);
    for field in 0..4 {
        let mut changed = request.clone();
        match field {
            0 => changed.body.push(1),
            1 => changed.namespace.0 += 1,
            2 => changed.schema.0 += 1,
            _ => changed.targets[0].id.0[0] ^= 1,
        }
        assert!(matches!(
            worker.client.publish(changed).await,
            Err(Error::IdempotencyConflict)
        ));
    }
    let changes = worker.client.changes(group, None, 10).await.unwrap();
    assert_eq!(
        changes
            .changes
            .iter()
            .filter(|c| matches!(c.kind, ChangeKind::Published(_)))
            .count(),
        1
    );
    assert!(!changes.history_pruned);
    assert!(worker
        .client
        .changes(group, Some(changes.next), 10)
        .await
        .unwrap()
        .changes
        .is_empty());
    let mut wrong = changes.next;
    wrong.group.0[0] ^= 1;
    assert!(matches!(
        worker.client.changes(group, Some(wrong), 10).await,
        Err(Error::InvalidInput)
    ));
    let mut wrong = changes.next;
    wrong.incarnation = prns_groups_store::StoreIncarnation::new([99; 24]).unwrap();
    assert!(matches!(
        worker.client.changes(group, Some(wrong), 10).await,
        Err(Error::InvalidInput)
    ));
    assert!(
        worker
            .client
            .delivery_progress(group, id, clock())
            .await
            .unwrap()
            .unwrap()
            .summary
            .pending
            == 1
    );
    assert_eq!(
        worker
            .client
            .record(group, id, clock())
            .await
            .unwrap()
            .unwrap()
            .record
            .metadata
            .record,
        id
    );
    let later = prns_groups::Timestamp(NOW + 15 * 86400);
    let later = ClockReading::Trusted(prns_groups::ClockInterval::new(later, later).unwrap());
    for _ in 0..4 {
        worker.client.maintain(later).await.unwrap();
    }
    assert!(worker
        .client
        .record(group, id, later)
        .await
        .unwrap()
        .is_none());
    assert!(
        worker
            .client
            .changes(group, None, 10)
            .await
            .unwrap()
            .history_pruned
    );
    worker.stop();
}

#[tokio::test]
async fn full_change_page_reports_trailing_retirement_on_the_next_page() {
    let identities = [IdentityHash::new([31; 16]), IdentityHash::new([32; 16])];
    let fixtures = fixture::generate("Feed retention", &identities, Timestamp(NOW)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        directory.path().join("feed.redb"),
        fixtures[0].clone(),
        identities[0],
    );
    let cfg = worker.client.network_config().await.unwrap();
    let at = |seconds| {
        let time = Timestamp(seconds);
        ClockReading::Trusted(prns_groups::ClockInterval::new(time, time).unwrap())
    };
    let mut later = request(cfg.group, cfg.peers.iter().map(|p| p.endpoint).collect());
    later.clock = at(NOW + 3600);
    later.client_request_id = prns_groups_runner_native::RequestToken::new(NOW + 3600, [51; 20])
        .unwrap()
        .into_bytes();
    let retained = worker.client.publish(later.clone()).await.unwrap();
    let old = request(cfg.group, later.targets.clone());
    let expired = worker.client.publish(old).await.unwrap();
    let floor = at(NOW + 14 * 86400 + 1);
    for _ in 0..4 {
        worker.client.maintain(floor).await.unwrap();
    }
    assert!(worker
        .client
        .record(cfg.group, expired, floor)
        .await
        .unwrap()
        .is_none());
    assert!(worker
        .client
        .record(cfg.group, retained, floor)
        .await
        .unwrap()
        .is_some());
    let first = worker.client.changes(cfg.group, None, 1).await.unwrap();
    assert_eq!(first.changes.len(), 1);
    assert!(!first.history_pruned);
    assert!(first.next.position < first.head.position);
    let tail = worker
        .client
        .changes(cfg.group, Some(first.next), 1)
        .await
        .unwrap();
    assert!(tail.changes.is_empty());
    assert!(tail.history_pruned);
    assert_eq!(tail.next, tail.head);
    worker.stop();
}
