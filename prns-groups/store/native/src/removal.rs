use super::*;

enum NamespacePresence {
    Missing,
    Present,
}

fn validate(
    write: &redb::WriteTransaction,
    names: &TableNames,
    incarnation: StoreIncarnation,
    limits: NativeLimits,
) -> Result<NamespacePresence, NativeError> {
    let mut found = 0;
    for table in write.list_tables().map_err(db_error)? {
        found += usize::from(names.contains(table.name()));
    }
    match found {
        0 => return Ok(NamespacePresence::Missing),
        3 => {}
        _ => return Err(NativeError::MissingNamespace),
    }
    let metadata = write.open_table(names.metadata()).map_err(db_error)?;
    let values = write.open_table(names.values()).map_err(db_error)?;
    let receipts = write.open_table(names.receipts()).map_err(db_error)?;
    State::load(&metadata, incarnation, limits)?.validate(&values, &receipts)?;
    Ok(NamespacePresence::Present)
}

impl NativeStoreOwner<'_> {
    /// Check deletion eligibility without modifying any tables. A caller can
    /// preflight all owned namespaces before deleting any; each deletion still
    /// revalidates its own transaction. Fully absent namespaces are valid retries.
    pub fn validate_delete(
        &mut self,
        namespace: StoreNamespace,
        incarnation: StoreIncarnation,
        limits: NativeLimits,
    ) -> Result<(), NativeError> {
        let write = self.database.begin_write().map_err(db_error)?;
        validate(&write, &TableNames::new(namespace), incarnation, limits)?;
        Ok(())
    }

    /// Delete exactly one retired namespace after its caller has durably fenced
    /// enrollment and network work. The mutable owner lease excludes every live
    /// backend. A missing namespace is an idempotent retry; partial tables or a
    /// different incarnation/configuration are never treated as permission to
    /// delete. This removes logical data, not historical database page contents.
    pub fn delete(
        &mut self,
        namespace: StoreNamespace,
        incarnation: StoreIncarnation,
        limits: NativeLimits,
    ) -> Result<(), NativeError> {
        let names = TableNames::new(namespace);
        let mut write = self.database.begin_write().map_err(db_error)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(db_error)?;
        match validate(&write, &names, incarnation, limits)? {
            NamespacePresence::Missing => return Ok(()),
            NamespacePresence::Present => {}
        }
        write.delete_table(names.values()).map_err(db_error)?;
        write.delete_table(names.receipts()).map_err(db_error)?;
        write.delete_table(names.metadata()).map_err(db_error)?;
        write.commit().map_err(db_error)
    }
}
