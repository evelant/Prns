# Named identities design

The bounded lifecycle, native storage/services, generated app APIs and named
identity UI are implemented. Use the [reusable library](../../prns-identities/README.md)
for mechanism and [validation](validation.md) for the limits of the recorded host
and historical phone checks.

Support several named, independent Reticulum identities on one shared node.
Users can keep Personal, Work or Community conversations separate without
running another app or attaching another radio. Start with explicitly created
identities; automatic per-contact or per-group identities can use the same
mechanism later. The reusable lifecycle contract remains `no_std` and
allocation-free, with caller-owned storage and bounded resources.

This design supplements [product scope](product-scope.md). It does not change
Reticulum packets, the signed contact-card format, or MLS. Linked devices sharing
one logical identity, remote revocation, secure deletion, backup/restore and
traffic anonymity are separate work. Protected phone storage remains deferred
for the maintainer alpha.

## Ownership and boundaries

An identity has an immutable local ID, public identity/hash, opaque vault key
reference, private local label, optional published display name, lifecycle
revision and service preferences. Metadata revision is separate from lifecycle
revision and runtime generation: renaming must not invalidate an admitted send.
Keys are independent; changing a label does not change keys. Local IDs and key
references are never reassigned. Importing an already known key resolves to that
identity instead of creating a duplicate;
import cannot silently reactivate a retired identity.

The private label, such as Work, is never automatically used as a card name,
announce name or group label. Newly created identities start with an empty
published name unless the user explicitly supplies one. Each existing-format
contact card exports one chosen identity and service, with no list of sibling
identities or public proof linking them.

| Layer | Responsibility |
| --- | --- |
| Reusable identity lifecycle core | Fixed-size IDs, revisions, states, transition/admission rules, bounded dependency reports and typed outcomes; no allocator, executor, database, UI or group dependency |
| Vault and catalog adapters | Durable key/catalog publication, bounded iteration, transition journal and recovery; native and embedded implementations declare their storage and atomicity capabilities |
| Host/runtime integration | Explicit identity-bound destinations and authenticated Links, resource reservations, ingress/egress fencing and aggregate scheduling on one node |
| Application services | Bind records and relationships to the local owner, report unresolved obligations and implement quiescence/recovery through supported hooks |
| App | Names, selection, confirmation, defaults, contact presentation and generated bindings |

Keep lifecycle types in a separate reusable library above core identity/vault
primitives; do not put messaging policy into `prns-core`. The first implementation
is `prns-identities-core`, in its own zero-dependency workspace, without making
other consumers depend on native composition. Borrowed strings/buffers, caller-provided bounded tables
and cursors replace owned collections in the portable contract. Native adapters
may allocate. Existing MLS/store implementations have their own allocation
requirements; this design does not relabel them as allocation-free.

Every admitted operation captures local ID and expected public identity. Runtime
work also carries the lifecycle revision and host-session generation. Conversation
ownership never follows a mutable UI default:

- Direct conversations use `(local identity, remote service destination)`.
- Group enrollment uses `(local identity, group ID)` and retains its own MLS
  endpoint/incarnation. V1 permits one local identity enrolled in each group.
- Outbox records, recovery actions, receipts, cursors, notifications and queries
  preserve that owner. A retry reuses the original record and sender.
- Authenticated Links are keyed by local identity and remote destination/service;
  never change personas by re-identifying a live Link.

Existing group manifests bind fresh MLS credentials to a Reticulum identity;
different MLS credentials alone do not hide that identity across groups.
Changing an existing group's local identity requires a membership operation,
not editing its catalog. Current enrollment has no removal or owner transfer.

## Activation and retirement

Durable desired state is separate from transient runtime status and UI selection.

| Desired state | Network and new operations | Retained work |
| --- | --- | --- |
| Enabled | May register configured services, authenticate, send, receive and synchronize when runtime/resources permit | Recover original pending work under normal retention and retry rules |
| Paused | Blocks fresh message publication, enrollment and announcements; service traffic stops when quiescence completes. Local contact editing/import, offline card export, browsing and drafts remain available | Preserve accepted work and evidence for explicit reactivation |
| Retired | Terminal local archive; no activation, new signing or network use through this catalog | Preserve history and exact-result recovery; no retransmission or fresh execution |

The intended presentation distinguishes starting, running, stopping and unavailable.
The current app API exposes durable `state`, `transitioning` and `ready` instead:
`ready` means lifecycle admission is open, not that an interface or peer is reachable.
A Paused identity with `transitioning=true` is still Pausing. Fresh sends are fenced,
but direct ingress can continue until endpoint retirement finishes; only completed
quiescence establishes that the identity's services are quiet. Enabled also does
not promise background execution. UI selection chooses the default for new
interactions; all enabled identities remain eligible for receiving. Pausing the
selected identity requires another explicit choice before starting a new
conversation, rather than silently changing sender.

Current paused local access requires the shared local node and `NamedRuntime` to
remain running. It needs neither a usable interface nor activation of that persona.
Stopping the whole local node currently makes history, local edits and contact-card
APIs unavailable too; node-independent local access is not implemented.

Creation commits the key and catalog before returning a usable identity, initially
Paused. A create-and-enable app flow performs explicit activation afterwards.
Activation preflights the whole configured service set against held-key,
destination, Link and queue capacities. It never evicts another identity or
changes defaults to make room. Failure returns `CapacityExceeded`,
`StorageUnavailable` or a specific runtime reason. Partial registration is
unwound before reporting failure; pending cleanup remains visibly unavailable.
Loss of interfaces leaves desired state Enabled and does not itself clear the
current API's lifecycle `ready` flag. Connectivity is a separate observation;
ordinary connectivity recovery does not require user activation again.

Pause closes admission, records transition intent, settles already admitted
durable mutations, removes scheduled announcements and service work, closes both
outgoing and incoming Links, unregisters destinations and releases active key
handles when safe. Teardown may finish protocol close packets. Report completion
only after the local quiescence barrier; until then show Stopping. Bytes already
handed to an interface or forwarded by another node cannot be recalled. Other
identities, shared interfaces and board-management credentials keep working.
A key may also serve transport or management roles in another embedding. Report
those other uses in the lifecycle review; pause/retirement governs this catalog's
application services and does not imply the key disappears from all node traffic.

Pause neither cancels accepted sends nor extends their retention, invitation
expiry or cryptographic key lifetime. Reactivation resumes exact recoverable
records, reconciles receipts and reports expired or unrecoverable work accurately.
For ratcheted services, preserve still-valid receive keys in durable storage
before unregistering, and restore them before reopening ingress. Core unregister
removes in-memory destination ratchets; pause must not shorten their retention or
resurrect erased keys. Admission of a lifecycle-managed service requires this
preservation capability. Catch-up is unavailable after required content or keys
have been pruned. New Send is disabled while paused; editable drafts are not
accepted outbox records.
Offline card signing uses an explicitly scoped vault handle without registering
destinations or enabling services. The sharing screen shows that the local identity
is paused; the card itself makes no presence claim. Manual announce, notification
taps and incoming work never activate it.

Retirement requires a current, bounded dependency review and explicit confirmation:

1. Block retirement during unresolved storage recovery, while the identity owns
   a group or has an unsettled accepted enrollment/control operation. Offer Pause.
   Committed Add/Welcome replay and accepted joins cannot be discarded merely
   because an invitation looks old.
2. Pending application messages must finish or be explicitly marked locally
   stopped before retirement can commit. Stopping retries does not assert that
   recipients never received them. Keep original IDs, receipt evidence and retry
   retirement fences; never convert pending work to Delivered.
3. Explain remaining ordinary group memberships: retirement stops this endpoint
   locally; peers still consider it a member and may retain/send ciphertext for
   it. No automatic leave, removal, replacement identity or forwarding occurs.

The normal V1 flow has no force-retire bypass for ownership or unresolved control
obligations. An identity can remain paused indefinitely. A service unable to
enumerate its obligations or stop pending work safely blocks retirement with a
typed reason; the catalog must not guess from UI projections.

Retirement does not delete private keys, undo signatures, invalidate distributed
cards or erase peers' caches/history. Retained secrets remain subject to existing
storage/key-retention policies; it supplies no new forward-secrecy guarantee.
Permanent erasure is deferred because it needs separate vault, flash, backup and
history-access semantics. The terminal catalog record prevents accidental reuse,
not use of an exported key elsewhere.

## Durable transitions and recovery

Lifecycle changes serialize with service admission under one coordinator.
Prepare returns an advisory revision-bound summary of affected services, pending
work and blockers, including service revisions; it does not hold admission closed
while the user reviews it. Apply reacquires the gates and rejects stale summaries.
Apply closes admission and drains/recovers already admitted mutations, then
authoritatively checks retirement blockers again. An admitted group creation or accepted invite
may have created new obligations. Changed consequences require a refreshed summary
and confirmation; the identity remains fenced/paused rather than completing
retirement under stale approval.

Persist an idempotent transition intent and increasing revision before external
activation/teardown effects. Already admitted mutations settle through their
original owners before the transition completes; losing a caller's reply does
not cancel an accepted mutation. Persist completion after service barriers.
On restart, replay unfinished transitions before reopening admission or scheduling
announcements. Unknown/corrupt lifecycle state fails closed for that identity.
Revision exhaustion is an error, never wraparound.

The vault and catalog are not assumed to share a transaction. Creation journals
the intended key reference before writing it, then publishes the catalog only
after durable key storage. Recovery can finish or clean up the referenced orphan
without requiring an unbounded vault scan. Never replace an existing key on an
uncertain result. Orphan cleanup is limited to newly created, provably unpublished
keys owned by that journal entry; uncertain or imported keys are retained for
explicit recovery. A catalog without its key stays unavailable. Create/import
accept a caller operation ID and retain a queryable durable result: retry after
commit-before-reply returns the same local ID/key, never another identity. Bounded
result pruning retains a rejection fence for old operation IDs rather than
treating an expired retry as a fresh creation. Use catalog-issued operation tokens
with an incarnation and monotonic sequence, bound to the command's digest;
bounded active slots and a completed-sequence floor make that rejection possible
without an unbounded set of random IDs. Reserve the token durably before key
generation. A stale/changed command must obtain a new token explicitly.

Old session/revision callbacks cannot reopen listeners, enqueue retries or update
another identity. Valid late delivery evidence can still be committed to its
original record through the service's recovery path; an observation must not
reactivate the identity. Exact retained-result lookup and settlement of previously
admitted operations remain allowed after retirement, without new signing, network
work or fresh execution. Durable records survive runtime generation changes.
Resource exhaustion returns bounded errors without deleting another identity's
state. Reserve journal/terminal-state capacity so stopping does not require free
message-payload storage.

## Contact separation and app behavior

Keep relationships private to each local identity. A relationship binds the local
ID to a verified remote identity and service destination; aliases, favorites,
verification decisions, notification preferences and drafts belong to that scope.
Personal's Alice and Work's Alice may refer to the same remote key without sharing
conversation history or automatically copying trust decisions.

A shared, bounded cache may deduplicate authenticated public identity/service
bindings. It must not contain private aliases or make an observed name/card into
a trusted relationship for every local identity. Card names remain self-asserted
and replayable. Verification results never silently overwrite an existing identity
binding; the favorite flag is independent of key verification/conflict checks.
Blocking a remote identity applies across its services for the selected local
identity; it does not remove group membership or suppress required authenticated
group control/receipt processing. Message display/notification filtering is a
separate group-profile policy.

The app lists contacts within the selected identity. A general import/scan preview
offers **Save contact to** and confirms **Save to …**. Explicitly changing this
destination retains the card draft, binds a new client and revalidates the preview;
it does not change the global default. A scan launched from a group is fixed to
that group's local identity. Switching the global selection while a preview/action
is open neither retargets it nor discards its captured owner. Pause blocks the
invitation/send step while allowing a local contact save; retirement blocks both
mutations and new contact-card signing. V1 has no automatic cross-identity contact
merging/copying or public identity linking. Board pairing/access remains node
management, outside the messaging persona selector.

Guided setup starts the shared node, checks Bluetooth permissions and actual
connection state, then lets the user choose a named identity and its separate
public messaging name. A paused identity requires explicit Resume. Setup does not
replace existing keys, connections or grants. Its contact/group actions capture
the chosen owner. Installation-identity import is a separate advanced operation,
not named-identity or conversation migration.

### Local activity and viewed state

The picker and Inbox project activity across up to eight identities without
merging their histories. Native summaries report actionable incoming invitations
and unread retained incoming group/direct records. Reading a summary or changing
the default does not acknowledge messages or activate an identity. Invitations
remain actionable until accepted, declined or expired; summary reads do not run
enrollment maintenance.

Viewed cursors are durable local state keyed by immutable owner and conversation.
The foreground, focused conversation marks only the incoming sequence actually
displayed, using its captured client. Sequence positions come from durable records,
not timestamps; a stale mark cannot consume a later arrival. Duplicate ingress,
device receipts and verification upgrades do not create new activity. These are
local viewed markers, not network read receipts, and reading/marking performs no
announce or transport operation.

The current adapter scans at most the newest 512 retained direct rows and includes
at most 64 direct conversations per owner. Group activity follows the current four
groups and 256 retained text records per group. Omitted rows/conversations set
`partial`; counts are then lower bounds. A busy owner returns an explicit
unavailable result without delaying another owner's summary. The UI preserves
these distinctions rather than showing a complete empty inbox. The seen store has
260 combined group/direct cursor slots per owner; exhaustion returns `Capacity`
without silently advancing viewed state.

Activity refresh runs every three seconds while the app is active, with a refresh
on foreground return. A busy/unavailable owner permits one follow-up read after
400 ms to avoid repeated collisions with Inbox's own refresh; it does not turn
unavailable state into a fabricated zero or an unbounded retry loop. Local activity
and marks work for paused identities while
the shared owner remains started. Whole-node Stop still makes them unavailable;
node-independent history, background notifications and cross-device viewed-state
synchronization are not implemented. The reusable generic core remains `no_std`
and allocation-free; these bounded projections and persistence are native adapters.

## RemoteControl and managed nodes

The current phone uses separate installation-owned RemoteControl controller and
target keys, plus a separate Bluetooth identity. Selecting, pausing or retiring a
messaging identity must preserve those keys, pairing grants, management Links and
interface settings. Paired boards alone are not retirement blockers. The board's
displayed Node ID is its RemoteControl target identity hash; renaming a messaging
identity does not change it. Erasing/reprovisioning that target key can change it.

The phone retains its original installation key as `HostConfig.identity`.
Its endpoint role does not install a transport routing identity. Production
messaging destinations and Links use explicit named service identities; no
legacy primary messaging worker runs alongside them. The first named Personal
identity has a fresh key. Selection never restarts the shared host.
`OwnedSession::stop` shuts down the entire shared node and is not a persona pause
barrier. Destination retirement alone also does not settle outgoing Links,
application queues, pending crypto or driver egress.

Generic embeddings may deliberately share an application key with transport or
management. E290's stable node key, for example, serves its transport and node
destinations; its management keys are separate. The adapter must retain a key
needed by another owner and report that use in the transition review. The portable
core needs generic dependency/service IDs, not RemoteControl grants or board types.
Activation budgets must preserve infrastructure headroom: the current RemoteControl
pairing peak reserves three held identities, three destinations and two request
handlers. These are pairing resources, not a per-persona requirement.

A phone using an E290 as its BLE-to-LoRa forwarder does not transfer persona or
MLS private keys to the board. Pairing grants management access, not group
membership, custody or phone-delivery evidence. Multiple phone identities can
use that same forwarding path without a new wire protocol. A board that hosts
its own group participant needs its own bounded catalog, persistence/runtime
adapters and admission budgets; it is a separate integration from forwarding.

Source anchors: [phone credential composition](../prns/native-composition/src/lifecycle.rs),
[controller selection](../../prns-runtime/core/src/runtime/remote_control_target_connection.rs),
[RemoteControl resource contract](../../prns-core/src/remote_control/core.rs),
[E290 key ownership](../../personal-hopspot/embedded/esp32/src/identity.rs),
[whole-node shutdown](../../prns-host/impls/native/src/owner.rs), and
[embedded participant roles](../../prns-groups/core/docs/embedded-participants.md).

## Announce policy

Policies attach to enabled services under each identity, with one shared scheduling
owner per node. Preserve current defaults while changing their scope:

| Service policy | Initial behavior |
| --- | --- |
| App group services | Startup plus 60 minutes with 0–25% positive jitter; choices Manual only, 15, 30, 60, 180 or 360 minutes |
| Reusable native group runner | Startup plus six hours with 0–25% positive jitter unless caller overrides |
| Direct LXMF | Explicit/manual announcements; adding personas does not enable a new periodic timer |
| Other applications | Caller supplies service policy within host resource/rate bounds |

All groups for one identity share sync/enrollment destinations. No announcements
per group, message, receipt, membership revision or retry. Group announce data
stays empty; LXMF announces publish only that identity's configured public name.
Shared-interface traffic and public keys can still correlate destinations.
N enabled group identities produce 2N periodic service announcements per interval,
independent of group count, before forwarding and requested responses.

Manual only disables unsolicited startup/periodic announcements. It permits
explicit Announce now and requested path responses for registered services; it
does not mean hidden or guarantee routes can be discovered. Pause/retire
unregisters those services, but cached announces and third-party copies persist.
Sharing/importing a card, changing UI selection, renaming or changing a preference
does not itself announce. The app action targets the displayed identity and
explicit service set; there is no implicit Announce all identities action.

The coordinator coalesces at most one due item per identity/service and uses
bounded fair selection. Startup/reactivation waits for an eligible interface.
The current app staggers identity/service batches; group sync and enrollment are
submitted consecutively within each batch, then use normal interface pacing.
There is no additional application delay between those two destinations. Preserve
the 60-second attempted-batch cooldown per identity/service set, including uncertain
or failed submissions. V1 additionally admits at most one application announce
batch per 60 seconds across all identities, with fair rotation and the existing
per-interface airtime budget. Intervals specify earliest eligibility, not promised
transmission times. Manual requests cannot bypass that budget or starve automatic
work. Current named group calls return Queued, Throttled (with the delay) or
Unavailable. Named LXMF calls return Queued, Throttled, Failed or LocalNodeStopped.
Both dispatch asynchronously: Queued retains process-local intent and makes no
durable-send promise; Throttled also retains the intent. Requested remains in the
shared API contract for other paths, but these named methods do not return it.
Where used, Requested means local host acceptance, not radio emission or peer
reception. Partial accepted/unconfirmed destination masks are persisted internally;
there is no app API for reading these dispatch outcomes yet. Queue saturation is
currently projected through each method's failure outcome, not a uniform Busy result.

Persist a compact attempt reservation before dispatch to survive crash loops;
do not persist a write on every timer tick. Adapters supply monotonic time and a
durable wall-time floor when available. After clock rollback or an unknown reboot
interval, wait at least a fresh cooldown and stagger/coalesce missed work instead
of replaying missed periods. Renaming, selection and preference changes never
clear reservations. V1 adds no automatic announce-on-every-reconnect loop;
existing directed path repair remains available and separately throttled.

PRNS's normal announce pacer already budgets local announcements across each
interface. Locally generated path responses take a separate send path; the
application coordinator is therefore not a bound on all discovery traffic.
Keep Python-compatible path handling, measure these packets separately, and
consider any additional core rate policy independently. Fast BLE does not justify
aggressive refresh because forwarded announcements can reach slow LoRa links.

## Integration and acceptance

1. Build the bounded lifecycle/catalog contract and an independent non-messaging
   consumer. Test creation publication and every transition crash point with
   injected storage/runtime failures before connecting real services. Implemented.
2. Add durable catalog/vault adapters and scoped runtime controls. Make native
   direct/group state, contacts, commands and Links explicitly owned, replacing
   singleton group catalogs and host-identity assumptions. Use a fresh scoped
   storage schema: no legacy-data migration is required because this application
   has no distributed installation or user data to preserve. Opening incompatible
   or corrupt storage must still fail explicitly, never silently replace keys.
   Native filesystem storage, bounded runtime controls, live direct/group service
   hooks and scoped application data are implemented and host-tested. See the
   [embedding contract](../prns/native-composition/src/identity_services/README.md)
   for admission, cancellation recovery and shutdown boundaries. Physical radio
   completion and per-identity inbound resource quotas remain unqualified.
3. Shared scheduling, generated owner-bound APIs, paused local access and the
   named-identity UI are implemented. Creation, selection, private rename and
   pause/resume are exposed. Group departure/retirement hooks remain separate.
   Keep testing two enabled identities talking to the same peer, different
   groups, and pause/restart during pending delivery and enrollment.
4. Qualify both phones with exact-ID retry, receipt isolation, cross-identity UI
   switches during scans/actions, pause/retire/restart, and retained scoped data.
   Measure announce counts/bytes and fair progress over a constrained shared route.

The current phone composition retains at most eight identities, including paused
ones. It runs one service or announce job per identity, with a 32-entry command lane
and a separate 32-entry deferred queue. Deferred calls recheck their captured
identity's admission before execution. Same-owner history/settings can wait behind
an unresolved recipient lookup; other identities and catalog reads remain
responsive during those jobs. Create returns Busy while any such job is active,
and Resume returns Busy while its owner is active. Activation and final quiescence
still await adapter work serially and can delay unrelated requests while their
barriers settle. These
native bounds do not change the caller-selected capacities of the portable library.

Host acceptance must include late callbacks, partial registration/teardown,
retirement blockers changing after prepare, duplicate-key import, rollback clocks,
full journal/catalog capacity, unrelated identity continuity and crash after
announcement reservation but before/after dispatch. Prove no allocation in the
new portable contract and build it for embedded targets. Fixed-capacity tests
exercise more saved than active identities and deterministic admission failures.

Exact phone/board capacities and scheduler airtime shares need measurement before
shipping defaults. Deletion/recovery, membership removal/owner transfer, automatic
per-context identities and simultaneous local personas in one group remain later
increments. Use [app validation](validation.md) to select checks and record exact
source and device evidence; this design alone is not a qualification claim.

## Sources and alternatives

- [Contact-card codec](../services/contact-card/README.md): already accepts arbitrary
  identity/service signers; no wire expansion or account-wide card is needed.
- [Vault contract](../../prns-core/src/identity/vault/core.rs),
  [held identities](../../prns-core/src/identity/held/core.rs),
  [host destination config](../../prns-host/core/src/config.rs): multiple keys and
  dedicated destinations are the underlying primitives. The new identity library
  supplies the separate durable catalog and lifecycle controller.
- [Current direct mailbox](../services/lxmf/src/mailbox.rs) and
  [group enrollment](../../prns-groups/runner/native/enrollment.md): peer-only
  conversations and the local group catalog are now separated by immutable scoped
  databases. Admitted mutation and enrollment replay obligations constrain shutdown.
- [Destination retirement](../../prns-core/src/engine/destination_retirement.rs)
  closes responder Links; a persona barrier must also account for its outgoing
  Links, application work and queued egress. This API alone is not that barrier.
- [Existing announce strategy](../../prns-groups/transport/reticulum/docs/announce-strategy.md),
  [pacer](../../prns-runtime/core/src/manifold/announce_pacer.rs) and
  [path responses](../../prns-core/src/engine/command_execution.rs) establish current
  behavior; the shared coordinator and durable reservations now sit above them.
- [Reticulum identities](https://reticulum.network/manual/understanding.html#identities)
  and [announce controls](https://reticulum.network/manual/interfaces.html#announce-rate-control),
  checked 2026-10-17; local Python RNS 1.5.4 `Destination`, `Transport` and `Link`
  source supports several independent SINGLE destinations on one transport.

Aliases under one public key were rejected for identity separation because
announces expose that key. Separate nodes per persona add radio/runtime cost
without being required by routing. Per-contact keys are compatible future policy,
but automatic proliferation and membership migration would complicate the first
app journey. Named identities with explicit, stable ownership provide the useful
separation while keeping those extensions possible.
