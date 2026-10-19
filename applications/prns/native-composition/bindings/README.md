# Generated application bindings

`@prns-internal/native-bindings` is the generated interface to the existing
`prns-app-native` composition. UniFFI is enabled by default. Swift/Kotlin
lifecycle owners and generated JavaScript calls use one `prns_app` shared native
image per target, with the same supervisor, storage owner, generation and node
identity. Native startup and iOS restoration can precede JavaScript. Loading or
reloading JavaScript does not construct another node or Tokio runtime.

## Generate and build

From the repository root:

```sh
npm --prefix applications run api:generate
npm --prefix applications run api:check
npm --prefix applications run bindings:ios -- --sim-only --targets aarch64-apple-ios-sim
npm --prefix applications run bindings:android -- --targets arm64-v8a --release
```

Generation refreshes product TypeScript, Swift, Kotlin and semantic contract
fingerprints, and selects this composition through `host-provider.json` for the
general Expo SDK. Checking rejects stale output without rewriting it. The SDK
package owns the sole `prns_app` native image; this package owns only product
bindings. The native client helpers build it before Expo prebuild/autolinking.
`bindings:aggregate:test` exercises real generated Python namespaces against
that image, including borrowed ownership, native event admission and restart;
the routine `native:verify` gate includes it.

See the [generation guide](../../../tools/generated-bindings/README.md) for
toolchain requirements, target selection and disposable build caches. Generated
files are outputs of that workflow and must not be edited directly.

## Imports and platform admission

Use `@prns-internal/native-bindings` for types, enum factories, generated
converters and fingerprint constants. This entry is safe to import on web and
does not install the native runtime. The `/native` entry installs the JSI runtime
and initializes the generated UniFFI ABI checks. The app-owned platform facade
loads it lazily after checking native capability, then checks the app and canonical
Host semantic fingerprints separately. Product calls use `@prns-internal/expo`
for platform admission; general host calls use `personal-rns-expo`.

The execution boundary is:

- Product snapshots, pairing/management workflows, contact operations and LXMF
  operations call generated async bindings. `previewIdentityImport` is a bounded synchronous
  parse of exactly 64 bytes; `bindingContract` reads immutable identifiers.
- Seven Expo methods retain platform-owned storage, identity and lifecycle
  admission: `prepareStorage`, `inspectIdentity`, `createGeneratedIdentity`,
  `createImportedIdentity`, `start`, `stop` and `reset`. Inputs and results cross
  Expo as owned byte arrays using the generated UniFFI codecs, without a
  handwritten domain model or field-by-field mapping.
- `prepareOutbound` retains the platform preflight before outbound work.
  Bluetooth authorization, permissions and platform status events remain native concerns.

The synchronous generated `native*` functions are for Swift/Kotlin background
or lifecycle queues only. They can perform storage I/O or wait for Start/Stop;
JavaScript must use the Expo admission methods instead. Storage preparation
establishes the existing database owner before offline async access. Async calls
cannot supply filesystem paths or silently reopen storage after reset. Native
Start uses that same owner.

## Canonical values

Application counters and identifiers use Rust `u64`, generated as exact
TypeScript `bigint`, Swift `UInt64` and Kotlin `ULong`. Keep them exact through
arithmetic and comparisons. Canonical Host `safeUint` fields remain checked
JavaScript numbers, while canonical exact counters remain `bigint`.

The repository's canonical generator owns all host records, conversions and
the TypeScript adapter to `personal-rns/contract`. The former app snapshot
generator has been deleted. This package imports `personal-rns-expo` converters
and the external `HostClientHandle`; its custom snapshot type delegates to those
same generated conversions. Canonical state remains owned by `prns_host`; the
no_std core has no UniFFI dependency. `sharedHost` lends command/query access
without the authority to stop native services.

Generated types support `exactOptionalPropertyTypes`. UniFFI `None` uses
`undefined`; the canonical Host adapter omits absent optional properties as its
contract requires. Generated tagged enum variants use `.tag` and, for payloads,
`.inner`. Fixed 16/32-byte inputs validate during lifting. Contact normalization,
identity and pin rules, mailbox pagination and other input bounds remain shared
Rust semantic checks; TypeScript types alone do not validate foreign inputs.

## Cancellation and ownership

Generated async functions accept optional `{ signal: AbortSignal }`. The app facade
forwards caller signals, including Effect interruption, and checks cancellation
after platform preflight before submitting domain work. Aborting a caller or
tearing down its JavaScript runtime releases its generated future without
stopping the native node.

Describe cancellation drops its app-owned network future and releases admission;
its deadline and priority Stop also bound the operation. Cancellation cannot undo
upstream work already issued before a Link handle is available. Accepted contact
and mailbox mutations, messaging-name writes, sends and retained announcement
operations remain owned
by their native lanes after the caller departs. Stop drains admitted durable
work. A rejected caller promise therefore does not prove that an accepted write
was rolled back and must not trigger an automatic retry.

Messaging profile reads/writes prepare local storage without starting the node or
announcing. A saved name updates the registered LXMF announce data of a running
node; `SavedButNotApplied` explicitly separates a committed preference from an
unconfirmed live update. `AnnounceLxmfOutcome.Requested` means fan-out admission,
not proof that another device heard it. Discovery clearing affects visibility,
not durable contacts or routing. A fresh send resolves missing authenticated
recipient metadata before durable mailbox acceptance; the UI may still be waiting
while the native send lane owns that work.

Network inspection is app-only: `DevelopmentNodeSnapshot.network` exposes current
routes and a bounded, generation-owned accepted-announcement history. It does
not change the canonical Host contract. `clearNetworkActivity` requires the
expected generation and returns an activity revision; presentation can suppress
older projections after acknowledgement without rewriting native snapshot
arrays or hiding newer observations. It needs neither storage preparation nor
outbound Bluetooth admission and never clears contacts, routes or messages.

Async admission uses bounded lanes and nonblocking supervisor access. It performs
no path resolution, database opening, synchronous response wait or worker join
on Hermes. A process-wide timer driver keeps offline query deadlines working;
it owns neither an executor nor a node.

## Group conversations and recoverable actions

The group APIs use the application database owner shared with contacts and LXMF.
Create groups and exchange authenticated invitations without development fixtures.
`createGroupRequestId` obtains an identifier from platform entropy; keep that ID
and the exact input when retrying an uncertain create, invitation or send.
`Accepted` means local durable acceptance. Per-device receipts do not imply human
read status. History uses exclusive local-sequence pagination and a durable
revision that changes when receipts arrive.

Use `prepareGroupAction` before submitting create/invite/respond/send intent.
Preparation checks the expected local identity and commits an immutable input.
`listGroupActions` recovers outstanding work after restart; `executeGroupAction`
reuses the original request ID. Completed results require acknowledgment, while
`RetryExpired` requires explicit discard. An uncertain reply cannot justify a
new request ID. The low-level methods remain available to consumers that own
their own durable intent.

`getGroupAnnounceSettings` and `setGroupAnnounceSettings` save preferences without
starting the network. The app adapter defaults to hourly announcements with
positive jitter; callers can select manual-only or 15/30-minute and 1/3/6-hour
intervals. `announceGroups` requests the two group service destinations. A
`Requested` outcome means local host admission, not peer reception. The reusable
runner retains its six-hour default.

One group worker shares the existing Reticulum host and interleaves inbound
requests with bounded outbound synchronization. Accepted database jobs drain on
Stop, and durable sends can be accepted without a live interface. Network
progress requires a running node and usable route. Synchronization/progress
projections are disposable observations; message IDs and device receipts remain
authoritative across restart. No JavaScript network retry loop is required.

The optional `group-fixture.json` path is only a regression tool: it installs into
vacant matching storage and never restores initial ratchets over live state.
The native backend remains development plaintext; protected phone storage and
fresh mobile acceptance are separate work.

## Runtime scope and validation

The runtime and generator use the exact jsi2 source revision and ordered patches
recorded in the [vendor distribution](../../../../vendor/ubrn/README.md). The current
patches cover strict optional properties, JavaScript runtime teardown, Android
queue teardown and Apple framework version metadata. They are shared runtime
maintenance, separate from the generated application API; the vendor guide
documents verification and eventual replacement with an upstream release.

This API uses generated future-completion callbacks, not application-defined
UniFFI callback interfaces. Bluetooth callbacks and platform events retain their
existing native ownership. Adding callback interfaces or new runtime features
requires separate support and lifecycle validation.

`npm --prefix applications run native:test` covers full snapshot transport,
native-before-JavaScript ownership, exact integers, cold offline storage,
generation-safe deadlines, caller cancellation and durable writes after caller
drop. `api:check` verifies generated outputs. These host checks complement actual
iOS/Android startup, reload and lifecycle validation; they do not replace it.
