use super::*;
use crate::contract::{CreateGroupInput, GroupActionInput, GroupActionState};

#[tokio::test]
async fn pending_and_expired_action_intents_remain_retirement_obligations_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let events = MessagingEvents::new(2).unwrap();
    let host = open_host(&events).await;
    let secret = IdentitySecretKey::new([67; 64]);
    let local = prns_core::identity::PrivateIdentityMaterial::from(secret.clone());
    let scope = StorageScope {
        identity: core::LocalIdentityId {
            incarnation: [68; 16],
            sequence: 1,
        },
        public_hash: core::PublicIdentityHash(*local.identity_hash().as_bytes()),
    };
    let key = core::KeyReference([69; 32]);
    let mut identity = MessagingIdentity::open(
        root.path(),
        scope,
        key,
        secret.clone(),
        true,
        host.client(),
        &events,
    )
    .unwrap();
    let before = identity.observe_dependencies().await.unwrap();
    assert!(!before.recovery_pending);
    let actions = identity.store.as_ref().unwrap().group_action_client();
    let request_id = RequestToken::new(1_700_000_000, [70; 20])
        .unwrap()
        .into_bytes();
    actions
        .execute(crate::group_actions::Request::Prepare(
            GroupActionInput::Create {
                input: CreateGroupInput {
                    client_request_id: request_id,
                    title: "Unexecuted intent".into(),
                },
            },
        ))
        .await
        .unwrap();
    let pending = identity.observe_dependencies().await.unwrap();
    assert_eq!(pending.authority_count, 0, "no group exists yet");
    assert!(pending.recovery_pending);
    assert_eq!(pending.unsettled_control, 1);
    assert!(pending.revision > before.revision);
    assert_eq!(identity.observe_dependencies().await.unwrap(), pending);
    actions
        .execute(crate::group_actions::Request::Finish {
            request_id,
            state: GroupActionState::RetryExpired,
        })
        .await
        .unwrap();
    let expired = identity.observe_dependencies().await.unwrap();
    assert!(
        expired.recovery_pending,
        "expired retry needs explicit review"
    );
    assert_eq!(expired.unsettled_control, 1);
    assert!(expired.revision > pending.revision);
    assert_eq!(identity.pause().await.unwrap(), expired);
    drop(identity);
    let mut identity = MessagingIdentity::open(
        root.path(),
        scope,
        key,
        secret,
        false,
        host.client(),
        &events,
    )
    .unwrap();
    assert_eq!(identity.observe_dependencies().await.unwrap(), expired);
    assert_eq!(identity.pause().await.unwrap(), expired);
    host.stop().await.unwrap();
}
