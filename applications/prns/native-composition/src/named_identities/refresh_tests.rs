#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::development_store::DevelopmentStoreOwner;
use prns_core::identity::vault::IdentitySecretKey;
use prns_lxmf::direct::{
    CallbackOutcome, DirectAnnounceFailure, DirectDeliveryReceipt, DirectNetwork,
    DirectNetworkFuture, DirectSendFailure, LocalLxmfIdentity,
};
use prns_lxmf::mailbox::{
    DurableLxmfDeliveryState, DurableSendDirectTextOutcome, MailboxConversationListRequest,
    MailboxReply, MailboxRequest, SystemMailboxClock,
};
use std::time::Duration;
use tokio::sync::Notify;

struct HeldNetwork {
    peer: LocalLxmfIdentity,
    entered: Notify,
    release: Notify,
}
impl DirectNetwork for HeldNetwork {
    fn has_route(&self, _: [u8; 16]) -> DirectNetworkFuture<'_, bool> {
        Box::pin(async { true })
    }
    fn request_path(&self, _: [u8; 16]) -> DirectNetworkFuture<'_, Result<(), DirectSendFailure>> {
        Box::pin(async { Ok(()) })
    }
    fn establish_link(
        &self,
        _: [u8; 16],
    ) -> DirectNetworkFuture<'_, Result<[u8; 16], DirectSendFailure>> {
        Box::pin(async { Ok([7; 16]) })
    }
    fn send_link_packet(
        &self,
        _: [u8; 16],
        _: Vec<u8>,
    ) -> DirectNetworkFuture<'_, Result<DirectDeliveryReceipt, DirectSendFailure>> {
        Box::pin(async {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(DirectDeliveryReceipt { rtt_millis: 135 })
        })
    }
    fn destination_public_key(
        &self,
        destination: [u8; 16],
    ) -> DirectNetworkFuture<'_, Option<[u8; 64]>> {
        Box::pin(
            async move { (destination == self.peer.destination()).then(|| self.peer.public_key()) },
        )
    }
    fn mark_destination_used(&self, _: [u8; 16]) -> DirectNetworkFuture<'_, ()> {
        Box::pin(async {})
    }
    fn announce(&self, _: [u8; 16]) -> DirectNetworkFuture<'_, Result<(), DirectAnnounceFailure>> {
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    store: DevelopmentStoreOwner,
    service: Arc<DurableDirectLxmfService>,
    network: Arc<HeldNetwork>,
}
impl Fixture {
    async fn new(seed: u8) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = DevelopmentStoreOwner::open(root.path(), &root.path().join("development.redb"))
            .unwrap();
        let local =
            LocalLxmfIdentity::from_secret_bytes(&IdentitySecretKey::new([seed; 64])).unwrap();
        let network = Arc::new(HeldNetwork {
            peer: LocalLxmfIdentity::from_secret_bytes(&IdentitySecretKey::new([99; 64])).unwrap(),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let (pending, _) = DurableDirectLxmfService::prepare(local);
        let service = Arc::new(
            pending
                .start(
                    network.clone(),
                    store.mailbox_submitter(),
                    Arc::new(SystemMailboxClock),
                )
                .await
                .unwrap(),
        );
        Self {
            _root: root,
            store,
            service,
            network,
        }
    }
    async fn health(&self) -> LxmfHealth {
        wait_for_service_change(&self.service, None).await.unwrap()
    }
    async fn close(self) {
        self.service.stop().await.unwrap();
        self.store.close().unwrap();
    }
    fn receive(&self) {
        let mut wire = [0; prns_lxmf::wire::MAX_BASIC_LXMF_WIRE_BYTES];
        let prepared = prns_lxmf::wire::compose_basic_direct_lxmf(
            self.service.local_destination(),
            self.network.peer.destination(),
            1_700_000_001_000,
            b"",
            b"verified reply",
            None,
            &self.network.peer,
            &mut wire,
        )
        .unwrap();
        assert_eq!(
            self.service
                .callbacks()
                .on_host_link_delivery(&prns_host::LinkDelivery {
                    link_id: prns_host::LinkId::new([7; 16]),
                    local_destination: Some(prns_host::DestinationHash::new(
                        self.service.local_destination()
                    )),
                    arrived_at_millis: 1_700_000_001_005,
                    source_interface: prns_host::InterfaceId::new([8; 8]),
                    plaintext: wire[..usize::from(prepared.wire_len())].to_vec(),
                }),
            CallbackOutcome::Enqueued
        );
    }
}

fn messages(outcome: LxmfMessageListOutcome) -> Vec<LxmfMessage> {
    let LxmfMessageListOutcome::Listed { messages } = outcome else {
        panic!("history unavailable")
    };
    messages
}
fn all_messages() -> MailboxListRequest {
    MailboxListRequest {
        peer: None,
        direction: None,
        before: None,
        limit: 20,
    }
}
async fn changed(fixture: &Fixture, after: u64) -> LxmfHealth {
    tokio::time::timeout(
        Duration::from_secs(2),
        wait_for_service_change(&fixture.service, Some(after)),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn named_projection_changes_cover_sending_delivery_and_verified_inbound() {
    let f = Fixture::new(31).await;
    let peer = personal_rns::identity::PrivateIdentityMaterial::from_bytes([99; 64]);
    let mut learned = f.service.subscribe();
    assert_eq!(
        f.service
            .callbacks()
            .on_authenticated_announce(&prns_host_native::AuthenticatedAnnounce {
                destination: prns_host::DestinationHash::new(f.network.peer.destination()),
                announced_identity: prns_host::IdentityHash::new(*peer.identity_hash().as_bytes()),
                arrived_at_millis: 1_700_000_000_000,
                app_data: &[],
                is_path_response: false,
                source_interface: prns_host::InterfaceId::new([8; 8]),
                hops: 2,
            }),
        CallbackOutcome::Enqueued
    );
    tokio::time::timeout(Duration::from_secs(2), learned.changed())
        .await
        .unwrap()
        .unwrap();
    let initial = f.health().await;
    assert_eq!(initial.mailbox_revision, 0);
    assert_eq!(
        f.service
            .send_direct_text(
                f.network.peer.destination(),
                1_700_000_000_000,
                b"",
                b"reply"
            )
            .await,
        DurableSendDirectTextOutcome::Accepted { local_record_id: 1 }
    );
    f.network.entered.notified().await;
    let sending = changed(&f, initial.projection_revision).await;
    assert_eq!(sending.mailbox_revision, 1);
    let rows = messages(
        direct::service_messages(&f.service, all_messages())
            .await
            .unwrap(),
    );
    assert!(matches!(
        rows[0].delivery_state,
        LxmfDeliveryState::Sending { failed_attempts: 0 }
    ));
    let conversations = messages(
        direct::service_conversations(
            &f.service,
            MailboxConversationListRequest {
                before: None,
                limit: 20,
            },
        )
        .await
        .unwrap(),
    );
    assert!(matches!(
        conversations[0].delivery_state,
        LxmfDeliveryState::Sending { failed_attempts: 0 }
    ));
    let MailboxReply::Listed { messages: raw, .. } = f
        .store
        .mailbox_submitter()
        .submit(MailboxRequest::List(all_messages()))
        .await
        .unwrap()
    else {
        panic!("raw history")
    };
    assert!(matches!(
        raw[0].delivery_state,
        DurableLxmfDeliveryState::Queued { failed_attempts: 0 }
    ));

    // Complete before starting the next wait: cursor comparison must catch the
    // already-finished delivery rather than waiting for a later unrelated event.
    let mut completed = f.service.subscribe();
    f.network.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let rows = messages(
                direct::service_messages(&f.service, all_messages())
                    .await
                    .unwrap(),
            );
            if matches!(
                rows[0].delivery_state,
                LxmfDeliveryState::Delivered { rtt: Some(135), .. }
            ) {
                break;
            }
            completed.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let settled = changed(&f, sending.projection_revision).await;
    assert_eq!(settled.mailbox_revision, 2);
    let mut waiting = Box::pin(wait_for_service_change(
        &f.service,
        Some(settled.projection_revision),
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut waiting)
            .await
            .is_err(),
        "an unchanged healthy snapshot must remain idle"
    );
    f.receive();
    let received = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.mailbox_revision, 3);
    let rows = messages(
        direct::service_messages(&f.service, all_messages())
            .await
            .unwrap(),
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].direction, LxmfDirection::Inbound);
    assert_eq!(rows[0].verification, LxmfVerification::Verified);
    f.close().await;
}

#[tokio::test]
async fn named_change_wait_is_owner_scoped_cancel_safe_and_terminal_after_stop() {
    let first = Fixture::new(32).await;
    let second = Fixture::new(33).await;
    let first_before = first.health().await;
    let second_before = second.health().await;
    let mut second_wait = Box::pin(wait_for_service_change(
        &second.service,
        Some(second_before.projection_revision),
    ));
    first.receive();
    let first_after = changed(&first, first_before.projection_revision).await;
    assert_eq!(first_after.mailbox_revision, 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut second_wait)
            .await
            .is_err()
    );
    drop(second_wait);
    assert_eq!(
        second.health().await,
        second_before,
        "cancelling a waiter must not mutate its mailbox"
    );
    let mut retiring = Box::pin(wait_for_service_change(
        &first.service,
        Some(first_after.projection_revision),
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut retiring)
            .await
            .is_err()
    );
    first.service.stop().await.unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(2), retiring)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.state, LxmfHealthState::Stopped);
    assert_eq!(
        wait_for_service_change(&first.service, Some(stopped.projection_revision))
            .await
            .unwrap()
            .state,
        LxmfHealthState::Stopped,
        "a terminal cursor must not start another idle wait"
    );
    first.close().await;
    second.close().await;
}
