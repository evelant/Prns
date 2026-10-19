# Application roadmap

The app focuses on messaging, groups, node management and network inspection.
The [product scope](product-scope.md) owns that direction. This page prioritizes
work; [validation](validation.md) records evidence rather than promises.

## Current baseline

The current source includes guided setup, signed contact import into an explicit
identity, authenticated groups, cross-identity activity and durable pending-send
recovery. The [validation index](validation.md) separates source checks from
historical phone trials; a new distribution candidate needs its own acceptance.

Reusable group records, enrollment, synchronization/progress APIs and telemetry
consumption are implemented, alongside the independent identity lifecycle and
contact-card libraries. The general Expo SDK remains a distinct host/runtime
layer. Node management and network inspection are already usable, with narrower
physical qualification than their implemented control surface.

## Before maintainer distribution

| Priority | Outcome to establish |
| --- | --- |
| Multi-member demo | Qualify three independently running participants: one offline, partial receipts becoming complete, and delivery while the owner is absent. Three identities on two phones do not establish these conditions. |
| Distribution candidate | Assign version/build identifiers and a stable Android signer; create the actual TestFlight candidate. Verify fresh installation and update with populated named identities, groups and pending work. |
| Repeatable handoff | Supply a small foreground demo script, supported-device/build details, one-app-variant Bluetooth guidance and a useful diagnostic report. |

The owner now defers Add while its current messages await receipts or definite
expiry. Coordinate other publishers: this local guard cannot prove every member
has flushed. A bounded wake after receipts could reduce join latency; current
automatic retries can be minutes apart. Removal and ownership recovery remain
separate design work.

Keep runtime, reusable libraries, native contracts and app journeys reviewable
as separate source boundaries. Package registries and generated contracts belong
with their first consumers; historical evidence does not qualify a new prefix.

Current capacity and missing group deletion can be managed by reusing a demo
group. Existing admission, expiry, maintenance and safe retry retirement should
be extended with targeted tests, not replaced. Do not clear app data to make a
stalled action appear successful.

## Subsequent reliability and product work

| Area | Next work and boundary |
| --- | --- |
| Group lifecycle | Design removal, ownership recovery, history sharing and group departure/reclamation. Keep authorization and delayed-content behavior explicit. |
| Durable phone storage | Protected keys, locked-state behavior, supported backup/recovery, disk-full handling and physical-space reclamation. Logical cleanup is not secure erasure. |
| Mobile lifecycle | Long idle, natural suspension, repeated restoration, permission transitions and message notification policy. A running service is not a background-delivery guarantee. |
| Radio qualification | Repeat isolated E290/LoRa exchanges under movement and load on matching builds. Earlier foreground radio trials do not qualify new app binaries automatically. |
| Embedded group members | Portable enrollment/reconciliation and durable custody within measured flash/RAM limits. Forwarding a phone's traffic does not make the board a group member. |
| Remote management | Broader Android/iOS capability-driven controls, successful Wi-Fi Keep, cancellation/uncertain results and complete headless/recipient onboarding. |
| Reusable direct messaging | Refine the LXMF mailbox/storage boundary independently of the app; retain supported interoperability checks. |
| Later messaging features | Attachments, notifications and linked-device identities after their delivery, storage and privacy semantics are defined. |

Use the [radio trial plan](../../prns-groups/transport/reticulum/docs/lora-qualification.md),
[embedded participant plan](../../prns-groups/core/docs/embedded-participants.md),
[remote-control plan](remote-control-expansion.md) and
[named-identity design](named-identities.md) for their detailed decisions.

## Contribution and release rules

- Keep application dependencies one-way. Rust owns protocol behavior and durable
  work; UI code renders state and submits commands through generated bindings.
- Preserve the allocation-free `no_std` core. Put host/runtime/storage choices in
  adapters and test reusable APIs through independent consumers.
- Change retained screen IDs, parameters, links and deep-link tests together.
  Remove unsupported product routes deliberately instead of advertising them as
  future features. Do not remove useful node capabilities with unrelated UI scope.
- Separate host, simulator, packaged-build and physical evidence. Reproduce old
  failures on relevant builds; historical resource headroom or CI is not current.
- Review dependency changes as upstream work lands. The
  [release contract](../release/README.md) requires a real containing revision and
  matching artifacts; working-tree qualification is a different gate. Do not
  promote historical digests or claim public distribution from a local test pass.

Choose one observable outcome per increment. Keep source revisions, package
hashes, results and remaining limits with the CI/release or external trial record.
Use the [validation index](validation.md) for repeatable checks; keep generated
logs, screenshots and per-run narratives out of the source documentation.
