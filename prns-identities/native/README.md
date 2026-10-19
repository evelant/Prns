# Native identity catalog and vault

`prns-identities-native` implements the bounded core's `Storage` contract on a
private local directory and publishes standard Reticulum identity key material.
It is a `std` adapter; the generic lifecycle core and runtime remain `no_std` and
allocation-free. It does not migrate or touch an existing application's data.

`announces::NativeAnnounces<N>` separately supplies an exclusive filesystem owner
for the portable [shared scheduler](../runtime/README.md#shared-announce-scheduling).
Create one dedicated directory per node, shared across its identities and
services. `create_new` and `open` are explicit, with no corrupt-state fallback.
The versioned, capacity-bound snapshot uses private permissions, an OS lock,
checksums, file sync, atomic rename and directory sync. Attempt reservation is
durable before the adapter returns a dispatch token. Ordinary timer ticks do
not write; settings, reservations and completion outcomes do. The adapter bounds
slots to 256; the caller must budget its typed snapshot and runtime memory.

Host tests kill a subprocess at each reservation publication boundary, then
verify old-or-new complete state, a fresh restart cooldown and no replay of the
uncertain attempt. Other tests retain partial outcomes and reject duplicate
owners, missing/corrupt data and mismatched capacity. This metadata is not message
custody, and a queued manual request is not durable delivery intent.

## Ownership and persistence

Use `NativeStore::<I, O, S>::create_new(path)` once for a **new dedicated directory**.
An existing directory is an error. Use `open(path)` for every subsequent start;
missing, corrupt, incompatible, or incomplete state fails without initialization
or rollback. Paths are resolved to an absolute location before retaining them.
An interrupted initial creation with no valid catalog requires explicit operator
recovery; calling `create_new` again never destroys that directory.

Keep the store alive for the entire coordinator **and runtime owner** lifetime.
Its OS advisory lock excludes another cooperative process/store using the same
root. Do not unlink/replace the lock file or let another adapter access these
files. A second in-memory coordinator advances the durable host session; key and
runtime effects must validate against that latest durable owner. This is not a
defence against a malicious process running as the same OS user.

Normal subprocess fork/exec is supported: dropping an owner explicitly unlocks
before closing, so a briefly inherited descriptor cannot prolong ownership.
Raw fork without exec is unsupported; a child must not reuse or drop a copied
store whose file description is still shared with the parent's owner.

Use `store.runtime_authority(&coordinator)` at the reusable runtime boundary so
effect, action, admission, and quiescence checks include the latest durable
catalog owner. Asynchronous drivers must repeat authorization at execution and
report completion only after their scoped work has actually finished.

Catalog updates use a versioned capacity-specific codec, length bounds, a SHA-256
checksum, a synced temporary file, atomic rename, and parent-directory sync. An
old valid catalog remains authoritative until rename. After an uncertain storage
error the core is poisoned and must reload before further work. A leftover
catalog stage may be superseded only after reading the valid current catalog;
corrupt current state never falls back to a potentially older stage.

The encoded catalog is limited to 4 MiB and its in-memory `Catalog<I, O, S>` and
individual identity/operation record types to 64 KiB each. These limits are
checked before capacity-dependent stack construction, including when zero
record capacity would otherwise hide a large service-bound record type.
Native temporary snapshots use heap storage but may first be constructed on the
thread stack; applications must also budget their two caller-owned core buffers.
The host tests exercise the 4-identity/16-operation/2-service configuration on a
512 KiB worker stack. Larger dimensions need separate stack/memory qualification
or a different adapter with caller-controlled scratch allocation.

## Keys and retries

1. Reserve a creation operation and retain its token; begin that same operation.
2. Pass its current `PublishKey` ticket to `publish_key`. Generation uses OS
   entropy through the existing PRNS entropy abstraction. Import checks the
   public identity hash with the existing Reticulum key implementation.
3. A fixed, checksummed key record binds the operation-derived reference to its
   public hash and 64-byte Reticulum private key. The stage is synced before
   rename and directory sync. Only verified durable readback returns a
   `VerifiedKey`; give that witness to the core's `ack_key`.
4. If a reply is lost, reopen the store/coordinator and obtain the fresh ticket
   for the **same operation**. A complete stage or final key reopens the same key;
   incomplete/corrupt material fails closed. It is never replaced with fresh
   entropy. A crash before any key stage exists can safely retry generation,
   since no key was published or acknowledged.

`activate_key` accepts only a current activation/restoration action and reads the
existing final key. It never repairs a missing key, generates new material, or
promotes a stage while restoring an already-published identity. Scoped service
drivers restore their own durable receive state before opening ingress; reading
an identity key alone does not establish service readiness.

A competing import can leave a second retained reference to the same identity.
Call `account_retained_key`, then `acknowledge_retained_key` on the core, before
pruning that operation result. The durable retention index survives result
pruning. There is no automatic key deletion or secret-retirement claim here.

## Security and platform limits

The optional `redb-store` feature exposes `scoped_store::ScopedDatabase` for
native service storage. Each file has an immutable binding to the catalog
incarnation, local identity sequence and public identity hash. Clients retain
that owner for their lifetime; a selected identity is never resolved at execution
time. Local record IDs, request IDs and cursors must always travel with their
storage scope. Different local identities may legitimately have the same local
record ID or retry token.

`StorageScope::path_under` provides a deterministic filename without labels or
public-key-dependent paths. Creation and reopening are explicit operations;
missing or different bindings fail without initialization. The caller initializes
and validates its service schemas separately, so a valid scope marker alone is
not service readiness. This isolates existing contact, direct-message and group
table formats without teaching them UI identity selection. Lifecycle admission,
draining accepted jobs and protecting secret contents remain caller obligations.

This native-only optional dependency never enters portable core, codec or runtime
builds. It does not open multiple writers for the same file or move installation
management/Bluetooth preferences into a persona database.

This initial adapter stores **plaintext private keys** in directories with Unix
mode `0700` and files with mode `0600`. Permission checks reject symlinks,
nonregular files, hard-linked files, and group/other access. This is not encrypted
storage, hardware-backed key protection, rollback resistance, or protection
against a same-user attacker who can replace both data and checksums. Protected
phone storage remains deferred.

The durability contract requires a Unix local filesystem supporting atomic
same-directory rename, file sync, directory sync, and OS advisory locks. The
current evidence is host process-crash recovery on macOS; real power-loss,
mobile-filesystem, phone permission/protection, and device qualification remain
separate work. Unsupported platforms or failed syncs return errors.

Run host checks with:

```sh
cargo test --manifest-path prns-identities/Cargo.toml -p prns-identities-native
cargo test --manifest-path prns-identities/Cargo.toml -p prns-identities-native --features fault-injection,redb-store
```

The optional `fault-injection` feature enables test subprocess exits at catalog,
key, and retention-index write boundaries. It must not be enabled in a shipping
build; normal builds do not inspect the crash-control environment variable.
