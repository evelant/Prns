# Group security profiles

MLS is the selected private group protocol. The [bounded adapter](mls-rs/README.md)
implements recoverable operations, credential binding and explicit quotas with
`no_std + alloc`. The generic group core remains allocation-free and independent
of MLS types. No application may silently weaken protection when memory,
connectivity or keys are unavailable.

Read the [security analysis](docs/security.md) for Reticulum/LXMF comparisons,
relay visibility, delayed delivery, key lifetime and the alternatives considered.
The [source audit](docs/mls-source-audit.md) records the pinned backend's integration
requirements; the adapter README distinguishes implemented controls from gaps.

The [embedded guide](../core/docs/embedded.md) selects ESP32-S3/PSRAM first and
separates the local E290 storage fixture from a complete network member. Native
recovery tests, library builds and historical device measurements do not establish
protected storage, secure erasure or a production-qualified group profile.

## Backend selection and regression scenarios

Native hosts, phones and opt-in embedded fixtures use the same bounded
[`mls-rs` adapter](mls-rs/README.md), named `prns-groups-mls`. OpenMLS is not a
runtime dependency. Its earlier host/SQLite experiment required `std`; replacing
its storage provider would not meet the portable foundation's requirements.
The selected `mls-rs` library supports `no_std + alloc`; the wrapper adds explicit
receive quotas, credential checks and recoverable operations. This selection is
not independent security or full-firmware resource qualification.

The retired prototypes' selected scenarios are maintained against the live
backend rather than as another crypto stack or clone-based model:

| Scenario | Current regression owner |
| --- | --- |
| Delayed receive across rekey; removal purges prior epochs | [Adapter recovery](mls-rs/tests/recovery.rs) and [secure reopen](mls-rs/tests/secure_recovery.rs) |
| Exact committed ciphertext retry after recovery; membership fences stop stale output | [Adapter recovery](mls-rs/tests/recovery.rs) |
| Failed staging or malformed input leaves a valid retry possible | [Composition](mls-rs/tests/composition.rs) and [adapter recovery](mls-rs/tests/recovery.rs) |
| Per-sender and aggregate skipped-key limits survive reopen | [Adapter recovery](mls-rs/tests/recovery.rs) and [vendor receive limits](mls-rs/vendor/mls-rs/validation/tests/receive_limits.rs) |
| Receipts retain exact endpoint incarnations, original targets and deadlines | [Portable delivery](../core/src/delivery.rs) and [clock admission](../core/src/time.rs) |

Timed MLS grace, independent custody and live-record FIFO eviction remain
separate policy work. A historical model is not evidence that those policies are
implemented; the record service must preserve its selected exhaustion behavior.
