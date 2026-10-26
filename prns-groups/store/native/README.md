# Native group storage

`prns-groups-store-native` implements the group coordinator's `AtomicBackend`
contract in the application's existing **redb 4.2.0** database. It is a native
development and conformance backend. It does not encrypt stored MLS secrets or
provide physical key erasure; it is not yet a production phone secret store.

The app or headless consumer owns the `Database`, its cache configuration and
its serialized work lane. Create one `NativeStoreOwner` from that database, then
explicitly initialize or open a `StoreNamespace`. A backend exclusively borrows
that owner. The application's unrelated tables remain usable through the same
database reference. This crate creates no database, executor, thread or runtime.
Creating duplicate owners for the same database, or writing reserved group
tables outside the backend, violates the exclusive-writer contract.

```rust
use prns_groups_store::{Coordinator, Limits, StoreIncarnation};
use prns_groups_store_native::{NativeLimits, NativeStoreOwner, StoreNamespace};

fn open_group_store<'a>(
    owner: &'a mut NativeStoreOwner<'_>,
    namespace: StoreNamespace,
    incarnation: StoreIncarnation,
    limits: NativeLimits,
) -> Result<Coordinator<prns_groups_store_native::NativeBackend<'a>>, Box<dyn std::error::Error>> {
    let backend = owner.open(namespace, incarnation, limits)?;
    Ok(Coordinator::new(backend, Limits {
        max_changes: limits.max_changes,
        max_key_bytes: limits.max_key_bytes,
        max_value_bytes: limits.max_value_bytes,
        max_staged_bytes: limits.max_staged_bytes,
    })?)
}
```

Initialization is a separate `owner.initialize(namespace, incarnation, limits)`
operation. The caller supplies a fresh CSPRNG-generated incarnation and retains
the selected namespace/configuration in its account lifecycle. Initialization
refuses any existing namespace table, including orphaned empty tables. Opening
requires the exact persisted incarnation and limits. There is no implicit reset,
replacement incarnation, schema upgrade or format migration.

## Durable contract

Each immediate-durability redb transaction publishes all KV mutations, revision
and receipt together. Crypto state, exact outgoing ciphertext and application
records therefore share one durable boundary. Replaying a committed ticket only
returns its original receipt, even if supplied different mutations. Tickets are
scoped to the store incarnation and admitted in sequence. Definitely rejected or stale attempts
do not consume a ticket.

Every open, recovery and operation validates metadata, value counts and lengths,
and the contiguous retained receipt range. Missing or inconsistent evidence fails
closed. Value length is checked against the caller's read cap before allocating
the returned buffer. Changes, staged payload bytes, live objects, live payload
bytes and retained receipt counts have explicit persisted bounds. Live-capacity
checks apply to the final transaction contents, so a replacement at capacity can
insert and delete in either order. A rejected batch leaves every prior value and
receipt unchanged.

Any commit error is conservatively uncertain at the coordinator boundary. Call
`Coordinator::recover` before doing more work; if redb itself requires reopening,
close the owner and reopen the same database, namespace and incarnation. Never
regenerate the ciphertext or allocate a replacement ticket to resolve an unknown
commit.

`NativeRequest::FinalizeThrough(ticket)` is the trusted caller's attestation that
all recovery obligations through that ticket have finished. One transaction
deletes those local receipts and advances a durable retired prefix without
changing application values or revision. Retired tickets remain consumed after
reopening. This is neither a remote delivery receipt nor erasure of old MLS keys.
Recovery after an uncertain finalization returns the verified persisted prefix;
the caller may retry the same requested prefix if it did not advance.

## One-time development enrollment

`export_enrollment` captures validated namespace state and receipt evidence for an
explicit pre-enrolled development fixture. `import_enrollment` validates limits,
incarnation, all values and receipt continuity inside one immediate transaction,
and refuses any existing table in the target namespace. An invalid import leaves
no partial namespace. Unrelated application tables remain intact. The native
runner additionally binds the fixture configuration and local Reticulum identity,
and opens current state on subsequent starts instead of importing again.

These plaintext snapshots contain initial MLS secrets and are not production
backup/restore APIs. Installing the same endpoint snapshot twice forks its state;
retaining it also retains initial ratchet secrets. See the
development enrollment guide (separate integration).

## Limits and remaining work

The bounds cover logical payloads, not redb pages, cache, transient transaction
copies or total file size. The database owner must choose cache and disk budgets,
handle disk-full errors and schedule shared-file reclamation. Each operation scans the bounded live-value and receipt tables to validate
evidence; this cost grows with retained records and must be measured before
choosing large limits. The adapter does
not compact or replace the application's database. Open read transactions and
backups can retain old pages; removing a value does not scrub these copies.

Plaintext MLS keys can remain in database pages, cache, filesystem snapshots and
backups. Before deployment with real secrets, qualify a platform wrapping-key and
record-encryption adapter, locked-device behavior, key retirement, backup/restore
and account-reset policies. Neither a store incarnation nor redb detects rollback
to an earlier complete valid backup. Host tests do not qualify iOS/Android file
protection, abrupt electrical power loss or filesystem/device durability.

## Verification

The host suite reopens actual redb files, checks atomic crypto/outbox/projection
state, exact retries, ordered tickets, receipt retirement, capacity, shared app
tables, missing/corrupt evidence and lost commit responses. A parent process
kills a child immediately before and after the actual commit and receipt
retirement commits, then reopens the database and checks all-or-none state and
receipt fences. These four barriers exist only in the unit-test binary. They
test process death, not interruption inside `fsync` or a physical power cut.

The ignored `crash_child` test is a subprocess helper; the normal parent test
invokes it four times. A compile-fail doctest checks the exclusive owner borrow.
