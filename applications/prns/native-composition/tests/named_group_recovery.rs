//! Production named composition and generated-API implementation, with independent
//! installation stores and real Reticulum transport over a controlled carrier.
#![cfg(feature = "host-test")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "support/named_wire.rs"]
mod wire;

use prns_app::{contract::*, identity_services::MessagingEvents, named_identities as named};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
enum RecoveryOrder {
    SenderFirst,
    ReceiverFirst,
}

fn use_owner(owner: &Arc<named::NamedRuntime>) {
    // Each endpoint models a separate app installation. Only the foreign-call
    // entry point is switched, between fully awaited calls; background runners
    // and admitted work always retain their own immutable native owner.
    named::install_global(Some(owner.clone()));
}
async fn catalog(owner: &Arc<named::NamedRuntime>) -> NamedIdentityCatalog {
    use_owner(owner);
    match named::list_named_identities().await {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog,
        other => panic!("catalog: {other:?}"),
    }
}
async fn open(
    wire: &wire::Wire,
    index: usize,
    root: &Path,
) -> (
    Arc<prns_host_native::owner::OwnedSession>,
    Arc<named::NamedRuntime>,
) {
    let events = MessagingEvents::new(named::MAX_IDENTITIES).unwrap();
    let host = wire.host(index, &events).await;
    let owner = named::NamedRuntime::open(root, host.client(), events)
        .await
        .unwrap();
    (host, owner)
}
async fn change(
    owner: &Arc<named::NamedRuntime>,
    identity: &NamedIdentityReference,
    change: NamedIdentityChange,
) {
    use_owner(owner);
    assert!(matches!(
        named::change_named_identity(ChangeNamedIdentityInput {
            identity: identity.clone(),
            change,
        })
        .await,
        NamedIdentityCatalogOutcome::Ready { .. }
    ));
}
fn token() -> [u8; 32] {
    prns_groups_runner_native::create_request_id().unwrap()
}
async fn history(
    owner: &Arc<named::NamedRuntime>,
    client: &named::NamedIdentityClient,
    group: [u8; 32],
) -> Vec<GroupTextMessage> {
    use_owner(owner);
    match client
        .list_group_messages(ListGroupMessagesInput {
            group_id: group,
            before_sequence: None,
            limit: 20,
        })
        .await
    {
        GroupMessageListOutcome::Ready { messages, .. } => messages,
        other => panic!("history: {other:?}"),
    }
}
async fn wait_members(
    owner: &Arc<named::NamedRuntime>,
    client: &named::NamedIdentityClient,
    group: [u8; 32],
) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            use_owner(owner);
            if matches!(client.list_group_conversations().await,
                GroupConversationListOutcome::Ready { conversations }
                if conversations.iter().any(|c| c.group_id == group && c.member_count == 2))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("authenticated membership reaches both named stores");
}
async fn assert_empty(owner: &Arc<named::NamedRuntime>, client: &named::NamedIdentityClient) {
    use_owner(owner);
    match client.list_group_conversations().await {
        GroupConversationListOutcome::Ready { conversations } => assert!(conversations.is_empty()),
        other => panic!("other-owner conversations: {other:?}"),
    }
    match client.list_group_actions().await {
        GroupActionListOutcome::Ready { actions } => assert!(actions.is_empty()),
        other => panic!("other-owner actions: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn named_group_recovery_preserves_records_in_both_carrier_and_reopen_orders() {
    // The foreign entry point is process-global, so these independent installation
    // pairs must run sequentially rather than as parallel Rust tests.
    for order in [RecoveryOrder::SenderFirst, RecoveryOrder::ReceiverFirst] {
        tokio::time::timeout(Duration::from_secs(360), run_recovery(order))
            .await
            .unwrap_or_else(|_| panic!("bounded named group recovery trial: {order:?}"));
    }
}

async fn wait_transport_failure(
    owner: &Arc<named::NamedRuntime>,
    client: &named::NamedIdentityClient,
    group: [u8; 32],
) {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            use_owner(owner);
            if matches!(client.list_group_conversations().await,
                GroupConversationListOutcome::Ready { conversations }
                if conversations.iter().any(|conversation|
                    conversation.group_id == group
                    && conversation.progress.running
                    && conversation.progress.peers.iter().any(|peer|
                        peer.failures > 0
                        && peer.phase == GroupSynchronizationPhase::RetryScheduled
                        && peer.pending == Some(true)
                        && peer.last_issue == Some(GroupSynchronizationIssue::Transport))))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("carrier outage must exercise an observed failed attempt, not only a stopped owner");
}

async fn wait_transport_healthy(
    owner: &Arc<named::NamedRuntime>,
    client: &named::NamedIdentityClient,
    group: [u8; 32],
) {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            use_owner(owner);
            if matches!(client.list_group_conversations().await,
                GroupConversationListOutcome::Ready { conversations }
                if conversations.iter().any(|conversation|
                    conversation.group_id == group
                    && conversation.progress.running
                    && conversation.progress.peers.iter().any(|peer|
                        peer.failures == 0
                        && peer.phase == GroupSynchronizationPhase::Waiting
                        && peer.last_issue.is_none())))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("recovered peer completes a healthy exchange before the second interruption");
}

async fn run_recovery(order: RecoveryOrder) {
    let root = tempfile::tempdir().unwrap();
    let sender_path = root.path().join("sender");
    let recipient_path = root.path().join("recipient");
    std::fs::create_dir_all(&sender_path).unwrap();
    std::fs::create_dir_all(&recipient_path).unwrap();
    let wire = wire::Wire::new();
    let (sender_host, sender) = open(&wire, 0, &sender_path).await;
    let (recipient_host, recipient) = open(&wire, 1, &recipient_path).await;
    let identity = catalog(&sender).await.identities[0].reference.clone();
    let peer_identity = catalog(&recipient).await.identities[0].reference.clone();
    let client = named::open_named_identity(identity.clone());
    let peer = named::open_named_identity(peer_identity.clone());
    use_owner(&sender);
    let other = match named::create_named_identity(CreateNamedIdentityInput {
        request_id: [0x78; 32],
        label: "Unrelated work".into(),
        published_name: None,
    })
    .await
    {
        NamedIdentityCatalogOutcome::Ready { catalog } => catalog.identities[1].reference.clone(),
        outcome => panic!("create: {outcome:?}"),
    };
    let other_client = named::open_named_identity(other.clone());
    let group = match client
        .create_group(CreateGroupInput {
            client_request_id: token(),
            title: "Named pending recovery".into(),
        })
        .await
    {
        CreateGroupOutcome::Accepted { group_id } => group_id,
        outcome => panic!("create group: {outcome:?}"),
    };
    let invitation = match client
        .invite_to_group(InviteToGroupInput {
            client_request_id: token(),
            group_id: group,
            recipient_identity: peer_identity.public_hash,
        })
        .await
    {
        InviteToGroupOutcome::Queued { invitation_id } => invitation_id,
        outcome => panic!("invite: {outcome:?}"),
    };
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            use_owner(&recipient);
            if let GroupInvitationListOutcome::Ready { invitations } =
                peer.list_group_invitations().await
            {
                if invitations.iter().any(|item| {
                    item.invitation_id == invitation && item.state == GroupInvitationState::Offered
                }) {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("invitation crosses a real authenticated Link");
    use_owner(&recipient);
    assert_eq!(
        peer.respond_group_invitation(RespondGroupInvitationInput {
            client_request_id: token(),
            invitation_id: invitation,
            accept: true,
        })
        .await,
        RespondGroupInvitationOutcome::Saved
    );
    wait_members(&sender, &client, group).await;
    wait_members(&recipient, &peer, group).await;

    // The complete peer carrier is disabled before publication. This cannot
    // accidentally pass because another app/interface delivered the record.
    wire.set_online(0, false);
    wire.set_online(1, false);
    let request_id = token();
    let reference = GroupActionReference {
        owner_identity: identity.public_hash,
        client_request_id: request_id,
    };
    use_owner(&sender);
    let prepared = client
        .prepare_group_action(PrepareGroupActionInput {
            owner_identity: identity.public_hash,
            input: GroupActionInput::Send {
                input: SendGroupTextInput {
                    group_id: group,
                    client_request_id: request_id,
                    text: "Accepted before named pause".into(),
                },
            },
        })
        .await;
    match prepared {
        GroupActionOutcome::Ready { action } => assert_eq!(action.state, GroupActionState::Pending),
        other => panic!("prepare action: {other:?}"),
    }
    let completed = client.execute_group_action(reference.clone()).await;
    let message_id = match &completed {
        GroupActionOutcome::Ready {
            action:
                GroupAction {
                    state:
                        GroupActionState::Completed {
                            result: GroupActionResult::Sent { message_id },
                        },
                    ..
                },
        } => *message_id,
        other => panic!("send must be durably accepted: {other:?}"),
    };
    let accepted = history(&sender, &client, group).await;
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].message_id, message_id);
    assert_eq!(accepted[0].client_request_id, Some(request_id));
    assert_eq!(
        (accepted[0].received_count, accepted[0].recipient_count),
        (0, 1)
    );
    assert!(history(&recipient, &peer, group).await.is_empty());
    change(&sender, &other, NamedIdentityChange::Select).await;
    change(&sender, &identity, NamedIdentityChange::Pause).await;
    use_owner(&sender);
    assert_eq!(
        client.execute_group_action(reference.clone()).await,
        completed
    );
    assert_empty(&sender, &other_client).await;
    named::install_global(None);
    sender.stop().await.unwrap();
    drop(sender);
    sender_host.stop().await.unwrap();
    drop(sender_host);
    recipient.stop().await.unwrap();
    drop(recipient);
    recipient_host.stop().await.unwrap();
    drop(recipient_host);

    // Actual service/database/host closure and reopening, not merely a new
    // JavaScript handle. Both orders use production policy and remain disconnected
    // during reopening. This is graceful closure; crash tests are separate.
    let ((sender_host, sender), (recipient_host, recipient)) = match order {
        RecoveryOrder::SenderFirst => {
            let sender = open(&wire, 0, &sender_path).await;
            let recipient = open(&wire, 1, &recipient_path).await;
            (sender, recipient)
        }
        RecoveryOrder::ReceiverFirst => {
            let recipient = open(&wire, 1, &recipient_path).await;
            let sender = open(&wire, 0, &sender_path).await;
            (sender, recipient)
        }
    };
    assert_eq!(
        catalog(&recipient).await.identities[0].reference,
        peer_identity
    );
    let restored = catalog(&sender).await;
    assert_eq!(restored.selected, Some(other.clone()));
    let paused = restored
        .identities
        .iter()
        .find(|item| item.reference == identity)
        .unwrap();
    assert_eq!(paused.state, NamedIdentityState::Paused);
    assert!(!paused.ready && !paused.transitioning);
    assert_eq!(
        client.execute_group_action(reference.clone()).await,
        completed
    );
    assert_eq!(history(&sender, &client, group).await, accepted);
    assert_empty(&sender, &other_client).await;
    assert!(matches!(
        other_client.execute_group_action(reference.clone()).await,
        GroupActionOutcome::InvalidInput { .. }
    ));
    assert!(!matches!(
        other_client
            .list_group_messages(ListGroupMessagesInput {
                group_id: group,
                before_sequence: None,
                limit: 20,
            })
            .await,
        GroupMessageListOutcome::Ready { .. }
    ));
    use_owner(&sender);
    assert!(!matches!(
        client
            .send_group_text(SendGroupTextInput {
                group_id: group,
                client_request_id: token(),
                text: "Must stay blocked while paused".into(),
            })
            .await,
        GroupTextSendOutcome::Accepted { .. }
    ));
    assert_eq!(history(&sender, &client, group).await.len(), 1);
    if matches!(order, RecoveryOrder::ReceiverFirst) {
        wire.set_online(1, true);
    }
    change(&sender, &identity, NamedIdentityChange::Resume).await;
    use_owner(&sender);
    assert_eq!(client.execute_group_action(reference).await, completed);
    if matches!(order, RecoveryOrder::SenderFirst) {
        wire.set_online(0, true);
    }
    // Observing an actual failed attempt also proves that simple history reads
    // and owner selection do not discard the retained obligation or rescue it.
    wait_transport_failure(&sender, &client, group).await;
    assert_eq!(history(&sender, &client, group).await, accepted);
    assert!(history(&recipient, &peer, group).await.is_empty());
    let recovery_started = Instant::now();
    wire.set_online(0, true);
    wire.set_online(1, true);
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let sent = history(&sender, &client, group).await;
            let received = history(&recipient, &peer, group).await;
            assert_eq!(sent.len(), 1, "recovery must not publish a replacement");
            assert!(received.len() <= 1, "recipient must deduplicate retry");
            if sent[0].received_count == 1 && received.len() == 1 {
                assert_eq!(sent[0].message_id, message_id);
                assert_eq!(sent[0].client_request_id, Some(request_id));
                assert_eq!(received[0].message_id, message_id);
                assert_eq!(received[0].text, accepted[0].text);
                assert!(!received[0].outgoing);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("resumed original record reaches recipient and receipt reconciles");
    assert_empty(&sender, &other_client).await;
    assert!(wire.delivered.load(std::sync::atomic::Ordering::SeqCst) > 0);
    println!(
        "{order:?}: reopened original record and receipt reconciled in {:?}",
        recovery_started.elapsed()
    );

    // Interrupt the recovered carrier again and publish in the opposite direction.
    // Keep both native owners running this time, so recovery must retain their
    // failed scheduling state rather than relying on process restart to clear it.
    wait_transport_healthy(&recipient, &peer, group).await;
    wire.set_online(0, false);
    wire.set_online(1, false);
    let reverse_request = token();
    use_owner(&recipient);
    let reverse_record = match peer
        .send_group_text(SendGroupTextInput {
            group_id: group,
            client_request_id: reverse_request,
            text: "Retained through the second carrier interruption".into(),
        })
        .await
    {
        GroupTextSendOutcome::Accepted { message_id } => message_id,
        outcome => panic!("reverse durable send: {outcome:?}"),
    };
    wait_transport_failure(&recipient, &peer, group).await;
    assert_eq!(history(&sender, &client, group).await.len(), 1);
    let pending = history(&recipient, &peer, group).await;
    assert_eq!(pending.len(), 2);
    let pending = pending
        .iter()
        .find(|message| message.message_id == reverse_record)
        .unwrap();
    assert_eq!(pending.client_request_id, Some(reverse_request));
    assert_eq!((pending.received_count, pending.recipient_count), (0, 1));

    // For the reversed send, endpoint 1 is now the sender. Restore only one side
    // first and check absence before enabling the second; no announce or resend.
    let first = match order {
        RecoveryOrder::SenderFirst => 1,
        RecoveryOrder::ReceiverFirst => 0,
    };
    wire.set_online(first, true);
    assert_eq!(history(&sender, &client, group).await.len(), 1);
    let recovery_started = Instant::now();
    wire.set_online(1 - first, true);
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let original_author = history(&sender, &client, group).await;
            let reverse_author = history(&recipient, &peer, group).await;
            assert!(original_author.len() <= 2);
            assert_eq!(reverse_author.len(), 2);
            assert_eq!(
                original_author
                    .iter()
                    .filter(|message| message.message_id == message_id)
                    .count(),
                1,
                "original record survives repeated carrier recovery"
            );
            if original_author.len() == 2
                && reverse_author.iter().any(|message| {
                    message.message_id == reverse_record && message.received_count == 1
                })
            {
                assert_eq!(
                    original_author
                        .iter()
                        .filter(|message| message.message_id == reverse_record && !message.outgoing)
                        .count(),
                    1
                );
                assert_eq!(
                    reverse_author
                        .iter()
                        .filter(|message| message.message_id == reverse_record
                            && message.outgoing
                            && message.client_request_id == Some(reverse_request))
                        .count(),
                    1
                );
                assert_eq!(
                    original_author
                        .iter()
                        .find(|message| message.message_id == message_id)
                        .unwrap()
                        .received_count,
                    1
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("second carrier interruption recovers the same reverse record and its receipt");
    assert_empty(&sender, &other_client).await;
    println!(
        "{order:?}: second interruption, reverse record and receipt reconciled in {:?}",
        recovery_started.elapsed()
    );
    named::install_global(None);
    sender.stop().await.unwrap();
    recipient.stop().await.unwrap();
    sender_host.stop().await.unwrap();
    recipient_host.stop().await.unwrap();
}
