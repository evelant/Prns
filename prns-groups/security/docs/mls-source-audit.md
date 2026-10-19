# MLS source audit and integration contract

This audit records upstream behavior and integration requirements at
`mls-rs` 0.56.0/core 0.27.0/RustCrypto 0.22.1,
[`d4c5b19f4bbf8a931face27fc9e30f2db33ce83e`](https://github.com/awslabs/mls-rs/tree/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e).
The [adapter](../mls-rs/README.md) implements the transaction boundary and local
receive quotas; its README is authoritative for current behavior. The audit's
remaining requirements are not claims of completed qualification. See the
[embedded guide](../../core/docs/embedded.md) for platform gates.

## A constrained MLS application profile

Use suite 1, bounded Basic credentials with an application identity provider,
owner-authorized commits, encrypted member controls and fresh commit paths.
Basic credentials are opaque identity data, not self-validating authorization.
Bind each endpoint's credential, signing key, incarnation and group permissions.
Support only our explicitly admitted application profile; do not claim universal
interoperability with every MLS deployment or arbitrary credential format.

| Constraint | Compatibility and implementation consequence |
| --- | --- |
| Suite 1 and Basic credentials | Suite 1 is mandatory to implement; credentials must be supported by the group. These restrictions preserve standard wire/crypto behavior. |
| Private member controls | Enable `private_message`; explicitly select private control messages. `EncryptionOptions::default()` uses public controls. |
| Owner commits, member key refresh | Keep `by_ref_proposal`: non-owner members need signed Update proposals and their pending private keys for PCS. Owner-only self-updates do not refresh every compromised member leaf. |
| Delayed receive | Keep `prior_epoch` and bounded `out_of_order`. Disabling the latter discards skipped keys; it does not enforce strict in-order delivery. |
| Smaller build | Exclude unneeded X.509/custom-proposal plugins, external-client APIs, parallelism and optional indexes. Removing default proposal support such as PSKs yields a restricted profile, not unrestricted RFC conformance. |
| Owner/governance limits | `MlsRules::filter_proposals` receives direction, committer, roster and context on send/receive. Reject unauthorized received commits; all members must apply the same rules. |
| Fresh path for every commit | Set `CommitOptions::with_path_required(true)`. This governs local construction only: incoming add-only commits can omit a path under the RFC. A receive-policy hook/check remains needed for our stronger rule. |

[RFC 9420 §§11.1, 12.2, 13.3, 15.3, 17.1](https://www.rfc-editor.org/rfc/rfc9420.html)
support capability, admission and retention restrictions. Default proposal and
extension types are assumed supported; omit a feature only with explicit profile
compatibility tests. Follow RFC handling of unknown extensions, rather than
rejecting every unknown field. Source:
[feature flags](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/Cargo.toml#L18-L43),
[policy hooks](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/mls_rules.rs#L142-L200),
[receive path check](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/message_processor.rs#L718-L722).

## Bound work before accepting it

Use one serialized MLS worker and one resident group/job initially. Load the latest
committed state on demand; unload inactive groups after commit. This avoids keeping
every group's live tree and temporary workspace resident. Loading still allocates
a complete serialized snapshot and decoded state; storing it in flash does not
remove that peak. A dedicated arena is possible platform work, not a per-group
allocator feature supplied by this library. Default `Vec` allocations are infallible;
an exhausted allocator can abort. Wrapper `try_reserve` cannot make hidden library
allocations recoverable. Arena routing must cover allocation lifetimes, other tasks
and interrupts; reset/zero only after all owned objects and references are gone.

Example qualification vector, **not selected limits or qualified capacity**:
16 members, one resident job, two past epochs, 32 skipped keys per sender and 128
device-wide, with an additional per-group cap. Key quotas count
application and handshake ratchets across retained epochs and persisted groups;
unloading a group must not reset its charge. A transactional quota ledger and
byte limits are required alongside counts. Bound credential/extension sizes,
proposals, pending updates/commits, key packages, wire/body bytes and queued work.

The wrapper can cap wire length, require complete decoder consumption, check
group/epoch and reject known prior authorization eras before invoking MLS.
Private sender/generation data is encrypted, so generation and retained-key limits
need enforcement inside the library before derivation/allocation. Sender-data
decryption alone does not authenticate the individual author. The upstream 1,024
forward-step limit alone does not bound accumulated skipped keys. The vendored
adapter patch adds pre-expansion count/work quotas; it does not make all decoding
and serialization allocations fallible. Bound both growth and
CPU work, including snapshot restoration. The adapter validates retained snapshots against its installed quota policy.
Sources: [ratchets](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/secret_tree.rs),
[ciphertext processing](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/ciphertext_processor.rs#L195-L249),
[snapshot loading](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/client.rs#L709-L745).

Exhaustion must have an explicit recovery path: bounded ordered recovery of holes,
a supported key-discard/advance policy, or owner epoch transition/rejoin. Encrypting
the same record at an even later generation does not repair an excessive gap.
A blind relay cannot sort by encrypted generation. Rate-limit failed attempts and
repair requests independently; never silently enlarge grace, limits or audience.

## One durable transaction around MLS and application state

Implement `GroupStateStorage` and `KeyPackageStorage` over one coordinator with an
exclusive transaction generation and read-your-writes staging. The library writes
current/prior state, then separately deletes a consumed key package. These calls
can stage into the same outer transaction without changing MLS wire behavior.
Provider clones must share that transaction; reject operations outside its scope.
The adapter tests fault-inject this outer boundary; the upstream traits alone
do not provide that guarantee.

1. Load committed authority/barrier, quota and crypto state; begin an exclusive attempt.
2. Enforce bounded ingress and owner/governance rules before durable state changes.
3. Process MLS; validate our exact control/record binding and application admission.
4. Stage crypto snapshots, consumed keys/key-package deletion, barrier/purge, quota,
   replay/admission decisions, obligations and **immutable exact outgoing bytes**.
5. Commit one recoverable marker, then publish output or application effects.

Retries send the committed bytes. Do not regenerate ciphertext or advance an old
snapshot. On abort, discard the mutated `Group` and reload committed state; on
uncertain commit, recover before deciding. Library pending caches are cleared by
`write_to_storage` before our outer commit. Its key-package removal marker is not
cleared, so provider deletion must be idempotent. Stage key-package generation and
advertisement together too. Source:
[write/delete ordering](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/state_repo.rs#L193-L242),
[storage trait](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs-core/src/group/group_state.rs).

Distinguish three rejection boundaries. Invalid MLS AEAD/signature/structure:
discard attempt state and reload; persist only separate bounded abuse metadata.
Successfully authenticated application content rejected by our application:
durably retain consumed crypto state and the rejection/dedup decision. A processed
MLS commit failing our authority/binding check: discard the entire staged merge;
never persist an unauthorized membership transition. Use `MlsRules` for checks it
can express, with final binding checks before outer commit. An arbitrary `MlsError`
does not prove no mutation; the public API lacks a general staged authentication
result. Authenticated control failures need a tested rollback/quarantine policy.
[Receive code](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/mod.rs#L1452-L1502)
mutates the current/prior ratchet before checking the content signature; it does
not automatically clone and roll back the whole operation.

## Erasure, rollback and the smallest library changes

Logical deletion is not physical erasure. Journal/compaction pages and backups can
retain an older complete snapshot and resurrect consumed keys. Static at-rest
encryption does not fix this if its key still decrypts old pages. Qualify an erase
schedule or crypto-erasure with separately protected, destroyed wrapping keys;
state the residual exposure window and wear cost. Retention barriers must deny
old epoch loads and purge staged as well as persisted old-era state atomically.

CRC/MACs and counters stored alongside snapshots cannot detect restoration of an
entire valid old image. Physical rollback resistance needs a protected monotonic
anchor or trusted external checkpoint before continuing. Crash recovery alone is
not that protection. Detectable restoration/state loss triggers a new incarnation
and rejoin; undetected rollback remains outside the claim until qualified.

The pin already wraps `GroupState.data` and `EpochRecord.data` in `Zeroizing<Vec<u8>>`;
[issue #297](https://github.com/awslabs/mls-rs/issues/297) describes an older revision.
Remaining serialized-secret copies deserve review: `PendingCommitSnapshot` holds
a plain `Vec<u8>` encoding epoch secrets, private tree, key schedule and signer;
`CommitSecrets::to_bytes` returns plain bytes, and snapshot encoding grows a plain
`Vec` before wrapping it. Zeroizing the final buffer does not erase earlier copies.
Sources: [pending snapshot](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/snapshot.rs#L55-L73),
[encoded secrets](https://github.com/awslabs/mls-rs/blob/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs/src/group/commit.rs#L69-L97).

Prefer narrow upstream changes, carried as reviewed pinned patches if necessary:
pre-mutation ratchet/cache quotas and checked generation arithmetic; bounded/fallible
decode/allocation paths; receive fresh-path enforcement; zeroizing secret buffers
and borrowed/preallocated snapshot encoding to reduce full-state copies. Existing
provider traits can support our transaction coordinator, so a storage rewrite or
permanent fork is not the first requirement. Each patch needs ordinary MLS interop,
failure/rollback and quota tests. Cryptographic provider qualification, board RNG,
stack/heap/flash measurements and recovery under radio/watchdog load remain gates.
