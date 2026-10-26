//! Change waits belong to one immutable service generation, never UI selection.
use super::*;
use prns_lxmf::mailbox::{DurableDirectLxmfService, MailboxListRequest};

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl NamedIdentityClient {
    /// Read the initial projection, or wait until this owner's LXMF state changes.
    /// Cancel the wait when its screen, identity or host generation is replaced.
    /// A stopped projection or `None` ends this subscription; resume with no cursor
    /// after the identity becomes available again.
    pub async fn wait_lxmf_change(
        &self,
        after_projection_revision: Option<u64>,
    ) -> Option<LxmfHealth> {
        self.wait_lxmf_change_with_owner(global().ok()?, after_projection_revision)
            .await
    }
}

impl NamedIdentityClient {
    pub(super) async fn wait_lxmf_change_with_owner(
        &self,
        owner: Arc<NamedRuntime>,
        after_projection_revision: Option<u64>,
    ) -> Option<LxmfHealth> {
        // The admitted job only lends the service handle. An idle change wait
        // must not hold its identity's service mutex or delay pause and sends.
        let service = self
            .call_with_owner(&owner, core::AdmissionClass::History, |services| {
                Box::pin(async move { services.direct().map_err(detail) })
            })
            .await
            .ok()?;
        wait_for_service_change(&service, after_projection_revision).await
    }
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;

async fn wait_for_service_change(
    service: &DurableDirectLxmfService,
    after_projection_revision: Option<u64>,
) -> Option<LxmfHealth> {
    // Subscribe before reading: a completion between the read and changed()
    // remains pending in the receiver rather than stranding the conversation.
    let mut changes = service.subscribe();
    loop {
        let snapshot = service
            .snapshot(MailboxListRequest {
                peer: None,
                direction: None,
                before: None,
                limit: 1,
            })
            .await
            .ok()?;
        let health = crate::lxmf::project_health(&snapshot).ok()?;
        if health.state == LxmfHealthState::Stopped
            || after_projection_revision != Some(health.projection_revision)
        {
            return Some(health);
        }
        changes.changed().await.ok()?;
    }
}
