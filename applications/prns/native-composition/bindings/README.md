# Generated application bindings

`@prns-internal/native-bindings` is the generated interface to the existing
`prns-app-native` composition. UniFFI is enabled by default. Swift/Kotlin
lifecycle owners and generated JavaScript calls use one `prns_app` shared native
image per target, with one supervisor, generation and shared Reticulum host.
Named messaging identities have separate keys and storage; Bluetooth and managed
node access retain the installation identity. Native startup and iOS restoration
can precede JavaScript. Loading or reloading JavaScript does not construct another
node or Tokio runtime.

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

## Named identity clients

Production messaging uses `listNamedIdentities`, `createNamedIdentity` and
`changeNamedIdentity` for the bounded local catalog (currently eight identities).
Changes include `Select`, `Rename`, `Pause` and `Resume`. Persist the creation
`requestId` before submission and retry the same ID and input after an uncertain
result. A private catalog `label` is separate from the public messaging name;
new identities publish no name unless one is explicitly supplied.

After starting the native node, capture a reference and bind all domain calls:

```ts
import { developmentRuntime } from "@prns-internal/expo";

const result = await developmentRuntime.listNamedIdentities();
if (result.tag === "Ready" && result.inner.catalog.selected !== undefined) {
  const reference = result.inner.catalog.selected;
  const identity = developmentRuntime.forIdentity(reference);
  const contacts = await identity.listContacts();
}
```

`forIdentity` copies the complete immutable reference and lazily opens the
generated `openNamedIdentity(reference)` / `NamedIdentityClient`. Keep that
client with the screen, request or recovery record. Changing the selected default
never retargets an existing client or admitted operation. Native code verifies
the catalog incarnation, sequence and public hash on every dispatch, including
deferred requests. Contacts, direct messages, groups, recovery actions and
announce preferences use that identity's storage.

`state` is the durable desired state; `transitioning` means activation or drainage
is unfinished. A successful Pause reply confirms completed drainage. Paused
identities retain history, local contact/profile edits and completed
action recovery, while fresh sends, group mutations and announces are rejected.
Other identities keep running. These local operations require the shared native
owner to remain started; stopping the entire node makes named clients unavailable
until restart. They never activate a paused identity implicitly.

Unbound global messaging functions remain for legacy/development fixtures; they
do not resolve the selected named identity. Production code must use the bound
client, without falling back to those globals when it is unavailable. Installation
startup, Bluetooth and remote management remain shared operations.

### Local activity and viewed cursors

`listNamedIdentityActivity()` returns per-owner summaries for the bounded catalog;
the bound client exposes `readIdentityActivity()` and
`markIdentityActivitySeen({ scope, throughSequence })`. `scope` identifies a group
or direct destination under that captured owner. Use the exact incoming record
position displayed by the consumer (`GroupTextMessage.sequence` or
`LxmfMessage.localRecordId`, both `bigint`), not a newer summary watermark after
rendering an older page. Marks are monotonic and durable. A later arrival remains
unread after a stale mark; selecting an identity never marks anything viewed.

These are local viewed markers, not network read receipts. Summary/mark calls do
not announce, activate a paused identity or transmit messages. Pending incoming
invitations remain actionable until accepted, declined or expired. Local activity
requires the shared native owner to remain started, including for paused identities.

The current adapter scans 512 retained direct rows, lists up to 64 direct
conversations and retains 260 combined group/direct cursor slots per identity.
Groups use the current four-group/256-text-record retention bounds. Treat `partial`
counts as lower bounds and preserve unavailable results; neither establishes an
empty inbox. Cursor exhaustion returns `Capacity` without discarding unread state.
A busy owner's activity can be unavailable while other owners remain readable.
The app refreshes and marks focused conversations only while foregrounded.

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

Messaging profile reads/writes do not activate the identity or announce it.
A saved public name updates that identity's registered LXMF announce data when
active; `SavedButNotApplied` explicitly separates a committed preference from an
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

## Group conversations and recoverable app actions

The bound group text API reads and writes through the identity-scoped database
owner shared with its contacts and LXMF. `createGroupRequestId` obtains an
identifier from platform entropy; `sendGroupText` binds that identifier to one
immutable text payload.
After an uncertain result, retry the same identifier and text. A committed send
survives a dropped caller, JavaScript reload, and database reopen. `Accepted`
means local durable acceptance. Per-device receipt counts never imply human read
status. History is newest first, with exclusive local sequence pagination and a
durable revision that also changes when receipts arrive.

The app creates groups and exchanges authenticated invitations without fixtures.
The optional `group-fixture.json` remains a legacy development regression tool:
its primary-identity checks apply to that fixture path, not production named
enrollment. It installs only into vacant matching group storage and never
restores initial ratchets over live state. The redb backend is still development
plaintext; protected mobile keys and history remain work.

The app UI prepares each create/invite/respond/send input durably before
execution. Preparation and action references include the expected device identity;
native admission refuses a stale identity before any mutation. `listGroupActions` restores outstanding intent and local outcomes;
`executeGroupAction` retries the saved immutable ID and input. Completed actions
require acknowledgment; `RetryExpired` requires explicit reviewed discard.
Native scope admission rejects a second outstanding action if the first prepare
reply was lost. This is a bounded UI recovery journal, not another delivery queue.
The existing low-level mutation functions remain available to clients that own
their own durable intent. Text not yet prepared is still an unsaved UI draft.

Bound `getGroupAnnounceSettings` and `setGroupAnnounceSettings` change local
preferences without activating the identity or announcing. The app defaults to
hourly announcements and supports
manual-only, 15/30 minutes and 1/3/6 hours; automatic timing has positive jitter.
`announceGroups` queues both group service destinations for the captured identity.
All identities and their LXMF/group services share a bounded scheduler and
60-second aggregate cooldown. Named manual calls return `Queued` or `Throttled`;
neither means host dispatch or reception. Where a legacy API returns `Requested`,
it means local host admission, not receipt by another device. Settings changes
do not announce.
The reusable runner still defaults to six hours. See the
[announcement rationale](../../../../prns-groups/transport/reticulum/docs/announce-strategy.md).

Each active identity's group driver uses its own destinations on the shared
Reticulum host and interleaves inbound requests with bounded outbound sync.
It owns no separate runtime. Pause/Stop fence admission and drain accepted
database jobs. An enabled identity can accept durable sends without a live
interface; paused identities allow history but reject fresh publication.
Network progress requires the shared node and a usable route. Platform background
lifetime and phone recovery still require device qualification.

`GroupConversation.synchronization` reports whether the generation's group
worker is enabled, stopped, or unavailable. `Running` is not delivery evidence.
A terminal group-worker error marks synchronization unavailable and retains
readable history and durable local send admission; it does not stop the shared
Reticulum host or direct LXMF service. Restarting the native node creates a new
group synchronization lifetime over the retained state.

`GroupConversation.progress` reads the shared runner's bounded, disposable
per-peer observations: connecting/synchronizing, retry delay, storage barriers,
membership catch-up and typed failure categories. An unknown pending value is
not an empty queue; a retry delay is not a delivery estimate. Stopping or losing
the worker suppresses live phases and retry deadlines. Durable message IDs and
device receipts remain authoritative across restart. The app displays this
projection and never schedules network retries in JavaScript. Message body
test identifiers include the durable record ID so physical lifecycle trials can
verify exact records without exposing private database contents.

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
