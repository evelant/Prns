# PRNS receive-policy patch

This directory contains only the `mls-rs` crate from [awslabs/mls-rs](https://github.com/awslabs/mls-rs/tree/d4c5b19f4bbf8a931face27fc9e30f2db33ce83e/mls-rs), version 0.56.0 at commit `d4c5b19f4bbf8a931face27fc9e30f2db33ce83e`. Sibling crate dependencies refer to that exact Git revision. The upstream Apache-2.0 and MIT licenses and source notices are preserved. `PROVENANCE.json` records the copied paths and patch hash; `PRNS.patch` contains every change to those upstream files.

The patch makes local receive resource limits enforceable before ratchet expansion. It does not change MLS messages, ciphersuites, encoded snapshots, or membership semantics. This is a local fork with focused regression coverage, not an upstream-reviewed security release.

The patch also scopes proposal-filter imports to their owning features, removes unused map helpers, and marks tree-index-only parameters explicitly. These changes keep the selected allocation-enabled `no_std` profile clean under warnings-as-errors without changing its feature set.

## Runtime policy

`Group::set_receive_ratchet_limits(ReceiveRatchetLimits { ... })` installs:

- `max_forward_distance`: the maximum gap between the next generation and the requested generation. Processing the requested message adds one ratchet step beyond that gap.
- `max_skipped_keys_per_ratchet`: retained skipped keys for one sender and one key type.
- `max_skipped_keys_per_epoch`: retained skipped keys across every sender and both application and handshake ratchets in one epoch.

Capacity exhaustion rejects the incoming message without eviction. Preflight walks existing ratchets without allocation and checks arithmetic before consuming a secret-tree node, deriving ratchet keys, or inserting skipped keys. Generation increments use checked arithmetic. Sender-data parsing/decryption occurs before the preflight; this is not a no-allocation receive API.

Errors preserve their cause through `MlsError::ReceiveRatchetLimitExceeded(ReceiveRatchetLimitError)`: `ForwardDistance`, `PerRatchetSkippedKeys`, and `PerEpochSkippedKeys` contain `requested` and `limit`; `GenerationExhausted` and `CounterOverflow` report arithmetic exhaustion. The first three distinguish work, one-ratchet retention, and whole-epoch retention failures.

`receive_ratchet_usage()` returns current-epoch total and largest per-ratchet counts. `prior_epoch_receive_ratchet_usage(epoch)` loads and validates a retained epoch, returning `None` when absent. `receive_ratchet_limits()` reports the active policy. `cached_proposal_count()` counts by-reference proposals without cloning their contents.

The setter validates the current tree and already loaded/pending prior trees, and changes the policy only after successful validation. Prior epochs loaded later are checked before receiving and before returning their usage. An adapter requiring whole-import validation must enumerate its retained epoch records and call the prior-epoch query after installing the policy. Policy is intentionally absent from snapshot encoding: adapters must install required limits after creating, joining, or loading a group. `None` retains upstream behavior for compatibility and must not be exposed by the bounded adapter.

Incoming commits can additionally require a fresh path through `MlsRules::require_receive_update_path()`. Its upstream-compatible default is `false`; the private PRNS profile overrides it to `true` and uses `CommitOptions::path_required` for its own commits. Missing incoming paths return `CommitMissingPath` before proposal application. This requirement is stronger than RFC 9420's minimum path requirement for add-only commits.

## Persistence boundaries

Group snapshot reads reject trailing bytes and a decoded group id different from the requested storage key before construction. This applies to both `load_group` and `load_group_with_ratchet_tree`. Lazy prior-epoch reads reject trailing bytes, a different group id, or a different epoch id before caching. These failures return the existing `SerializationError`, `GroupIdMismatch`, or `InvalidEpoch` variants. They do not authenticate storage contents.

Decoding still precedes usage validation and uses upstream infallible allocation. The adapter must cap stored-record and incoming-message bytes before decoding, bound members/proposals/epoch retention/group count, and supply authenticated transactional storage. The patch does not provide arbitrary byte budgets, a fallible allocator, or a device-wide skipped-key quota. One epoch's quota can be reserved across configured groups and retained epochs by the adapter.

General authentication or MLS-processing failures may have consumed a key before failing. The adapter must discard/reload such working state and roll back failed outer transactions. Fully authenticated application data rejected by application policy must instead durably record rejection and the consumed key. A receive-capacity rejection alone does not consume the target secret-tree ratchet, but this does not replace the adapter's transaction boundary.

The selected private feature subset omits PSKs, custom proposals, and X.509; it is an application-restricted MLS profile. The PSK resumption lookup and optional raw secret-tree access APIs are outside the bounded adapter contract. The copied upstream tests reference omitted large fixtures and are not claimed to have run.

## Reproduction

From the repository root, with dependencies already cached:

```sh
cargo +stable test --locked --offline --manifest-path prns-groups/security/mls-rs/vendor/mls-rs/validation/Cargo.toml --test receive_limits
cargo +stable build --locked --offline --manifest-path prns-groups/security/mls-rs/vendor/mls-rs/validation/Cargo.toml --lib --target thumbv7em-none-eabihf
cargo +stable build --locked --offline --manifest-path prns-groups/security/mls-rs/vendor/mls-rs/validation/Cargo.toml --lib --target riscv32imac-unknown-none-elf
python3 prns-groups/security/mls-rs/vendor/mls-rs/verify_vendor.py /path/to/official/mls-rs/git/checkout
```

The verification script reconstructs the copied paths from the pinned Git objects in a temporary directory, applies the checked-in patch, and compares every resulting file. It does not modify the source checkout or this vendor. Maintainers changing the vendor must regenerate the patch and provenance hash against the same pinned revision, then rerun verification.

The validation workspace uses real RustCrypto MLS operations on the host and keeps its lockfile. Its bare-metal entropy handler always returns `UNSUPPORTED`; cross-build success establishes library compilation only, not linked firmware size, target execution, working entropy, allocator behavior, or board readiness. Keep generated test logs and target-build output in an external evidence directory.
