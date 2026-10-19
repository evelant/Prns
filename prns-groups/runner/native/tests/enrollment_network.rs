//! Public runner APIs over fresh independent stores and real Reticulum Links.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[allow(dead_code)]
#[path = "../../../transport/reticulum/tests/support/wire.rs"]
mod wire;

use prns_groups_runner_native::{
    run_multi, Client, Clock, EnrollmentRequest, EnrollmentResult, Error, GroupManager, Job,
    SendText, Submit, SystemClock,
};
use prns_groups_service::{Endpoint, EndpointId, EndpointIncarnation, GroupId};
use prns_host::IdentityHash;
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

fn endpoint(peer: usize) -> Endpoint {
    Endpoint {
        id: EndpointId([(peer + 1) as u8; 32]),
        incarnation: EndpointIncarnation::new(1).unwrap(),
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

enum Work {
    Job(Box<Job>),
    Reopen(mpsc::SyncSender<()>),
    Stop,
}
struct Lane(mpsc::SyncSender<Work>);
impl Submit for Lane {
    fn submit(&self, job: Job) -> Result<(), Error> {
        self.0
            .try_send(Work::Job(Box::new(job)))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Error::Busy,
                mpsc::TrySendError::Disconnected(_) => Error::Unavailable,
            })
    }
}
struct Worker {
    client: Client,
    sender: mpsc::SyncSender<Work>,
    task: thread::JoinHandle<()>,
}
impl Worker {
    fn open(path: PathBuf, identity: IdentityHash) -> Self {
        let (sender, receiver) = mpsc::sync_channel(32);
        let (ready, readiness) = mpsc::sync_channel(1);
        let client = Client::new(Arc::new(Lane(sender.clone())));
        let task = thread::spawn(move || {
            let mut ready = ready;
            loop {
                let database = redb::Database::create(&path).unwrap();
                let mut manager = GroupManager::open(&database, identity, None).unwrap();
                ready.send(()).unwrap();
                loop {
                    match receiver.recv().unwrap() {
                        Work::Job(job) => manager.execute(*job),
                        Work::Reopen(next) => {
                            ready = next;
                            break;
                        }
                        Work::Stop => return,
                    }
                }
                // Close the actual database and manager, retaining only the
                // serialized lane that still owns any subsequently admitted jobs.
            }
        });
        readiness.recv().unwrap();
        Self {
            client,
            sender,
            task,
        }
    }
    fn stop(self) {
        self.sender.send(Work::Stop).unwrap();
        self.task.join().unwrap();
    }
    fn reopen(&self) {
        let (ready, readiness) = mpsc::sync_channel(1);
        self.sender.send(Work::Reopen(ready)).unwrap();
        readiness.recv().unwrap();
    }
}

fn request_token(nonce: u8) -> [u8; 32] {
    static ISSUED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    prns_groups::retry::RequestToken::new(*ISSUED.get_or_init(now), [nonce; 20])
        .unwrap()
        .into_bytes()
}

async fn create(client: &Client, request: u8, title: &str) -> GroupId {
    let result = client
        .admit_enrollment(EnrollmentRequest::Create {
            request_id: request_token(request),
            title: title.into(),
            now: now(),
        })
        .unwrap()
        .await
        .unwrap();
    let EnrollmentResult::Created(group) = result else {
        panic!("expected created group")
    };
    group
}

async fn invite(owner: &Client, member: &Client, group: GroupId, request: u8) {
    let result = owner
        .admit_enrollment(EnrollmentRequest::Invite {
            request_id: request_token(request),
            group,
            recipient: member.identity().await.unwrap(),
            now: now(),
        })
        .unwrap()
        .await
        .unwrap();
    let EnrollmentResult::Invited(invitation) = result else {
        panic!("expected invitation")
    };
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if member.invitations(now()).await.unwrap().iter().any(|item| {
                item.id == invitation
                    && item.state == prns_groups_runner_native::enrollment::InvitationState::Pending
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("invitation reaches the identified recipient without provisioning files");
    member
        .admit_enrollment(EnrollmentRequest::Respond {
            request_id: request_token(request + 1),
            invitation,
            accept: true,
            now: now(),
        })
        .unwrap()
        .await
        .unwrap();
}

async fn members(client: &Client, group: GroupId, count: usize) {
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if client
                .network_configs()
                .await
                .unwrap()
                .iter()
                .any(|config| config.group == group && config.readers.len() == count)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("member catches up to the current authenticated control");
    let conversation = client
        .conversations()
        .await
        .unwrap()
        .into_iter()
        .find(|conversation| conversation.group == group)
        .unwrap();
    let config = client.for_group(group).network_config().await.unwrap();
    assert_eq!(conversation.members.len(), count);
    assert_eq!(conversation.member_count, count);
    assert_eq!(conversation.control_head, config.control.head);
    assert_eq!(
        conversation
            .members
            .iter()
            .filter(|member| member.is_local)
            .count(),
        1
    );
    assert_eq!(
        conversation
            .members
            .iter()
            .filter(|member| member.is_owner)
            .count(),
        1
    );
    for member in &conversation.members {
        assert!(config.readers.contains(&member.endpoint));
        if member.is_local {
            assert_eq!(member.identity, config.local_reticulum_identity);
        } else {
            assert!(config
                .peers
                .iter()
                .any(|peer| peer.endpoint == member.endpoint && peer.identity == member.identity));
        }
        assert_eq!(
            member.is_owner,
            Some(member.identity) == conversation.owner_identity
        );
    }
}

async fn delivered(client: &Client, group: GroupId, text: &str, receipts: Option<usize>) {
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let page = client
                .messages(group, None, 100, SystemClock.now())
                .await
                .unwrap();
            if page.messages.iter().any(|message| {
                message.text == text && receipts.is_none_or(|count| message.received_count == count)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("message and authenticated receipts converge through the network hub");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fresh_managers_create_invite_join_and_sync_multiple_groups_without_fixtures() {
    let directory = tempfile::tempdir().unwrap();
    let mut nodes = wire::Nodes::open().await;
    let identities: Vec<_> = nodes.owners[..3]
        .iter()
        .map(|host| host.client().identity_hash().unwrap())
        .collect();
    let workers: Vec<_> = identities
        .iter()
        .enumerate()
        .map(|(index, identity)| {
            Worker::open(
                directory.path().join(format!("member-{index}.redb")),
                *identity,
            )
        })
        .collect();
    assert!(workers[0].client.conversations().await.unwrap().is_empty());
    let mut stops = Vec::new();
    let mut tasks = Vec::new();
    for (index, worker) in workers.iter().enumerate() {
        let (sender, receiver) = watch::channel(false);
        stops.push(sender);
        tasks.push(tokio::spawn(run_multi(
            worker.client.clone(),
            nodes.owners[index].client(),
            prns_groups_reticulum::destination_for(identities[index]).unwrap(),
            prns_groups_reticulum::enrollment::destination_for(identities[index]).unwrap(),
            nodes.jobs[index].take().unwrap(),
            nodes.enrollment_jobs[index].take().unwrap(),
            wire::limits(),
            wire::enrollment_limits(),
            receiver,
        )));
    }
    let alice = &workers[0].client;
    let bob = &workers[1].client;
    let carol = &workers[2].client;
    let group = create(alice, 1, "A group created on the network").await;
    invite(alice, bob, group, 2).await;
    tokio::join!(members(alice, group, 2), members(bob, group, 2));
    nodes.state.set_online(1, false);
    alice
        .send_text(SendText {
            group,
            client_request_id: request_token(4),
            text: "before Carol".into(),
            clock: SystemClock.now(),
        })
        .await
        .unwrap();
    invite(alice, carol, group, 5).await;
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if carol
                .invitations(now())
                .await
                .unwrap()
                .iter()
                .any(|invite| invite.group == group && invite.waiting_for_delivery)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("accepted join waits while owner's current record lacks its receipt");
    assert_eq!(
        alice
            .for_group(group)
            .network_config()
            .await
            .unwrap()
            .readers
            .len(),
        2
    );
    assert!(carol
        .conversations()
        .await
        .unwrap()
        .iter()
        .all(|item| item.group != group));
    workers[2].reopen();
    assert!(carol
        .invitations(now())
        .await
        .unwrap()
        .iter()
        .any(|invite| { invite.group == group && invite.waiting_for_delivery }));
    nodes.state.set_online(1, true);
    delivered(bob, group, "before Carol", None).await;
    delivered(alice, group, "before Carol", Some(1)).await;
    tokio::join!(
        members(alice, group, 3),
        members(bob, group, 3),
        members(carol, group, 3)
    );
    bob.send_text(SendText {
        group,
        client_request_id: request_token(7),
        text: "after Carol".into(),
        clock: SystemClock.now(),
    })
    .await
    .unwrap();
    tokio::join!(
        delivered(alice, group, "after Carol", None),
        delivered(carol, group, "after Carol", None)
    );
    delivered(bob, group, "after Carol", Some(2)).await;

    let private = create(alice, 8, "Another independent group").await;
    invite(alice, bob, private, 9).await;
    tokio::join!(members(alice, private, 2), members(bob, private, 2));
    alice
        .send_text(SendText {
            group: private,
            client_request_id: request_token(11),
            text: "separate conversation".into(),
            clock: SystemClock.now(),
        })
        .await
        .unwrap();
    delivered(bob, private, "separate conversation", None).await;
    delivered(alice, private, "separate conversation", Some(1)).await;
    assert_eq!(bob.conversations().await.unwrap().len(), 2);
    assert_eq!(carol.conversations().await.unwrap().len(), 1);
    assert!(!bob
        .messages(group, None, 100, SystemClock.now())
        .await
        .unwrap()
        .messages
        .iter()
        .any(|message| message.text == "separate conversation"));

    // Reopen the sender's MLS/catalog database before another send. Carol misses
    // that transmission; after Bob goes offline she must obtain it from Alice.
    workers[1].reopen();
    nodes.state.set_online(2, false);
    bob.send_text(SendText {
        group,
        client_request_id: request_token(12),
        text: "sent after database restart".into(),
        clock: SystemClock.now(),
    })
    .await
    .unwrap();
    delivered(alice, group, "sent after database restart", None).await;
    delivered(bob, group, "sent after database restart", Some(1)).await;
    assert!(!carol
        .messages(group, None, 100, SystemClock.now())
        .await
        .unwrap()
        .messages
        .iter()
        .any(|message| message.text == "sent after database restart"));
    nodes.state.set_online(1, false);
    nodes.state.set_online(2, true);
    delivered(carol, group, "sent after database restart", None).await;
    nodes.state.set_online(1, true);
    delivered(bob, group, "sent after database restart", Some(2)).await;

    for stop in stops {
        stop.send(true).unwrap();
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    for worker in workers {
        worker.stop();
    }
    // Reopen current ratchets/catalogs without any import or enrollment fixture.
    let reopened = Worker::open(directory.path().join("member-1.redb"), identities[1]);
    assert_eq!(reopened.client.conversations().await.unwrap().len(), 2);
    delivered(&reopened.client, group, "after Carol", Some(2)).await;
    delivered(
        &reopened.client,
        group,
        "sent after database restart",
        Some(2),
    )
    .await;
    delivered(&reopened.client, private, "separate conversation", None).await;
    reopened.stop();
    nodes.stop().await;
}
