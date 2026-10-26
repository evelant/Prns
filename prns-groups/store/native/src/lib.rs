//! Native durable group storage in the application's existing redb database.
//!
//! The database owner must create exactly one [`NativeStoreOwner`] and must not
//! write its reserved tables directly. A backend exclusively borrows that owner;
//! unrelated application tables remain available through the shared database.
//! This backend provides atomic durability, not encryption or secure erasure.
#![forbid(unsafe_code)]

mod enrollment;
mod metadata;
mod removal;
mod reserve;
pub use enrollment::EnrollmentSnapshot;
pub use reserve::NativeReserve;
#[cfg(test)]
mod tests;

use metadata::{State, TableNames};
use prns_groups_store::{
    AtomicBackend, CommitOutcome, CommitReceipt, MaintenanceBackend, MaintenanceOutcome, Mutation,
    OperationId, OperationRejection, OperationTicket, RecoveryOutcome, Revision, StoreIncarnation,
};
use redb::{Database, Durability, ReadableDatabase, TableHandle};
use std::{fmt, marker::PhantomData};

/// Stable application-selected namespace, distinct from a fresh store incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreNamespace(pub [u8; 16]);

/// Logical payload bounds. redb page/cache overhead and file size are separate
/// database-owner concerns; this is not a bound on resident memory or disk size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeLimits {
    pub max_changes: usize,
    pub max_key_bytes: usize,
    pub max_value_bytes: usize,
    pub max_staged_bytes: usize,
    pub max_live_objects: usize,
    pub max_live_bytes: usize,
    pub max_receipts: usize,
}

#[derive(Debug)]
pub enum NativeError {
    Database(redb::Error),
    AlreadyInitialized,
    MissingNamespace,
    Corrupt,
    Configuration,
    ForeignIncarnation,
    Capacity,
    Allocation,
    CounterExhausted,
}
impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native group storage error: {self:?}")
    }
}
impl std::error::Error for NativeError {}
fn db_error(error: impl Into<redb::Error>) -> NativeError {
    NativeError::Database(error.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeUsage {
    pub revision: Revision,
    pub retired_through: u64,
    pub live_objects: usize,
    pub live_bytes: usize,
    pub receipts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRequest {
    /// The trusted owner attests every recovery obligation through this ticket
    /// is discharged. This is not a network receipt or physical key erasure.
    FinalizeThrough(OperationTicket),
}

/// Construct once per database owner. This is an explicit ownership contract:
/// constructing a second owner or bypassing its tables violates the backend's
/// exclusive-writer requirement. No database, thread, runtime or global is created.
///
/// A second backend cannot borrow the same owner while the first is in use:
///
/// ```compile_fail
/// use prns_groups_store::{AtomicBackend, StoreIncarnation};
/// use prns_groups_store_native::{NativeLimits, NativeStoreOwner, StoreNamespace};
/// fn overlapping(db: &redb::Database, incarnation: StoreIncarnation, limits: NativeLimits) {
///     let mut owner = NativeStoreOwner::new(db);
///     let mut first = owner.open(StoreNamespace([1; 16]), incarnation, limits).unwrap();
///     let second = owner.open(StoreNamespace([2; 16]), incarnation, limits);
///     first.head().unwrap();
/// }
/// ```
pub struct NativeStoreOwner<'db> {
    database: &'db Database,
}
impl<'db> NativeStoreOwner<'db> {
    pub fn new(database: &'db Database) -> Self {
        Self { database }
    }

    /// Initialize only a namespace with no existing tables. Even empty orphan
    /// tables are evidence that must be inspected, never silently reformatted.
    pub fn initialize<'a>(
        &'a mut self,
        namespace: StoreNamespace,
        incarnation: StoreIncarnation,
        limits: NativeLimits,
    ) -> Result<NativeBackend<'a>, NativeError> {
        let names = TableNames::new(namespace);
        let state = State::initial(incarnation, limits)?;
        let mut write = self.database.begin_write().map_err(db_error)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(db_error)?;
        for table in write.list_tables().map_err(db_error)? {
            if names.contains(table.name()) {
                return Err(NativeError::AlreadyInitialized);
            }
        }
        {
            let mut metadata = write.open_table(names.metadata()).map_err(db_error)?;
            metadata
                .insert(0, state.encode()?.as_slice())
                .map_err(db_error)?;
            write.open_table(names.values()).map_err(db_error)?;
            write.open_table(names.receipts()).map_err(db_error)?;
        }
        write.commit().map_err(db_error)?;
        self.open(namespace, incarnation, limits)
    }

    /// Open existing state with the exact persisted limits and incarnation.
    /// Missing/corrupt evidence never becomes a newly initialized empty store.
    pub fn open<'a>(
        &'a mut self,
        namespace: StoreNamespace,
        incarnation: StoreIncarnation,
        limits: NativeLimits,
    ) -> Result<NativeBackend<'a>, NativeError> {
        let backend = NativeBackend {
            database: self.database,
            names: TableNames::new(namespace),
            incarnation,
            limits,
            admission: reserve::Admission::Full,
            lease: PhantomData,
        };
        backend.snapshot()?;
        Ok(backend)
    }
}

/// The lifetime is the exclusive namespace-owner lease, not database ownership.
pub struct NativeBackend<'owner> {
    database: &'owner Database,
    names: TableNames,
    incarnation: StoreIncarnation,
    limits: NativeLimits,
    admission: reserve::Admission,
    lease: PhantomData<&'owner mut ()>,
}
impl NativeBackend<'_> {
    fn require_tables(&self, write: &redb::WriteTransaction) -> Result<(), NativeError> {
        let mut found = 0;
        for table in write.list_tables().map_err(db_error)? {
            found += usize::from(self.names.contains(table.name()));
        }
        if found != 3 {
            return Err(NativeError::MissingNamespace);
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<State, NativeError> {
        let read = self.database.begin_read().map_err(db_error)?;
        let meta = read.open_table(self.names.metadata()).map_err(open_error)?;
        let values = read.open_table(self.names.values()).map_err(open_error)?;
        let receipts = read.open_table(self.names.receipts()).map_err(open_error)?;
        let state = State::load(&meta, self.incarnation, self.limits)?;
        state.validate(&values, &receipts)?;
        Ok(state)
    }
    pub fn usage(&self) -> Result<NativeUsage, NativeError> {
        Ok(self.snapshot()?.usage())
    }
    fn receipt(&self, ticket: OperationTicket) -> CommitReceipt {
        CommitReceipt {
            operation_id: ticket.operation_id(),
            revision: Revision(ticket.sequence()),
        }
    }
}
fn open_error(error: redb::TableError) -> NativeError {
    match error {
        redb::TableError::TableDoesNotExist(_) => NativeError::MissingNamespace,
        other => db_error(other),
    }
}
fn ticket(state: &State, operation: OperationId) -> Result<OperationTicket, OperationRejection> {
    let ticket = OperationTicket::from_operation_id(operation)
        .map_err(|_| OperationRejection::InvalidTicket)?;
    if ticket.incarnation() != state.incarnation {
        return Err(OperationRejection::ForeignIncarnation);
    }
    if ticket.sequence() > state.revision.0.saturating_add(1) {
        return Err(OperationRejection::OutOfOrder {
            next: state
                .revision
                .0
                .checked_add(1)
                .and_then(|seq| OperationTicket::new(state.incarnation, seq).ok()),
        });
    }
    Ok(ticket)
}

impl AtomicBackend for NativeBackend<'_> {
    type Error = NativeError;
    fn head(&mut self) -> Result<Revision, Self::Error> {
        Ok(self.snapshot()?.revision)
    }
    fn read(&mut self, key: &[u8], max_bytes: usize) -> Result<Option<Vec<u8>>, Self::Error> {
        if key.len() > self.limits.max_key_bytes {
            return Err(NativeError::Capacity);
        }
        self.snapshot()?;
        let read = self.database.begin_read().map_err(db_error)?;
        let values = read.open_table(self.names.values()).map_err(open_error)?;
        let Some(value) = values.get(key).map_err(db_error)? else {
            return Ok(None);
        };
        let bytes = value.value();
        if bytes.len() > max_bytes || bytes.len() > self.limits.max_value_bytes {
            return Err(NativeError::Capacity);
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(bytes.len())
            .map_err(|_| NativeError::Allocation)?;
        result.extend_from_slice(bytes);
        Ok(Some(result))
    }
    fn recover(&mut self, operation: OperationId) -> Result<RecoveryOutcome, Self::Error> {
        let state = self.snapshot()?;
        let ticket = match ticket(&state, operation) {
            Ok(t) => t,
            Err(r) => return Ok(RecoveryOutcome::Rejected(r)),
        };
        if ticket.sequence() <= state.floor {
            return Ok(RecoveryOutcome::Retired {
                revision: state.revision,
                retired_through: OperationTicket::new(self.incarnation, state.floor)
                    .map_err(|_| NativeError::Corrupt)?,
            });
        }
        if ticket.sequence() <= state.revision.0 {
            return Ok(RecoveryOutcome::Committed(self.receipt(ticket)));
        }
        if state.receipt_count()? == self.limits.max_receipts {
            return Ok(RecoveryOutcome::Rejected(
                OperationRejection::RecoveryWindowFull,
            ));
        }
        Ok(RecoveryOutcome::NotCommitted {
            revision: state.revision,
        })
    }
    fn commit(
        &mut self,
        expected: Revision,
        operation: OperationId,
        changes: &[Mutation],
    ) -> Result<CommitOutcome, Self::Error> {
        let mut write = self.database.begin_write().map_err(db_error)?;
        self.require_tables(&write)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(db_error)?;
        let receipt;
        {
            let mut meta = write.open_table(self.names.metadata()).map_err(db_error)?;
            let mut values = write.open_table(self.names.values()).map_err(db_error)?;
            let mut receipts = write.open_table(self.names.receipts()).map_err(db_error)?;
            let mut state = State::load(&meta, self.incarnation, self.limits)?;
            state.validate(&values, &receipts)?;
            let ticket = match ticket(&state, operation) {
                Ok(t) => t,
                Err(r) => return Ok(CommitOutcome::Rejected(r)),
            };
            if ticket.sequence() <= state.floor {
                return Ok(CommitOutcome::Rejected(OperationRejection::RetiredTicket));
            }
            if ticket.sequence() <= state.revision.0 {
                return Ok(CommitOutcome::Committed(self.receipt(ticket)));
            }
            if expected != state.revision {
                return Ok(CommitOutcome::NotCommitted {
                    revision: state.revision,
                });
            }
            if state.receipt_count()? == self.limits.max_receipts {
                return Ok(CommitOutcome::Rejected(
                    OperationRejection::RecoveryWindowFull,
                ));
            }
            if changes.len() > self.limits.max_changes {
                return Err(NativeError::Capacity);
            }
            let mut staged_bytes = 0usize;
            for change in changes {
                let value_bytes = change.value().map_or(0, <[u8]>::len);
                staged_bytes = staged_bytes
                    .checked_add(change.key().len())
                    .and_then(|n| n.checked_add(value_bytes))
                    .ok_or(NativeError::Capacity)?;
                if change.key().len() > self.limits.max_key_bytes
                    || value_bytes > self.limits.max_value_bytes
                    || staged_bytes > self.limits.max_staged_bytes
                {
                    return Err(NativeError::Capacity);
                }
                if let Some(old) = values.remove(change.key()).map_err(db_error)? {
                    state.objects = state.objects.checked_sub(1).ok_or(NativeError::Corrupt)?;
                    state.bytes = state
                        .bytes
                        .checked_sub(change.key().len() + old.value().len())
                        .ok_or(NativeError::Corrupt)?;
                }
                if let Some(value) = change.value() {
                    state.objects = state.objects.checked_add(1).ok_or(NativeError::Capacity)?;
                    state.bytes = state
                        .bytes
                        .checked_add(change.key().len())
                        .and_then(|n| n.checked_add(value.len()))
                        .ok_or(NativeError::Capacity)?;
                    values.insert(change.key(), value).map_err(db_error)?;
                }
            }
            if let Err(reason) = self.admission.check(
                state.objects,
                state.bytes,
                state
                    .receipt_count()?
                    .checked_add(1)
                    .ok_or(NativeError::Capacity)?,
            ) {
                return Ok(CommitOutcome::Rejected(reason));
            }
            if state.objects > self.limits.max_live_objects
                || state.bytes > self.limits.max_live_bytes
            {
                return Err(NativeError::Capacity);
            }
            state.revision = state.revision.next().ok_or(NativeError::CounterExhausted)?;
            receipt = self.receipt(ticket);
            receipts
                .insert(ticket.sequence(), state.revision.0)
                .map_err(db_error)?;
            meta.insert(0, state.encode()?.as_slice())
                .map_err(db_error)?;
        }
        #[cfg(test)]
        tests::crash_point("before-commit");
        write.commit().map_err(db_error)?;
        #[cfg(test)]
        tests::crash_point("after-commit");
        Ok(CommitOutcome::Committed(receipt))
    }
}

impl MaintenanceBackend for NativeBackend<'_> {
    type Request = NativeRequest;
    type Report = NativeUsage;
    fn maintain(
        &mut self,
        request: Self::Request,
    ) -> Result<MaintenanceOutcome<Self::Report>, Self::Error> {
        let mut write = self.database.begin_write().map_err(db_error)?;
        self.require_tables(&write)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(db_error)?;
        let usage;
        {
            let mut meta = write.open_table(self.names.metadata()).map_err(db_error)?;
            let values = write.open_table(self.names.values()).map_err(db_error)?;
            let mut receipts = write.open_table(self.names.receipts()).map_err(db_error)?;
            let mut state = State::load(&meta, self.incarnation, self.limits)?;
            state.validate(&values, &receipts)?;
            let NativeRequest::FinalizeThrough(ticket) = request;
            if ticket.incarnation() != self.incarnation
                || ticket.sequence() > state.revision.0
                || ticket.sequence() < state.floor
            {
                return Ok(MaintenanceOutcome::Rejected(state.usage()));
            }
            if ticket.sequence() == state.floor {
                return Ok(MaintenanceOutcome::Complete(state.usage()));
            }
            for seq in state.floor..ticket.sequence() {
                receipts.remove(seq + 1).map_err(db_error)?;
            }
            state.floor = ticket.sequence();
            meta.insert(0, state.encode()?.as_slice())
                .map_err(db_error)?;
            usage = state.usage();
        }
        #[cfg(test)]
        tests::crash_point("before-retirement");
        write.commit().map_err(db_error)?;
        #[cfg(test)]
        tests::crash_point("after-retirement");
        Ok(MaintenanceOutcome::Complete(usage))
    }
    fn recover_maintenance(&mut self) -> Result<MaintenanceOutcome<Self::Report>, Self::Error> {
        Ok(MaintenanceOutcome::Complete(self.snapshot()?.usage()))
    }
}
