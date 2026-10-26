# Application roadmap

The [product scope](product-scope.md) defines the intended messaging, group,
management and inspection product. This app boundary includes direct LXMF
messaging, contacts, local identity setup, node controls and network inspection.
It does not include group enrollment or named-identity app APIs/UI.

## Integration priorities

| Area | Next outcome |
| --- | --- |
| Group services | Connect bounded records, storage and MLS to transport, authenticated enrollment and observable recovery; prove an independent consumer before app integration |
| App group journey | Create/invite/join, send/reply, partial receipts and offline catch-up using public library APIs |
| Named identities | Explicit owner-bound contacts, groups, operations and service lifecycle; keep installation management credentials separate |
| Direct messaging | Refine the reusable mailbox/storage boundary while retaining LXMF interoperability and exact recovery ownership |
| UI | Keep messaging primary, remove unsupported shell routes and make supported controls/details easy to find |

Coordinate group, identity and synchronization choices through a shared design
before app integration. Library mechanisms alone do not establish an app journey;
shared keys or copied databases do not provide authenticated device linking.

## Reliability and handoff

- Qualify reconnect, silent-peer expiry, competing connections, long idle,
  natural suspension and permission/radio transitions on the actual binaries.
  See [Bluetooth recovery](../../docs/bluetooth-session-recovery.md).
- Complete capability-driven management acceptance, successful Wi-Fi Keep and
  recipient onboarding before exposing another-controller authorization. See the
  [remote-control plan](remote-control-expansion.md).
- Verify fresh installation and retained-data update for signed distribution
  candidates. Default SDK and app aggregate ownership need separate evidence.
- Treat firmware linked fit, peak stack/heap behavior and physical radio recovery
  as different gates. Transit forwarding is not durable group custody.
- Keep removal, ownership recovery, linked-device identity, protected storage,
  attachments and notification policy explicit future work.

## Contribution and release rules

Dependencies remain one-way: the app consumes reusable Rust and public host APIs.
Keep protocol behavior, durable state and recovery out of screens. Preserve
allocation-free generic cores and caller-selected resource limits.

Change screen IDs, parameters, routes and deep-link tests together. Unsupported
shell routes are not product commitments. Keep node capabilities separate from
unrelated UI scope changes.

Use [validation](validation.md) for repeatable checks. Store exact source revisions,
package hashes and results with CI/release artifacts or external trial records,
not chronological source-document diaries. Old physical results do not qualify a
new binary. The [release contract](../release/README.md) requires a containing
revision and matching artifacts; local source qualification does not promote it.
