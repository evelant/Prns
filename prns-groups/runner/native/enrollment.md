# Authenticated group enrollment

## Scope and use

A device can create a group locally, invite a specific primary Reticulum identity,
and accept or decline invitations in the app. Each device generates its own MLS
credential locally; joining devices generate their own KeyPackage. No private enrollment fixture is transferred.
Group creation and the user's decisions are durable while offline. Invitation
delivery, joining and control catch-up require a running node and eventual contact
with the owner. Established members can exchange current-era content with each
other while the owner is offline. Saved contact names are display hints, not proof
of a person's identity.

The initial policy is deliberately small: one immutable owner, owner-ordered
single-member additions, four locally managed groups, eight members per group,
sixteen retained invitations, 64 concurrently retained API request identifiers and seven-day
invitation validity. Issued invitations reserve available membership slots. New
members receive current-era content after joining; earlier history is not
transferred. Later additions can also interrupt a delayed join's catch-up across
intermediate eras. Removal, ownership transfer, transferable invitation links,
history sharing and concurrent administrators are outside this slice. Existing fixture
groups remain readable and usable in their original namespaces.

The app consumes `createGroup`, `inviteToGroup`, `listGroupInvitations` and
`respondGroupInvitation`. Mutations take a native-generated 32-byte request token:
`PGR1`, its creation time in seconds, and 20 random bytes. Retry the same immutable
input and token after an ambiguous result. Tokens are not authentication credentials.
`Queued` or a saved
acceptance means local durable intent, not enrollment at the remote device. A
group becomes usable after committing MLS state through its required membership
checkpoint. MLS keys, KeyPackages and Welcome bytes never cross the JavaScript API.

`Client::conversations` also returns the complete current `members` roster and
its `control_head`. Each member carries its exact endpoint and incarnation,
Reticulum identity, and local/owner flags. The roster is taken from the same
validated configuration as the group title and authority; duplicate identities,
missing endpoint mappings and exceeded membership limits fail rather than produce
a partial roster. Legacy fixed fixtures have no owner. The app's `GroupMember`
projection preserves these fields and resolves contact names locally. Neither a
contact name nor inclusion in this roster means a device is connected. The
existing conversation `revision` is a record/change revision, not a membership
revision; use `control_head` for membership provenance. Cached membership can
still precede updates held by other devices.

`Error::rejection_reason` distinguishes definite invalid input, no recipients,
request-input conflict, expired invitation, missing item, membership conflict,
non-owner, incomplete joining and failed authentication. A request-input conflict
means the same request identifier was presented with different input; it is
separate from inviting an already enrolled or already invited identity. Capacity,
busy, uncertain storage/transport outcomes and expired retry evidence retain
their existing distinct recovery paths. Never infer that an uncertain action did
not commit or replace its request token automatically.

The app carries these categories in `GroupRejectionReason` on direct rejected
input outcomes and durable `GroupActionResult::Rejected` records. Previously
saved rejections decode as `Unknown`, preserving their original input and detail.
A replay of a saved completed action returns that result; adding typed reasons
does not execute it again. New action admission after rejection or expired retry
evidence still requires the existing explicit review step.

## Authentication and ordering

Enrollment uses `prns.groups.enrollment`, a separate Reticulum `SINGLE` destination,
with identified encrypted Links and bounded request/Resource frames. Opaque MLS
bytes use base64 inside JSON, with a 64 KiB field limit and a 128 KiB complete
application-frame limit. The carrier
checks the announced destination identity against the expected primary identity;
ingress rejects unidentified requests before copying or queueing their payloads.
A claimed identity inside JSON is never transport authentication. Discovery and
routing retain the existing Reticulum protocol; this adds no core packet format.

The initial directed offer binds the owner's MLS signing key to its authenticated
Reticulum Link identity. Explicit acceptance pins that owner and key. This is
identity-key authentication, not a claim that a saved contact has been verified in
person. An invitation is not a transferable bearer token or an independently
verifiable, offline invitation file.

The owner authenticates acceptance from the invited primary identity and binds it
to the recipient's exact credential and KeyPackage. An owner-signed membership
manifest binds the group, title, parent head, revision, MLS epoch, Commit digest,
and complete mapping of Reticulum identities to MLS credentials. An addition must
preserve every prior member, add exactly one member, and advance the epoch once.
The adapter checks the **actual resulting MLS roster**, not just an allowed
superset, before committing application facts with the crypto state. Existing
members apply a contiguous chain of controls before using its new descriptor;
wrong parents, transitions conflicting with the locally accepted history, and
substituted credentials fail closed. This does not provide consensus or prevent
a compromised owner from presenting separate histories to disconnected members.

The owner retains the exact Welcome and membership result for a consumed invite.
A retry cannot enroll a second member or substitute another KeyPackage. A delayed
Welcome includes the owner's current signed tip; the joining device stays out of
the conversation list until it applies the chain through that tip. The owner shows
“Device joined” only after an authenticated acknowledgement from that device's
durable joined state. Control catch-up transfers one bounded transition at a time. The network runner refreshes
membership from durable state and fences each asynchronous content operation to
its starting control head. A membership change committed locally during that
operation forces a retry. Members can continue using cached membership until
updates reach them; this does not require globally current membership or an online
owner before every content send.

Completion reports scheduling activity only after correlating the pending work
and validating/committing its result. An unchanged control response is idle and
does not wake content synchronization; a committed join, invitation update or
new control is progress. An unchanged control response itself writes no group
state; independently due retention/recovery maintenance can still run.

## Recovery and delivery

One `GroupManager` owns the existing native database lane and one
`NativeStoreOwner` across all group namespaces. Catalog intents and local private
credentials are persisted before crypto output can escape. MLS Create, KeyPackage,
Add, Join and received control transitions stage their enrollment facts in the
same atomic operation as the MLS mutation. Network waits hold no database lease.
Dropping an admitted foreign caller or stopping a network waiter does not cancel
its admitted durable operation.

API results remain replayable while retained. Maintenance retires results after
14 days and commits the deletion with a monotonic time fence, so an old token
cannot become a new operation after a clock rollback. An expired retry fails
explicitly; it must not silently generate another token. Existing opaque request
IDs receive a persisted 14-day migration grace; exact retained retries work, but
unknown opaque IDs cannot admit new work. Durable Create and acceptance intents
recover independently of their API result records.

The invitation catalog reserves twelve slots for locally issued or explicitly
accepted work. Unaccepted incoming offers share four slots, at most two per
authenticated peer. Declining retains its suppression record until the offer's
seven-day expiry. Maintenance then prunes unaccepted/declined offers and persists
an invitation clock fence against redisplay after rollback. Confirmed joins and
definitively expired accepts retain catalog records for 14 days after that state
is recorded. Unfinished accepted joins and owner-committed Adds remain protected:
an invitation deadline cannot undo an Add or discard its exact Welcome.

Enrollment output cleanup checks the durable snapshot before retiring redundant
Create, KeyPackage, Add, Join and received-control outputs, in batches of at most
sixteen per group. Exact signed controls and Welcome bytes remain in that snapshot;
service recovery facts and ciphertext outputs are excluded. Catalog retirement
and each group's output retirement use separate atomic transactions and safely
resume after interruption. This is logical storage reuse, not physical erasure.
The native owner finalizes backend receipts only below the oldest retained MLS
output. New MLS work leaves two receipt slots for cleanup; the last slot can only
retire that oldest output. An already-full older database with no safely finalizable
prefix fails closed rather than forgetting a live obligation to manufacture space.

Add remains an authorization barrier: old receive epochs and old application or
proposal output retries are fenced. Retained Commit/Welcome replay remains
available for control and enrollment recovery. Existing plaintext history and authenticated receipt counts remain
local. Old-era records cease to be offered, receipt emission stops, and logical
receipt reservations are recalculated for the new era. Unfinished messages are
shown as **delivery interrupted by membership change**, never as delivered. This
first version does not repair unfinished old-audience content across an Add; it
must not re-encrypt it into the enlarged group and disclose it to the new member.
Immediately before an Add, the owner checks its durable current-era authored
records. A record with missing receipts blocks the Add until its receipts arrive
or it definitely expires, including records sent after the invitation was issued.
The bounded service query uses publication facts and receipt evidence, so retiring
retry ciphertext cannot masquerade as delivery. Unknown or overlapping time bounds
do not establish expiry. The check and Add run on the same exclusive storage lane.

A blocked Join returns `AwaitingDelivery`. Both endpoints retain the exact Join
hash and KeyPackage across restart; retries with changed input fail. Invitations
expose `waiting_for_delivery` without changing their existing state. Repeated
deferrals are idle, preserve normal retry backoff and do not write again. A
committed Add always replays its Welcome before this check, even if later records
are pending. Deferral does not extend the invitation deadline; an uncommitted
Join can still expire. The visible waiting flag ends at expiry or committed join.

This protects the owner's known authored deliveries, not a group-wide drained
state: another member can still have an offline send the owner has never seen.
Continuous owner traffic or a missing receipt can postpone joining until the
record deadline, potentially past the invitation deadline. For the alpha, finish
important sends before inviting and coordinate additions with existing members.
There is no override that silently abandons pending delivery or re-encrypts
old-audience history for the new member.

## Boundaries and remaining qualification

The generic core's invitation-time and addition-transition checks are allocation-free
`no_std`. The MLS adapter and record service remain bounded `no_std + alloc`.
The enrollment catalog, JSON carrier, native database and mobile scheduling are
`std` adapters; they do not become requirements for embedded implementations.
An embedded enrollment consumer can reuse the core checks and MLS transaction
seams with its own bounded representation and durable backend. The complete
enrollment catalog, wire codec and scheduler have not been implemented for embedded
targets.

The native backend still stores private state without application-level encryption
or physical key erasure. Removing fixture transfer does not establish protected
phone storage, secure backups or at-rest forward secrecy. Group membership and
Welcome contents are carried inside encrypted Links; relays still see ordinary
Reticulum routing and traffic metadata. There is no anonymous enrollment or blind
relay custody in this version.

Storage limits are independent ceilings, not guaranteed usable capacity. Strangers
can still exhaust the four-slot unsolicited inbox, including through multiple
identities; these quotas do not provide spam filtering or identity admission.
Pre-upgrade catalogs above the new incoming quota retain their existing records
until safe expiry. Unfinished or expired accepted joins retain their group slot
and private namespace; group deletion and namespace reclamation are not implemented.
The bounded control/Welcome history remains for each group's lifetime. A full
64-result retry window or protected invitation quota reports capacity instead of
evicting a live recovery obligation. Host tests cover aborted catalog transactions,
database reopen, retirement/clock rollback, repeated invitation cycles and exact
Welcome recovery across retirement; they are not physical power-loss evidence.
Membership tests also reopen both sides after a lost deferral, preserve a Join
issued before an intervening send, keep Add blocked after delivery without its
receipt, and replay a lost committed Welcome despite later pending records.

See [application validation](../../../applications/docs/validation.md) for
the phone acceptance procedure and release limits. Host recovery tests do not
qualify mobile background execution, Python-router application interoperability
or production security. The provisional wire encoding
must be versioned before external interoperability.
