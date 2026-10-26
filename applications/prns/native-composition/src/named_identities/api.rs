use super::*;
use crate::directory::DirectoryRequest;
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl NamedIdentityClient {
    pub async fn list_group_conversations(&self) -> GroupConversationListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { Ok(groups::conversations(services).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupConversationListOutcome::Unavailable { detail })
    }
    pub async fn list_group_messages(
        &self,
        input: ListGroupMessagesInput,
    ) -> GroupMessageListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { Ok(groups::list_messages(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupMessageListOutcome::Unavailable { detail })
    }
    pub async fn send_group_text(&self, input: SendGroupTextInput) -> GroupTextSendOutcome {
        self.call(core::AdmissionClass::ServiceMutation, move |services| {
            Box::pin(async move { Ok(groups::send_text(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupTextSendOutcome::Unavailable { detail })
    }
    pub async fn create_group(&self, input: CreateGroupInput) -> CreateGroupOutcome {
        self.call(core::AdmissionClass::ServiceMutation, move |services| {
            Box::pin(async move { Ok(groups::create_group(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| CreateGroupOutcome::Unavailable { detail })
    }
    pub async fn invite_to_group(&self, input: InviteToGroupInput) -> InviteToGroupOutcome {
        self.call(core::AdmissionClass::ServiceMutation, move |services| {
            Box::pin(async move { Ok(groups::invite_to_group(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| InviteToGroupOutcome::Unavailable { detail })
    }
    pub async fn respond_group_invitation(
        &self,
        input: RespondGroupInvitationInput,
    ) -> RespondGroupInvitationOutcome {
        self.call(core::AdmissionClass::ServiceMutation, move |services| {
            Box::pin(async move { Ok(groups::respond_group_invitation(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| RespondGroupInvitationOutcome::Unavailable { detail })
    }
    pub async fn list_group_invitations(&self) -> GroupInvitationListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { Ok(groups::list_group_invitations(services).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupInvitationListOutcome::Unavailable { detail })
    }
    pub async fn prepare_group_action(&self, input: PrepareGroupActionInput) -> GroupActionOutcome {
        let admission = if crate::group_actions::is_local_mutation(&input.input) {
            core::AdmissionClass::LocalMutation
        } else {
            core::AdmissionClass::ServiceMutation
        };
        self.call(admission, move |services| {
            Box::pin(async move { Ok(actions::prepare(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupActionOutcome::Unavailable { detail })
    }
    pub async fn list_group_actions(&self) -> GroupActionListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { Ok(actions::list(services).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupActionListOutcome::Unavailable { detail })
    }
    pub async fn execute_group_action(&self, input: GroupActionReference) -> GroupActionOutcome {
        let reference = input.clone();
        let previous = self
            .call(core::AdmissionClass::Recovery, move |services| {
                Box::pin(async move { Ok(actions::read(services, reference).await) })
            })
            .await
            .unwrap_or_else(|detail| GroupActionOutcome::Unavailable { detail });
        if !matches!(&previous,GroupActionOutcome::Ready{action} if matches!(action.state,GroupActionState::Pending))
        {
            return previous;
        }
        let admission = match &previous {
            GroupActionOutcome::Ready { action }
                if crate::group_actions::is_local_mutation(&action.input) =>
            {
                core::AdmissionClass::LocalMutation
            }
            _ => core::AdmissionClass::ServiceMutation,
        };
        // Recheck lifecycle admission after reading the immutable journal input.
        self.call(admission, move |services| {
            Box::pin(async move { Ok(actions::execute(services, input).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupActionOutcome::Unavailable { detail })
    }
    pub async fn acknowledge_group_action(
        &self,
        input: GroupActionReference,
    ) -> GroupActionAcknowledgeOutcome {
        self.call(core::AdmissionClass::Recovery, move |services| {
            Box::pin(async move { Ok(actions::acknowledge(services, input, false).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupActionAcknowledgeOutcome::Unavailable { detail })
    }
    pub async fn discard_expired_group_action(
        &self,
        input: GroupActionReference,
    ) -> GroupActionAcknowledgeOutcome {
        self.call(core::AdmissionClass::Recovery, move |services| {
            Box::pin(async move { Ok(actions::acknowledge(services, input, true).await) })
        })
        .await
        .unwrap_or_else(|detail| GroupActionAcknowledgeOutcome::Unavailable { detail })
    }
    pub async fn create_manual_contact(
        &self,
        input: CreateManualContactInput,
    ) -> ContactMutationOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move {
                direct::mutation(
                    services,
                    DirectoryRequest::CreateManual {
                        destination: input.destination,
                        identity: input.identity,
                        alias: input.alias,
                    },
                )
                .await
            })
        })
        .await
        .unwrap_or_else(|detail| ContactMutationOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn set_contact_alias(&self, input: SetContactAliasInput) -> ContactMutationOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move {
                direct::mutation(
                    services,
                    DirectoryRequest::SetAlias {
                        destination: input.destination,
                        alias: input.alias,
                    },
                )
                .await
            })
        })
        .await
        .unwrap_or_else(|detail| ContactMutationOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn set_contact_pinned(&self, input: SetContactPinnedInput) -> ContactMutationOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move {
                direct::mutation(
                    services,
                    DirectoryRequest::SetPinned {
                        destination: input.destination,
                        pinned: input.pinned,
                    },
                )
                .await
            })
        })
        .await
        .unwrap_or_else(|detail| ContactMutationOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn delete_contact(&self, input: ContactDestinationInput) -> ContactMutationOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move {
                direct::mutation(
                    services,
                    DirectoryRequest::Delete {
                        destination: input.destination,
                    },
                )
                .await
            })
        })
        .await
        .unwrap_or_else(|detail| ContactMutationOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn save_observed_destination(
        &self,
        input: ContactDestinationInput,
    ) -> ContactMutationOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move { direct::observed(services, input, false).await })
        })
        .await
        .unwrap_or_else(|detail| ContactMutationOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn save_discovered_contact(
        &self,
        input: ContactDestinationInput,
    ) -> ContactMutationOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move { direct::observed(services, input, true).await })
        })
        .await
        .unwrap_or_else(|detail| ContactMutationOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn get_contact(&self, input: ContactDestinationInput) -> ContactLookupOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { direct::get(services, input).await })
        })
        .await
        .unwrap_or_else(|detail| ContactLookupOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn list_contacts(&self) -> ContactListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { direct::contacts(services).await })
        })
        .await
        .unwrap_or_else(|detail| ContactListOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn list_lxmf_messages(&self, input: ListLxmfMessagesInput) -> LxmfMessageListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { direct::messages(services, input).await })
        })
        .await
        .unwrap_or_else(|detail| LxmfMessageListOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn list_lxmf_conversations(
        &self,
        input: ListLxmfConversationsInput,
    ) -> LxmfMessageListOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { direct::conversations(services, input).await })
        })
        .await
        .unwrap_or_else(|detail| LxmfMessageListOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn send_direct_text(&self, input: SendDirectTextInput) -> SendDirectTextOutcome {
        self.call(core::AdmissionClass::Network, move |services| {
            Box::pin(async move { direct::send(services, input).await })
        })
        .await
        .unwrap_or_else(|detail| SendDirectTextOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn retry_lxmf_message(
        &self,
        input: RetryLxmfMessageInput,
    ) -> RetryLxmfMessageOutcome {
        self.call(core::AdmissionClass::Network, move |services| {
            Box::pin(async move {
                Ok(crate::lxmf::project_retry_outcome(
                    services
                        .direct()
                        .map_err(detail)?
                        .retry_lxmf_message(input.local_record_id)
                        .await,
                ))
            })
        })
        .await
        .unwrap_or_else(|detail| RetryLxmfMessageOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn cancel_lxmf_message(
        &self,
        input: CancelLxmfMessageInput,
    ) -> CancelLxmfMessageOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move { direct::cancel(services, input).await })
        })
        .await
        .unwrap_or_else(|detail| CancelLxmfMessageOutcome::DevelopmentUnavailable { detail })
    }
    pub async fn read_messaging_profile(&self) -> LocalMessagingProfileOutcome {
        self.call(core::AdmissionClass::History, move |services| {
            Box::pin(async move { direct::profile(services, None).await })
        })
        .await
        .unwrap_or_else(|detail| LocalMessagingProfileOutcome::Unavailable { detail })
    }
    pub async fn set_messaging_name(&self, name: String) -> LocalMessagingProfileOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move { direct::profile(services, Some(name)).await })
        })
        .await
        .unwrap_or_else(|detail| LocalMessagingProfileOutcome::Unavailable { detail })
    }
    pub async fn export_contact_card(
        &self,
        input: ExportContactCardInput,
    ) -> ContactCardExportOutcome {
        self.call(core::AdmissionClass::OfflineSigning, move |services| {
            Box::pin(async move {
                if input.expected_local_identity != services.scope().public_hash.0 {
                    return Ok(ContactCardExportOutcome::IdentityChanged);
                }
                match services
                    .store()
                    .map_err(detail)?
                    .admit_contact_card(crate::contact_cards::Request::Export(input))
                    .map_err(detail)?
                    .await
                    .map_err(detail)?
                {
                    crate::contact_cards::Reply::Export(outcome) => Ok(outcome),
                    _ => Err("Unexpected contact card result.".into()),
                }
            })
        })
        .await
        .unwrap_or_else(|detail| ContactCardExportOutcome::Unavailable { detail })
    }
    pub async fn import_contact_card(
        &self,
        input: ImportContactCardInput,
    ) -> ContactCardImportOutcome {
        self.call(core::AdmissionClass::LocalMutation, move |services| {
            Box::pin(async move {
                if input.expected_local_identity != services.scope().public_hash.0 {
                    return Ok(ContactCardImportOutcome::IdentityChanged);
                }
                if let Err(outcome) = crate::contact_cards::check_import(&input) {
                    return Ok(outcome);
                }
                match services
                    .store()
                    .map_err(detail)?
                    .admit_contact_card(crate::contact_cards::Request::Import(input))
                    .map_err(detail)?
                    .await
                    .map_err(detail)?
                {
                    crate::contact_cards::Reply::Import(outcome) => Ok(outcome),
                    _ => Err("Unexpected contact card result.".into()),
                }
            })
        })
        .await
        .unwrap_or_else(|detail| ContactCardImportOutcome::Unavailable { detail })
    }
    pub async fn list_lxmf_peers(&self) -> LxmfPeerListOutcome {
        self.call(core::AdmissionClass::History, |services| {
            Box::pin(async move {
                let snapshot = services
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
                Ok(crate::lxmf::project_peers(&snapshot, direct::now()))
            })
        })
        .await
        .unwrap_or(LxmfPeerListOutcome::LocalNodeStopped)
    }
    pub async fn clear_lxmf_discovery(&self) -> LxmfDiscoveryClearOutcome {
        self.call(core::AdmissionClass::LocalMutation, |services| {
            Box::pin(async move {
                services
                    .direct()
                    .map_err(detail)?
                    .clear_discovered_peers()
                    .await;
                Ok(LxmfDiscoveryClearOutcome::Cleared)
            })
        })
        .await
        .unwrap_or(LxmfDiscoveryClearOutcome::LocalNodeStopped)
    }
    pub async fn measure_lxmf_text(&self, input: MeasureLxmfTextInput) -> MeasureLxmfTextOutcome {
        self.call(core::AdmissionClass::History, move |_| {
            Box::pin(async move { Ok(crate::lxmf::measure_text(&input)) })
        })
        .await
        .unwrap_or(MeasureLxmfTextOutcome::LocalNodeStopped)
    }
}
