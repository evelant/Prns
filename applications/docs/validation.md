# Validation and known limits

This is a maintainer alpha. Host tests, source exports, packaged builds and
physical trials establish different things. A later source tree does not inherit
phone or board acceptance from an older binary. Consult the actual CI run for
current status.

Start with the [maintainer quickstart](group-maintainer-alpha.md),
[build guide](../README.md) and [roadmap](roadmap.md).

## Evidence boundaries

| Evidence | Establishes | Does not establish |
| --- | --- | --- |
| Host and controlled-time tests | The tested contracts, recovery paths and bounded policies on that source | Phone lifecycle or physical radio behavior |
| Source export and detached consumer checks | Dependency closure, generated interfaces and supported consumer integration | A signed APK, TestFlight installation or public distribution |
| Firmware build/resource checks | Linked fit under the selected configuration | Peak stack/heap use, energy or physical recovery |
| Packaged-build and device trials | The recorded scenarios on the identified binaries/devices | Acceptance of a later source tree or untested lifecycle states |

Keep source revisions, package hashes, commands and results together in CI/release
artifacts or an external trial record. Historical counts and machine-local paths
are not current qualification. Preserve failed attempts when comparing recovery;
clearing data is not a successful retry.

The app's native owner surviving JavaScript replacement differs from the
standalone SDK's JavaScript-owned session reopening after reload; see the
[SDK qualification guide](../../prns-react-native/docs/qualification.md).

The [LAN/TCP guide](ip-connections.md) records independent configured TCP,
Bonjour and stock multicast boundaries and remaining package acceptance.

## Repeatable checks

Run the [workspace setup](../README.md#setup) first, then select checks for the
boundary changed:

| Gate | What it establishes |
| --- | --- |
| `api:check` | Generated contracts match source |
| `verify` | Portable native, SDK/platform, UI, routes, configuration and web-export checks |
| `mobility:verify -- --working-tree` | Isolated current-source export, packed dependencies and supported native/Python LXMF exchange |
| `mobility:verify` | Historical recorded-release contract; promotion requires review |
| `native:ios:test` | Swift lifecycle and release-symbol checks |
| `native:android:standalone` / `native:ios:build` | Embedded-JavaScript packages; installation and physical acceptance are separate |

From the repository root, reusable group checks include:

```sh
cargo test --locked --manifest-path prns-groups/Cargo.toml --workspace --features prns-groups-store/secure
cargo check --locked --manifest-path prns-groups/Cargo.toml -p prns-groups -p prns-group-text --target thumbv7em-none-eabihf
npm --prefix applications run native:test
npm --prefix applications/prns/app run test
```

The [runner guide](../../prns-groups/runner/native/README.md) explains independent
host/process tests. The core, text/contact formats and portable identity lifecycle
remain allocation-free `no_std`; MLS and record-service layers use bounded
allocation, and native composition requires a host runtime.

## Before handing out a build

- Record source and package hashes, build identifiers, signer/distribution method
  and supported devices. Verify the actual APK and TestFlight installation/update.
- Repeat fresh setup, explicit contact ownership, invitation/joining and two-way
  messaging. Check original pending sends across identity pause and app restart.
- Demonstrate three independently running participants, partial receipts, an
  offline participant and catch-up while the author or owner is unavailable.
- Keep one app variant participating in Bluetooth on each phone. Preserve data
  during recovery; clearing it is not a successful retry.

Protected phone storage is deferred for this maintainer alpha. Wider deployment
still needs protected keys, backup policy, storage-full and crash-at-write checks,
removal/owner recovery, sustained reclamation and mobile lifecycle qualification.
Embedded transit forwarding is not durable group custody. The
[radio procedure](../../prns-groups/transport/reticulum/docs/lora-qualification.md)
and [participant plan](../../prns-groups/core/docs/embedded-participants.md)
set those separate gates.
