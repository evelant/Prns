//! Bounded app create-result receipts survive pruning the core lifecycle journal.
//! Publication happens before advancing the core's completed floor.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateReceipt {
    request: [u8; 32],
    label: String,
    published: String,
    incarnation: [u8; 16],
    sequence: u64,
    hash: [u8; 16],
}
impl Actor<'_> {
    pub(super) fn read_receipts(&mut self) -> Result<(), String> {
        let path = self.disk.root().join("create-receipts.json");
        if !path.try_exists().map_err(detail)? {
            return Ok(());
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(detail)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16_384 {
            return Err("Invalid identity create receipts.".into());
        }
        let receipts: Vec<CreateReceipt> =
            serde_json::from_slice(&std::fs::read(path).map_err(detail)?).map_err(detail)?;
        if receipts.len() > MAX_IDENTITIES {
            return Err("Identity receipt capacity exceeded.".into());
        }
        for (index, receipt) in receipts.iter().enumerate() {
            self.check(&receipt.reference())?;
            if receipts[..index]
                .iter()
                .any(|r| r.request == receipt.request || r.sequence == receipt.sequence)
            {
                return Err("Duplicate identity create receipt.".into());
            }
        }
        self.create_receipts = receipts;
        Ok(())
    }
    pub(super) fn created_before(&self, input: &CreateNamedIdentityInput) -> Result<bool, String> {
        let Some(receipt) = self
            .create_receipts
            .iter()
            .find(|r| r.request == input.request_id)
        else {
            return Ok(false);
        };
        if receipt.label != input.label.trim()
            || receipt.published != input.published_name.as_deref().unwrap_or("")
        {
            return Err("This create request already belongs to another identity.".into());
        }
        self.check(&receipt.reference())?;
        Ok(true)
    }
    pub(super) fn retire_operations(&mut self) -> Result<(), String> {
        let mut changed = false;
        for operation in self.coordinator.snapshot().operations() {
            if let (
                core::Command::Create(request),
                core::OperationState::Completed(
                    core::OperationResult::Created { identity }
                    | core::OperationResult::Existing { identity },
                ),
            ) = (operation.command, operation.state)
            {
                if self
                    .create_receipts
                    .iter()
                    .any(|r| r.request == operation.token.digest.0)
                {
                    continue;
                }
                if self.create_receipts.len() == MAX_IDENTITIES {
                    return Err("Identity create receipt capacity exceeded.".into());
                }
                let record = self.coordinator.identity(identity).map_err(detail)?;
                self.create_receipts.push(CreateReceipt {
                    request: operation.token.digest.0,
                    label: request.metadata.local_label.as_str().into(),
                    published: request.metadata.published_name.as_str().into(),
                    incarnation: identity.incarnation,
                    sequence: identity.sequence,
                    hash: record.public_hash.0,
                });
                changed = true;
            }
        }
        // Re-write even if a prior uncertain receipt publication updated memory.
        // A durable core floor must never outrun its durable retry evidence.
        if changed || !self.create_receipts.is_empty() {
            replace_file(
                self.disk.root(),
                "create-receipts.json",
                &serde_json::to_vec(&self.create_receipts).map_err(detail)?,
            )?;
        }
        let through = self
            .coordinator
            .snapshot()
            .operations()
            .filter(|o| !matches!(o.state, core::OperationState::Completed(_)))
            .map(|o| o.token.sequence.saturating_sub(1))
            .min()
            .unwrap_or(self.coordinator.snapshot().issued_sequence);
        self.coordinator
            .prune_completed(&mut self.disk, through)
            .map_err(detail)
    }
}
impl CreateReceipt {
    fn reference(&self) -> NamedIdentityReference {
        NamedIdentityReference {
            incarnation: self.incarnation,
            sequence: self.sequence,
            public_hash: self.hash,
        }
    }
}
pub(super) fn replace_file(root: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let stage = root.join(format!("{name}.next"));
    match std::fs::symlink_metadata(&stage) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
            return Err("Unsafe identity state staging path.".into())
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(detail(e)),
        _ => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&stage).map_err(detail)?;
    file.write_all(bytes).map_err(detail)?;
    file.sync_all().map_err(detail)?;
    std::fs::rename(stage, root.join(name)).map_err(detail)?;
    std::fs::File::open(root)
        .and_then(|f| f.sync_all())
        .map_err(detail)
}
