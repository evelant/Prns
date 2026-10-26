# Prns applications

Prns is a messaging app with direct and group conversations, named identities,
node management and network inspection. The phone works without an external
board. The app consumes public Prns APIs; the core does not depend on this tree.

## Start here

| I want to… | Read |
| --- | --- |
| Try a group demo | [Maintainer quickstart](docs/group-maintainer-alpha.md) |
| Find a feature or setting | [App guide](prns/app/README.md) |
| Choose checks and understand acceptance limits | [Validation and limits](docs/validation.md) |
| Build or change the app | Setup and commands below; [iOS](docs/ios.md) / [Android](docs/android.md) |
| Understand the design | [Product scope](docs/product-scope.md), [named identities](docs/named-identities.md) |
| Choose the next contribution | [Roadmap](docs/roadmap.md) |
| Use the libraries elsewhere | [Group runner](../prns-groups/runner/native/README.md), [identity library](../prns-identities/README.md), [contact cards](services/contact-card/README.md), [Expo SDK](docs/react-native-sdk-implementation.md) |

Active guides describe current scope and workflows. The
[validation guide](docs/validation.md) defines source checks and physical
acceptance requirements. Keep per-run results with the exact source and package
in CI/release artifacts or external trial records.

## Ownership

| Path | Responsibility |
| --- | --- |
| `prns/app` | Expo screens, navigation and presentation |
| `prns/native-composition` | App policy, durable storage, service composition and shared-host ownership |
| `prns/native-composition/bindings` | Generated product API; shared host converters and one selected native image |
| `prns/platform` | Product storage/startup/reset admission and platform facade |
| `services` | Reusable LXMF services/wire format and signed contact-card codec |
| `../prns-groups`, `../prns-identities` | App-independent group and identity mechanisms |
| `../prns-host/impls/native` | Shared native runtime, resources and shutdown |
| `../prns-react-native` | General Expo host SDK |
| `target/react-native-sdk` | Generated app-selected SDK package; no separate SDK sources |
| `tools/generated-bindings` | Product generation recipe using shared `../tools/uniffi` tooling |

Rust owns protocol behavior, identity, durable work and recovery. TypeScript
renders projections and submits commands. The generic group/identity cores and
contact/text codecs remain allocation-free `no_std`; native adapters and bounded
MLS implementations have different resource requirements.

Edit Rust declarations and generator inputs, then regenerate bindings. Do not
hand-edit generated TypeScript, Kotlin or Swift. The
[binding guide](prns/native-composition/bindings/README.md) explains cancellation,
owner-bound clients and the native lifetime boundary.

## Connection and delivery contracts

Automatic Bluetooth is network access, not contact authentication or a node-control
grant. Use the shared peer/session policy, bounded queues and backoff; do not add
an app-owned transport or periodic radio reset. Announcements are requested
through native scheduling, never from screen renders. Acceptance for fan-out is
not RF transmission or remote discovery. See [Bluetooth recovery](../docs/bluetooth-session-recovery.md)
and the platform guides for ownership and lifecycle constraints.

Discovery and Network history are bounded, session-local observations. Clearing
them preserves saved contacts, authenticated recipient metadata and queued work.
Fresh sends resolve missing recipient metadata before durable acceptance; never
silently replace a pinned identity. A verified repeat of a received message can
upgrade that saved record, but learning a sender key alone does not reverify old
history. Delivery proof and sender verification are separate facts.

Network observations come from the native capture and monotonic clock. A route
or interface snapshot is not the path of an earlier message; persist per-attempt
evidence if the UI needs it after restart, otherwise label it unavailable. An
inspection failure is not an empty network. Keep physical peer counts, routes,
Links and contacts distinct, and retain generation/revision fencing when clearing
observations so stale snapshots cannot restore cleared rows.

## Setup

Install Node/npm, Python and Rust using the repository's pinned toolchains.
Linux native checks also need `pkg-config` and DBus development headers
(`libdbus-1-dev` on Ubuntu). From the repository root:

```sh
npm --prefix prns-js ci --ignore-scripts --no-audit --no-fund
npm --prefix prns-js run build:code
npm --prefix prns-react-native ci --ignore-scripts --no-audit --no-fund
python3 applications/tools/generated-bindings/generate.py stage
npm --prefix applications ci --ignore-scripts --no-audit --no-fund
```

`stage` checks committed app bindings and writes the ignored
`applications/target/react-native-sdk` package. Run it before installing the app
workspace in a fresh checkout and after removing `applications/target`.

Mobile builds additionally need Xcode/CocoaPods on macOS for iOS, or JDK 21,
the Android SDK, pinned NDK, `cargo-ndk` and Rust targets for Android. Follow the
platform guides rather than mixing their build outputs or native providers.

## Generate, build and check

Run from the repository root:

```sh
npm --prefix applications run api:generate
npm --prefix applications run api:check
npm --prefix applications run verify
npm --prefix applications run mobility:verify -- --working-tree
```

`verify` checks portable Rust, generated output, SDK/platform boundaries, UI and
web export. The working-tree mobility gate checks an isolated source export,
packed dependencies and native/Python LXMF interoperability. Neither is a phone
or distribution result. Without `--working-tree`, mobility checks the historical
recorded release; see [release qualification](release/README.md) before promoting
that contract. Do not rewrite historical release digests to describe local work.

Build standalone apps with embedded JavaScript:

```sh
npm --prefix applications run native:android:standalone
npm --prefix applications run native:ios:build
```

The default helpers produce a development-signed Android Release APK and an
unsigned iOS simulator Release app. They do not qualify APK distribution or
TestFlight. For signing, device installation and Metro development clients, use
the [Android](docs/android.md) and [iOS](docs/ios.md) guides. On macOS, also run
`npm --prefix applications run native:ios:test` for Swift lifecycle/symbol checks.

Set `PRNS_BUILD_ID` when packaging a candidate to expose a useful build label in
About and diagnostic reports. Keep its source/artifact record alongside the
package; the label and API contract fingerprint are not source-hash attestations.

The app and general SDK select different native images from the same source.
App builds must preserve canonical SDK files and committed product bindings.
CI configuration describes intended coverage; consult the actual run for results.
`PRNS_UBRN_CACHE` and `CARGO_TARGET_DIR` can place disposable caches on a larger
volume; keep host operating systems and source checkouts separate.
