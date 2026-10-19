use super::{detail, MessagingIdentity};
use crate::{
    contract::*,
    directory::{DirectoryRequest, DirectoryResponse},
};
use prns_lxmf::mailbox::{MailboxReply, MailboxRequest};

pub(super) async fn directory(
    s: &MessagingIdentity,
    request: DirectoryRequest,
) -> Result<DirectoryResponse, String> {
    s.store()
        .map_err(detail)?
        .admit_directory_async(request)
        .map_err(detail)?
        .await
        .map_err(detail)?
        .map_err(detail)
}
pub(super) async fn mutation(
    s: &MessagingIdentity,
    request: DirectoryRequest,
) -> Result<ContactMutationOutcome, String> {
    match directory(s, request).await? {
        DirectoryResponse::Mutation(value) => Ok(value),
        _ => Err("Unexpected contact result.".into()),
    }
}
pub(super) async fn get(
    s: &MessagingIdentity,
    input: ContactDestinationInput,
) -> Result<ContactLookupOutcome, String> {
    match directory(
        s,
        DirectoryRequest::Get {
            destination: input.destination,
        },
    )
    .await?
    {
        DirectoryResponse::Lookup(value) => Ok(value),
        _ => Err("Unexpected contact result.".into()),
    }
}
pub(super) async fn contacts(s: &MessagingIdentity) -> Result<ContactListOutcome, String> {
    match directory(s, DirectoryRequest::List).await? {
        DirectoryResponse::List(value) => Ok(value),
        _ => Err("Unexpected contact result.".into()),
    }
}
pub(super) async fn observed(
    s: &MessagingIdentity,
    input: ContactDestinationInput,
    messaging_only: bool,
) -> Result<ContactMutationOutcome, String> {
    let handle = s
        .host()
        .native_services()
        .map_err(detail)?
        .protocols()
        .clone();
    let Some(observation) = handle
        .destination_identity(
            personal_rns::node_introspection::DestinationIdentityQuery::Destination(
                personal_rns::DestinationHash::new(input.destination),
            ),
        )
        .await
    else {
        return Ok(ContactMutationOutcome::NotObserved);
    };
    let identity = *observation.identity.as_bytes();
    let request = if messaging_only {
        let snapshot = s
            .direct()
            .map_err(detail)?
            .snapshot(prns_lxmf::mailbox::MailboxListRequest {
                peer: None,
                direction: None,
                before: None,
                limit: 1,
            })
            .await
            .map_err(detail)?;
        let Some(peer) = snapshot
            .peers
            .into_iter()
            .find(|p| p.destination == input.destination && p.announced_identity == identity)
        else {
            return Ok(ContactMutationOutcome::NotObserved);
        };
        DirectoryRequest::SaveDiscovered {
            destination: input.destination,
            identity,
            announced_name: peer.display_name,
        }
    } else {
        DirectoryRequest::SaveObserved {
            destination: input.destination,
            identity,
        }
    };
    mutation(s, request).await
}
pub(super) async fn messages(
    s: &MessagingIdentity,
    input: ListLxmfMessagesInput,
) -> Result<LxmfMessageListOutcome, String> {
    let request = match crate::lxmf::mailbox_list_request(input) {
        Ok(r) => r,
        Err(o) => return Ok(o),
    };
    match s
        .store()
        .map_err(detail)?
        .admit_mailbox_async(MailboxRequest::List(request))
        .map_err(detail)?
        .await
        .map_err(detail)?
        .map_err(detail)?
    {
        MailboxReply::Listed { messages, .. } => Ok(crate::lxmf::project_messages(&messages)),
        _ => Err("Unexpected history response.".into()),
    }
}
pub(super) async fn conversations(
    s: &MessagingIdentity,
    input: ListLxmfConversationsInput,
) -> Result<LxmfMessageListOutcome, String> {
    let request = match crate::lxmf::conversation_list_request(input) {
        Ok(r) => r,
        Err(o) => return Ok(o),
    };
    match s
        .store()
        .map_err(detail)?
        .admit_mailbox_async(MailboxRequest::ListConversations(request))
        .map_err(detail)?
        .await
        .map_err(detail)?
        .map_err(detail)?
    {
        MailboxReply::ConversationsListed { messages, .. } => {
            Ok(crate::lxmf::project_messages(&messages))
        }
        _ => Err("Unexpected conversation response.".into()),
    }
}
pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
pub(super) async fn send(
    s: &MessagingIdentity,
    input: SendDirectTextInput,
) -> Result<SendDirectTextOutcome, String> {
    if !crate::input::bounded_text(&[&input.title, &input.content]) {
        return Err("The text exceeds the native size limit.".into());
    }
    let expected = match get(
        s,
        ContactDestinationInput {
            destination: input.destination,
        },
    )
    .await?
    {
        ContactLookupOutcome::Found { contact } => contact.identity,
        ContactLookupOutcome::NotFound => None,
        outcome => return Err(format!("Contact lookup unavailable: {outcome:?}")),
    };
    Ok(crate::lxmf::project_send_outcome(
        s.direct()
            .map_err(detail)?
            .send_direct_text_to_identity(
                input.destination,
                expected,
                now(),
                input.title.as_bytes(),
                input.content.as_bytes(),
            )
            .await,
    ))
}
pub(super) async fn profile(
    s: &MessagingIdentity,
    name: Option<String>,
) -> Result<LocalMessagingProfileOutcome, String> {
    let name = match name.map(|n| normalize_name(&n)).transpose() {
        Ok(n) => n,
        Err(detail) => return Ok(LocalMessagingProfileOutcome::InvalidInput { detail }),
    };
    let update = name.is_some();
    let (tx, rx) = tokio::sync::oneshot::channel();
    s.store()
        .map_err(detail)?
        .admit_messaging_name(name, move |result| {
            let _ = tx.send(result);
        })
        .map_err(detail)?;
    let name = rx.await.map_err(detail)?.map_err(detail)?;
    s.cache_published_name(name.clone()).map_err(detail)?;
    let profile = crate::messaging_profile::profile(
        name.clone(),
        &PrimaryIdentityState::Present {
            identity_hash: s.scope().public_hash.0.to_vec(),
        },
    );
    if update && s.is_network_active() {
        let data = if name.is_empty() {
            Vec::new()
        } else {
            crate::messaging_profile::announce_data(&name)?
        };
        let app_data =
            personal_rns::routing::announce::emit::AnnounceAppDataBytes::from_slice(&data)
                .map_err(detail)?;
        if let Some(destination) = profile.destination {
            let result = match s.network_scope() {
                Ok(scope) => scope
                    .protocol_operation(move |handle| async move {
                        handle
                            .set_registered_announce_app_data(
                                personal_rns::engine::SetRegisteredAnnounceAppData {
                                    destination: personal_rns::DestinationHash::new(destination),
                                    app_data,
                                },
                            )
                            .await
                            .map_err(detail)
                    })
                    .await
                    .map_err(detail)
                    .and_then(|result| result),
                Err(error) => Err(detail(error)),
            };
            if let Err(detail) = result {
                return Ok(LocalMessagingProfileOutcome::SavedButNotApplied { profile, detail });
            }
        }
    }
    Ok(LocalMessagingProfileOutcome::Ready { profile })
}

pub(super) async fn cancel(
    s: &MessagingIdentity,
    input: CancelLxmfMessageInput,
) -> Result<CancelLxmfMessageOutcome, String> {
    if let Ok(service) = s.direct() {
        return Ok(crate::lxmf::project_cancel_outcome(
            service
                .cancel_lxmf_message(input.local_record_id, now())
                .await,
        ));
    }
    let reply = s
        .store()
        .map_err(detail)?
        .admit_mailbox_async(MailboxRequest::Cancel {
            local_record_id: input.local_record_id,
            cancelled_at_millis: now(),
        })
        .map_err(detail)?
        .await
        .map_err(detail)?
        .map_err(detail)?;
    use prns_lxmf::mailbox::{CancelLxmfMessageOutcome as Outcome, CancelTransition};
    let MailboxReply::Cancel(transition) = reply else {
        return Err("Unexpected cancellation response.".into());
    };
    Ok(crate::lxmf::project_cancel_outcome(match transition {
        CancelTransition::Cancelled { message, .. } => Outcome::Cancelled {
            local_record_id: message.local_record_id,
        },
        CancelTransition::NotFound => Outcome::NotFound,
        CancelTransition::AlreadyDelivered => Outcome::AlreadyDelivered,
        CancelTransition::AlreadyCancelled => Outcome::AlreadyCancelled,
        CancelTransition::NotCancellable { current } => Outcome::NotCancellable { current },
    }))
}

pub(super) fn normalize_name(name: &str) -> Result<String, String> {
    if name.trim().is_empty() {
        Ok(String::new())
    } else {
        crate::messaging_profile::normalize_name(name)
    }
}
