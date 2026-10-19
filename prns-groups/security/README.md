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
