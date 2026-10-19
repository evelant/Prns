# Reusable group communication

Address independently credentialed endpoints as one logical group, distribute
records across intermittent contacts and reconcile durable device receipts.
Messaging and telemetry share the same record APIs; applications own schemas,
projections and effects. The group layer uses existing Reticulum SINGLE
destinations, Links, requests and Resources without changing core routing.

Start with the [architecture](core/docs/design.md),
[behavioral contract](core/docs/v0.md), [policies](core/docs/policies.md) and
[security analysis](security/docs/security.md). These distinguish implemented
behavior from wider design options. Current native enrollment supports one owner
and ordered additions; removal, ownership transfer and history sharing remain
separate work.

| Component | Boundary |
| --- | --- |
| [Core](core/README.md) | Allocation-free `no_std` identifiers, bounded policy and delivery evidence |
| [Transaction store](store/README.md) | Bounded `no_std + alloc` commit/recovery coordinator and optional secure flash backend |
| [Native store](store/native/README.md) | Caller-owned database adapter; development storage is plaintext |
| [MLS adapter](security/mls-rs/README.md) | Bounded `no_std + alloc` crypto operations committed with application facts and exact output |
| [Records](service/README.md) | Opaque application bytes, receipt intent, retention and inventory; `no_std + alloc` |
| [Text profile](profiles/text/README.md) | Allocation-free borrowed UTF-8 encoding |
| [Reticulum transport](transport/reticulum/README.md) | Native adapter using a borrowed Host client and authenticated peers |
| [Native runner](runner/native/README.md) | Caller-owned storage jobs and network future; [enrollment](runner/native/enrollment.md), [generic records](runner/native/generic-records.md) and [progress](runner/native/progress.md) |

A receipt proves durable acceptance by an enrolled endpoint incarnation. It does
not prove a human read, application execution or network-wide delivery. Delivery
targets select obligations; all MLS readers remain in the encryption audience.
Plaintext history retention is separate from ratchet-key retirement.

For embedded work, start with [support and release gates](core/docs/embedded.md)
and the [participant design](core/docs/embedded-participants.md). ESP32-S3 with
PSRAM is the first full-member target. The E290 storage fixture is separate from
a complete durable network member; historical software-reset results do not
establish brownout tolerance, protected provisioning or current resource margins.

The private application formats remain provisional. Host recovery tests do not
qualify mobile background behavior, protected phone storage or board power loss.
See [application validation](../applications/docs/validation.md) for the containing
source and device evidence boundaries. Raw experiment crates, logs and daily
trial captures are retained outside this shipping workspace.

From the repository root:

```sh
cargo test --locked --manifest-path prns-groups/Cargo.toml --workspace --features prns-groups-store/secure
python3 validation/run.py run --suite group-communication-tests --suite group-communication-clippy --platform current
```

Follow the [contribution guide](../CONTRIBUTING.md) and
[verification ladder](../docs/testing.md). Applications own transport lifetime,
clock, entropy, storage and scheduling; these libraries do not create a second
node or implicitly select a security profile.
