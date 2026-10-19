use super::*;
use crate::contact_cards::{Failure, Reply as CardReply, Request};

pub async fn export(input: ExportContactCardInput) -> ContactCardExportOutcome {
    export_with_supervisor(supervisor(), input).await
}

pub async fn import(input: ImportContactCardInput) -> ContactCardImportOutcome {
    import_with_supervisor(supervisor(), input).await
}

pub(super) async fn export_with_supervisor(
    supervisor: &Supervisor,
    input: ExportContactCardInput,
) -> ContactCardExportOutcome {
    match admit(supervisor, Request::Export(input)) {
        Ok(receive) => match receive.await {
            Ok(CardReply::Export(outcome)) => outcome,
            _ => Failure::Unavailable(
                "The native owner stopped before returning the contact card.".into(),
            )
            .export(),
        },
        Err(failure) => failure.export(),
    }
}

pub(super) async fn import_with_supervisor(
    supervisor: &Supervisor,
    input: ImportContactCardInput,
) -> ContactCardImportOutcome {
    if let Err(outcome) = crate::contact_cards::check_import(&input) {
        return outcome;
    }
    match admit(supervisor, Request::Import(input)) {
        // Admitted imports remain owned through commit, even if the caller
        // disappears. A timeout must not report an ambiguous write as rejected.
        Ok(receive) => match receive.await {
            Ok(CardReply::Import(outcome)) => outcome,
            _ => Failure::Unavailable(
                "The native owner stopped before confirming contact import.".into(),
            )
            .import(),
        },
        Err(failure) => failure.import(),
    }
}

fn admit(
    supervisor: &Supervisor,
    request: Request,
) -> Result<oneshot::Receiver<CardReply>, Failure> {
    let state = admission::try_state(supervisor).map_err(|_| Failure::Busy)?;
    let snapshot = supervisor.snapshots.read();
    match snapshot.primary_identity {
        PrimaryIdentityState::Present { identity_hash } => {
            if identity_hash.as_slice() != request.expected_identity() {
                return Err(Failure::IdentityChanged);
            }
        }
        PrimaryIdentityState::Missing => return Err(Failure::MissingIdentity),
        PrimaryIdentityState::Unavailable { detail } => return Err(Failure::Unavailable(detail)),
        PrimaryIdentityState::DevelopmentResetRequired { reason } => {
            return Err(Failure::ResetRequired(reason))
        }
    }
    if matches!(
        snapshot.runtime,
        DevelopmentNodeRuntime::Starting | DevelopmentNodeRuntime::Stopping
    ) || supervisor.snapshots.is_explicit_stop_in_progress()
    {
        return Err(Failure::Busy);
    }
    let owner = state.application_owner.as_ref().ok_or_else(|| {
        Failure::Unavailable("Prepare native storage before using contact cards.".into())
    })?;
    if state
        .worker
        .as_ref()
        .is_some_and(|worker| worker.storage_root != owner.root)
    {
        return Err(Failure::Unavailable(
            "The native storage owner changed.".into(),
        ));
    }
    owner.admit_contact_card(request).map_err(Failure::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::foreign_block_on;

    fn fixture(root: &Path) -> (Supervisor, [u8; 16]) {
        let mut vault = FileVault::new(root.join("identities"));
        vault
            .store(&IdentityLabel::new("primary").unwrap(), &[17; 64])
            .unwrap();
        let identity = *PrivateIdentityMaterial::from_bytes([17; 64])
            .identity_hash()
            .as_bytes();
        let snapshots = Arc::new(SnapshotStore::new());
        snapshots.set_primary_identity(PrimaryIdentityState::Present {
            identity_hash: identity.to_vec(),
        });
        let owner = DevelopmentStoreOwner::open(root, &root.join("application.redb")).unwrap();
        (
            Supervisor {
                snapshots,
                operation_admitted: Arc::new(AtomicBool::new(false)),
                state: Mutex::new(SupervisorState {
                    application_owner: Some(owner),
                    ..SupervisorState::default()
                }),
            },
            identity,
        )
    }

    #[test]
    fn contact_cards_work_offline_without_a_node_or_tokio_runtime_and_fence_identity() {
        let temp = tempfile::tempdir().unwrap();
        let (supervisor, identity) = fixture(temp.path());
        let ContactCardExportOutcome::Ready { card } = foreign_block_on(export_with_supervisor(
            &supervisor,
            ExportContactCardInput {
                expected_local_identity: identity,
            },
        )) else {
            panic!("offline export failed")
        };
        let input = ImportContactCardInput {
            uri: card.uri,
            expected_local_identity: identity,
            alias: None,
        };
        assert!(matches!(
            foreign_block_on(import_with_supervisor(&supervisor, input.clone())),
            ContactCardImportOutcome::Saved { .. }
        ));
        let mut stale = input;
        stale.expected_local_identity = [42; 16];
        assert_eq!(
            foreign_block_on(import_with_supervisor(&supervisor, stale)),
            ContactCardImportOutcome::IdentityChanged
        );
        assert_eq!(
            supervisor.snapshots.read().runtime,
            DevelopmentNodeRuntime::Stopped
        );
        assert!(supervisor.lock_state().worker.is_none());
        supervisor
            .snapshots
            .set_runtime(DevelopmentNodeRuntime::Starting);
        assert_eq!(
            foreign_block_on(export_with_supervisor(
                &supervisor,
                ExportContactCardInput {
                    expected_local_identity: identity
                }
            )),
            ContactCardExportOutcome::Busy
        );
        supervisor
            .lock_state()
            .application_owner
            .take()
            .unwrap()
            .close()
            .unwrap();
    }

    #[test]
    fn card_admission_does_not_reopen_storage_after_owner_is_closed() {
        let temp = tempfile::tempdir().unwrap();
        let (supervisor, identity) = fixture(temp.path());
        supervisor
            .lock_state()
            .application_owner
            .take()
            .unwrap()
            .close()
            .unwrap();
        std::fs::remove_dir_all(temp.path().join("identities")).unwrap();
        assert!(matches!(
            foreign_block_on(export_with_supervisor(
                &supervisor,
                ExportContactCardInput {
                    expected_local_identity: identity
                }
            )),
            ContactCardExportOutcome::Unavailable { .. }
        ));
        assert!(!temp.path().join("identities").exists());
    }
}
