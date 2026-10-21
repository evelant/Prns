use super::*;
use personal_rns::remote_control::{
    RemoteControlControllerCard, RemoteControlControllerIdentity, RemoteControlTargetAccessCard,
    SetRemoteControlTargetAccessOutcome,
};
use personal_rns::runtime::RemoteControlTargetInventoryControlError as IdentityError;

pub fn preview(text: String) -> TargetAccessCardPreviewOutcome {
    match RemoteControlTargetAccessCard::decode_text(&text) {
        Ok(card) => TargetAccessCardPreviewOutcome::Valid {
            preview: TargetAccessCardPreview {
                target_identity_fingerprint: *card.access().target().identity_hash().as_bytes(),
                recipient_controller_fingerprint: *card.recipient().identity_hash().as_bytes(),
                authority: match card.access().authority() {
                    personal_rns::remote_control::RemoteControlControllerAuthority::Operator =>
                        RemoteControlControllerAuthority::Operator,
                    personal_rns::remote_control::RemoteControlControllerAuthority::Administrator =>
                        RemoteControlControllerAuthority::Administrator,
                },
                permissions: crate::pairing::request_kinds(&card.access().effective_requests()),
            },
        },
        Err(error) => TargetAccessCardPreviewOutcome::Invalid { detail: format!("Invalid target access card: {error:?}") },
    }
}

pub async fn export() -> ControllerSetupCardOutcome {
    let admitted = (|| {
        let supervisor = supervisor();
        let state =
            admission::try_state(supervisor).map_err(|_| ControllerSetupCardOutcome::Busy)?;
        let worker = admission::running_worker(supervisor, &state)
            .map_err(|_| ControllerSetupCardOutcome::LocalNodeStopped)?;
        let (response, receive) = oneshot::channel();
        worker
            .commands
            .try_send(Command::ExportControllerCard(response))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ControllerSetupCardOutcome::Busy,
                mpsc::error::TrySendError::Closed(_) => {
                    ControllerSetupCardOutcome::LocalNodeStopped
                }
            })?;
        Ok(receive)
    })();
    match admitted {
        Ok(receive) => admission::bounded_reply(receive, COMMAND_TIMEOUT)
            .await
            .unwrap_or_else(|detail| ControllerSetupCardOutcome::Unavailable {
                detail: detail.to_owned(),
            }),
        Err(outcome) => outcome,
    }
}

pub async fn import(input: ImportTargetAccessCardInput) -> TargetAccessCardImportOutcome {
    if !input.trusted_source_confirmed {
        return TargetAccessCardImportOutcome::TrustRequired;
    }
    if let Err(error) = RemoteControlTargetAccessCard::decode_text(&input.card) {
        return TargetAccessCardImportOutcome::Invalid {
            detail: format!("Invalid target access card: {error:?}"),
        };
    }
    let supervisor = supervisor();
    let admitted = (|| {
        let state =
            admission::try_state(supervisor).map_err(|_| TargetAccessCardImportOutcome::Busy)?;
        let worker = admission::running_worker(supervisor, &state)
            .map_err(|_| TargetAccessCardImportOutcome::LocalNodeStopped)?;
        if supervisor
            .operation_admitted
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TargetAccessCardImportOutcome::Busy);
        }
        let (response, receive) = oneshot::channel();
        if let Err(error) = worker
            .commands
            .try_send(Command::ImportTargetAccessCard(input, response))
        {
            supervisor
                .operation_admitted
                .store(false, Ordering::Release);
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => TargetAccessCardImportOutcome::Busy,
                mpsc::error::TrySendError::Closed(_) => {
                    TargetAccessCardImportOutcome::LocalNodeStopped
                }
            });
        }
        Ok(receive)
    })();
    match admitted {
        // Caller departure never cancels an accepted durable import in the actor.
        Ok(receive) => admission::bounded_reply(receive, COMMAND_TIMEOUT)
            .await
            .unwrap_or(TargetAccessCardImportOutcome::OutcomeUnknown),
        Err(outcome) => outcome,
    }
}

pub(super) async fn export_with_handle(handle: &PrnsNodeHandle) -> ControllerSetupCardOutcome {
    match handle.remote_control_controller_identity().await {
        Ok(controller) => controller_card(controller),
        Err(IdentityError::Busy) => ControllerSetupCardOutcome::Busy,
        Err(IdentityError::NodeStopped) => ControllerSetupCardOutcome::LocalNodeStopped,
        Err(error) => ControllerSetupCardOutcome::Unavailable {
            detail: format!("Controller identity unavailable: {error:?}"),
        },
    }
}

fn controller_card(controller: RemoteControlControllerIdentity) -> ControllerSetupCardOutcome {
    let encoded = (|| {
        let card = RemoteControlControllerCard::new(controller)?;
        let mut buffer = [0; RemoteControlControllerCard::MAX_TEXT_LEN];
        let count = card.encode_text(&mut buffer)?;
        Ok::<_, personal_rns::remote_control::RemoteControlPublicCardError>(
            String::from_utf8_lossy(&buffer[..count]).into_owned(),
        )
    })();
    match encoded {
        Ok(text) => ControllerSetupCardOutcome::Ready {
            card: ControllerSetupCard {
                text,
                public_key_hex: crate::pairing::bytes_hex(
                    &controller.public_keys().public_key_bytes(),
                ),
                controller_identity_fingerprint: *controller.identity_hash().as_bytes(),
            },
        },
        Err(error) => ControllerSetupCardOutcome::Unavailable {
            detail: format!("Controller card unavailable: {error:?}"),
        },
    }
}

pub(super) async fn import_with_handle(
    handle: &PrnsNodeHandle,
    snapshots: &SnapshotStore,
    input: ImportTargetAccessCardInput,
) -> TargetAccessCardImportOutcome {
    use personal_rns::runtime::SaveRemoteControlTargetAccessError as SaveError;
    if !input.trusted_source_confirmed {
        return TargetAccessCardImportOutcome::TrustRequired;
    }
    let card = match RemoteControlTargetAccessCard::decode_text(&input.card) {
        Ok(card) => card,
        Err(error) => {
            return TargetAccessCardImportOutcome::Invalid {
                detail: format!("Invalid target access card: {error:?}"),
            }
        }
    };
    let controller = match handle.remote_control_controller_identity().await {
        Ok(controller) => controller,
        // No save has been submitted while obtaining the identity.
        Err(IdentityError::Busy) => return TargetAccessCardImportOutcome::Busy,
        Err(IdentityError::NodeStopped) => return TargetAccessCardImportOutcome::LocalNodeStopped,
        Err(error) => {
            return TargetAccessCardImportOutcome::Unavailable {
                detail: format!("Controller identity unavailable: {error:?}"),
            }
        }
    };
    if controller.identity_hash().as_bytes() != &input.expected_controller_identity {
        return TargetAccessCardImportOutcome::ControllerChanged;
    }
    let target_identity_fingerprint = *card.access().target().identity_hash().as_bytes();
    match handle.save_remote_control_target_access_card(card).await {
        Ok(outcome) => {
            let _ = crate::remote_control::refresh_targets(handle, snapshots).await;
            match outcome {
                SetRemoteControlTargetAccessOutcome::Unchanged => {
                    TargetAccessCardImportOutcome::Existing {
                        target_identity_fingerprint,
                    }
                }
                _ => TargetAccessCardImportOutcome::Saved {
                    target_identity_fingerprint,
                },
            }
        }
        Err(SaveError::Busy) => TargetAccessCardImportOutcome::Busy,
        Err(SaveError::NodeStopped) => TargetAccessCardImportOutcome::OutcomeUnknown,
        Err(SaveError::Unavailable) => TargetAccessCardImportOutcome::Unavailable {
            detail: "Remote control is unavailable.".into(),
        },
        Err(SaveError::CapacityExhausted) => TargetAccessCardImportOutcome::CapacityExhausted,
        Err(SaveError::PersistenceUnavailable) => {
            TargetAccessCardImportOutcome::PersistenceUnavailable
        }
        Err(SaveError::PersistenceFailed) => TargetAccessCardImportOutcome::PersistenceFailed,
        Err(SaveError::RecipientMismatch) => TargetAccessCardImportOutcome::RecipientMismatch,
        Err(SaveError::Conflict) => TargetAccessCardImportOutcome::Conflict,
    }
}
