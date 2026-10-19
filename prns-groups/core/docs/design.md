# Group addressing and synchronization

Status: implementation in progress. The [group core](../README.md)
contains an allocation-free core, bounded transaction and record services,
a bounded MLS adapter, and native transport and runner implementations. Earlier
host models and crypto experiments are retained separately from this publication.
This document describes the wider design, including capabilities that remain
planned; it does not establish an interoperable protocol or qualified guarantees.
Messaging is one consumer; the
[messaging product scope](../../../applications/docs/product-scope.md) remains separate.

Provide reusable group addressing, authenticated record distribution and optional
state synchronization above Reticulum. Applications supply data meaning and
policy. Federation is cooperation between storage/delivery peers, not a required
server topology or a synonym for group membership.

The [policy and recovery design](policies.md) develops
freshness, governance, protection, durability and retention profiles, control
distribution, protocol outcomes and worked failure cases.
The [behavioral contract](v0.md) selects a narrower design with one fixed owner,
cached publication and explicit repair/history rules. It also describes planned
custody and public-record profiles; the current private implementation uses MLS.
The [embedded guide](embedded.md) identifies supported layers and remaining
integration gates without making a member-support promise for every board.
The [security analysis](../../security/docs/security.md) compares Reticulum,
LXMF and the proposed group profile, including relay visibility, forward secrecy,
compromise recovery and retained history.
The [delayed-delivery options](policies.md#delayed-delivery-across-routine-rekeys)
separate routine encryption epochs from authorization/security barriers. The
MLS profile permits bounded old-epoch delivery within unchanged authority.
All profiles retain conservative rejection across known authorization barriers.

## Goals and non goals

Goals:

- Address logical groups of people, devices or services without managing each
  physical destination in application code.
- Support intermittent contact, selective replication and explicit delivery
  evidence under bounded storage, memory and airtime.
- Reuse mechanisms across messaging, telemetry, shared state and objects.
- Support `no_std` embedded participants with explicit board, feature-set and
  role limits. Start with ESP32-S3/PSRAM; qualify C6, no-PSRAM S3 and nRF52840
  separately rather than requiring every board to host MLS.
- Prefer Python Reticulum core compatibility; consider extensions when they
  demonstrably improve the design.

Non goals: universal database replication or conflict resolution, unlimited
history, unconditional delivery, exactly-once physical effects, new cryptographic
primitives, or every board and group size in the first implementation. Embedded
support is an architectural requirement; qualification remains specific to a
board, role, security profile and measured limits.

## Model

A **logical group address** identifies an authenticated, versioned descriptor:
authority, member credentials and reachable endpoints/ingress services. It stays
stable across endpoint and membership changes. Resolve it through an invitation,
cached descriptor or reachable peer; a central directory is not mandatory.

Keep these sets distinct:

- **Principals and endpoints:** a principal may be a person, sensor or service;
  independently credentialed endpoint instances perform network operations.
- **Authorization and encryption audience:** who may administer, publish or read.
- **Replication scope and subscriptions:** which namespaced records/content a
  consumer wants and which copies a custodian promises to retain.

Subscription filtering saves traffic; it is not a confidentiality boundary.
Applications bind records to their namespace and schema version, using separate
security contexts when authorized readers differ. A relay may store opaque
content without being a reader or subscriber.

## Layers and compatibility

| Layer | Responsibility |
| --- | --- |
| Reticulum core | Destination routing, packet protection, Links, requests and Resources. |
| Host/runtime adapters | Network access, streaming, time/entropy, key/storage integration, bounded queues and scheduling. |
| Reusable group library | Descriptors and authority checks; security profiles; record reconciliation, delivery obligations/evidence, selective object transfer. These are separable components. |
| Application profiles | Schemas, subscriptions, retention, merge/checkpoint rules, presentation and execution of effects. |

Start the library over ordinary `SINGLE` destinations and Links. Existing Python
routers can carry it unchanged; Python applications need the new application
protocol to participate. This does not imply unmodified LXMF/RFed compatibility.

The Python source explains why this belongs above today's routing layer:

- `GROUP` uses shared-key encryption, but ordinary GROUP packets bypass routed
  lookup and are rejected beyond one hop. Only `SINGLE` destinations can announce
  or accept Links. It is not routed reliable multicast.
  [Destination](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Destination.py#L252-L256),
  [Transport](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Transport.py#L1655-L1666),
  [Link](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Link.py#L230-L231).
- A destination has one selected next hop. Cloning its identity does not create
  a recipient set. [Transport](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Transport.py#L1383-L1405).
- Resource proofs precede application completion callbacks. Transport success
  therefore cannot stand for durable application receipt.
  [Resource](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Resource.py#L721-L757).

Native multihop multicast could reduce repeated transmission over shared paths,
but would require routing/membership semantics legacy routers do not provide.
It still needs offline repair, authorization and custody. Benchmark direct
fan-out and relay-assisted distribution first. Any core extension needs capability
negotiation, a compatible fallback and mixed Python/PRNS tests. Missing streaming
or scheduling APIs are host improvements, not necessarily wire changes.

## Embedded execution and storage

The reusable mechanism must not require `std`, SQL, a filesystem or a particular
executor. Follow the existing split between
[fixed-capacity nRF storage](../../../personal-hopspot/embedded/nrf52840/src/storage.rs)
and [ESP32 memory profiles](../../../personal-hopspot/embedded/esp32/src/storage.rs).
`no_std` and allocation-free are separate requirements: provide a fixed-storage
core path, with allocation-backed adapters only where the board budget permits.
Use borrowed payloads or stored-object handles and bounded working buffers;
cloning the entire retained state is a host-model technique, not the device
transaction strategy.

| Role | Embedded requirements |
| --- | --- |
| Reticulum router | Existing forwarding; no group membership or MLS keys are required. |
| Opaque custodian | Bounded ciphertext transfer/storage, access control, quotas and custody evidence; no group decryption keys are required. |
| Group participant | Group/control verification, the selected security profile, replay and receipt state, and recoverable persistence. Embedded telemetry and command consumers need this role. |
| Group owner | Participant functions plus membership and profile-specific control work, including MLS Commits when selected; qualify its peak costs separately. |

The same commit contract applies to native and embedded stores: crypto state,
admission/replay facts and outgoing work become recoverable together before any
ciphertext or receipt is released. A board-owned flash journal or checkpoint
protocol can implement this without SQL. Existing
[flash-journal code](../../../prns-core/src/persistence/impls/flash_journal/mod.rs)
provides useful commit/recovery patterns, but its Reticulum-specific records do
not already supply group transactions. Budget erase blocks, compaction, write
endurance and uncertain-commit recovery. Logical deletion is not physical secret
erasure. Time/entropy adapters must also work on-device; uptime alone does not
establish trusted wall-clock expiry across reboot.

Earlier host models and OpenMLS/SQLite experiments informed the design and are
not shipping dependencies. MLS is the selected private protocol; the current
adapter uses `mls-rs`. The [embedded guide](embedded.md) separates board resource
contracts from unqualified library allocation and firmware headroom estimates.
A `no_std` build is only the first gate: measure allocation/stack peaks, retained
epochs, join/update costs, flash writes and airtime within the existing firmware.
An embedded reader must not silently fall back to a weaker security profile or
require a phone merely because the initial test harness used a host database.

## Reusable contract and current thinking

Consumers open a scope with credentials, storage, a security profile and budgets.
They publish typed records or object references, subscribe to streams, request
reconciliation/content and observe explicit outcomes: local commit, custody
accepted, recipient receipt, expiry or unavailable content. Applications report
semantic outcomes such as applied or executed separately.

**Reconcile records, not databases.** Records bind stable IDs to group, namespace,
author, authorization version and content. Persist pending delivery obligations
alongside durable admission; duplicate arrival must not erase unfinished work.
Exchange paged inventories and fetch gaps with backoff. Reject conflicting authenticated
content under one ID. The logical record stays immutable across retries; refreshed
epoch/session, recipient and attempt bindings belong in separately authenticated
envelopes or obligation revisions. Optional causality references support merge
rules; neither wall-clock ordering nor consensus is implicit.

**Separate content transfer.** Replicate small manifests independently from large
objects. Fetch verified chunks selectively, persisting progress across contacts.
Resources carry transfers; they do not replace persistent object inventories.
Custodians may carry data across disconnected regions without local subscribers.

**Separate approval from ordering.** Who may authorize membership changes is
configurable: any member, designated administrators, k of n, or all administrators.
The initial contract selects one fixed owner as approver and ordering authority
while data remains multi-writer; delegated/threshold approval remains later work.
Approvals alone do not establish an order or safe
failover. Signed predecessor references expose conflicting histories; recovery
must resolve them, not choose by wall clock. The effective change must align
authorization with the selected security protocol's key transition.

**Make evidence precise.** Bind authenticated receipts to record/content,
recipient incarnation, membership version and meaning; emit them after the
required durable commit. Gossip within the authorized scope and merge valid
evidence without recursive acknowledgments. Report received/pending/expired/
removed targets against the record's explicit target set. Receipt differs from
current availability, continuing custody, application success and human read
state; one receipt does not release obligations to other targets.

All members need not be online or current to send. Cached membership permits
disconnected operation and authenticated repair; a signed snapshot cannot prove
that no newer state exists. Strict freshness requires membership changes and
new admissions to share one authoritative order, with encryption excluding
removed recipients. A separate freshness check has a check/send race.

Return authorized corrections and retry stale attempts under the same logical
record ID where permitted. Distinguish new attempts from evidenced earlier
admissions. The initial cached profile cannot prove that an unseen stale record
predates revocation: it requires current-author reauthorization or explicit history
import. Retained local acceptance permits that receiver's historical restoration.
Corrections cannot
undo disclosure, and even strict admission cannot retract ciphertext accepted
before removal. Carry compact authenticated control references, fetching private
updates through encrypted exchanges; public service announces do not carry rosters.

## How applications consume it

| Application | Shared mechanism | Application rule |
| --- | --- | --- |
| Messaging | Records, device membership, receipts, object references | Timeline, attachments, optional read state and history import. |
| Telemetry | Authorized streams, gap repair, subscriptions | Archive every sample or retain latest state; declare gaps/coalescing. |
| Shared documents/configuration | Operations, dependency retrieval, snapshots | Explicit merge/CRDT/conflict rules. State convergence need not require every historical operation on every device. |
| File/software distribution | Catalog records, verified chunks, availability | Select content; validate release authority before installation. |
| Commands/work distribution | Authenticated intent and outcome records | Designated executor, expiry, preconditions and durable idempotency/fencing. Receipt is not execution. |
| Live audio/video | Authorization, rendezvous and security setup | Separate deadline-based media path; discard stale frames. Recordings use object transfer. |

Applications may use addressing/security without durable replication, or record
replication without shared-state merge. A delivery acknowledgment quorum is not
consensus or exclusive work ownership.

For example, a telemetry app publishes samples to an authorized stream. An archive
requires every sample, while dashboards reconcile snapshots. Dashboard receipts
cannot cancel archive obligations; compaction must respect both policies. The
sensor need not gain read access to the archive by publishing to it.

## Security and sources analyzed

| Source/direction | Implication |
| --- | --- |
| [RFed channel ingress](https://github.com/jrl290/RFed/blob/14716b9277924e2a44e6e56a3f094fdf02865358/rfed/src/destinations.rs#L2787-L2855), [sync](https://github.com/jrl290/RFed/blob/14716b9277924e2a44e6e56a3f094fdf02865358/rfed/src/sync.rs#L355-L420), [queue](https://github.com/jrl290/RFed/blob/14716b9277924e2a44e6e56a3f094fdf02865358/rfed/src/deferred_queue.rs#L290-L323) | Opaque blobs and inventory/fetch are reusable beyond chat; Distro remains LXMF-coupled. Bound manifest exchange; separate object deduplication from delivery completion and pull from durable acknowledgment. Local interest must not be the only reason for custody. Do not inherit [name-derived channel keys](https://github.com/jrl290/RFed/blob/14716b9277924e2a44e6e56a3f094fdf02865358/rfed/src/channel.rs#L39-L41) as the authority model. |
| [Sesame](https://signal.org/docs/specifications/sesame/) | Generic asynchronous session management for independently addressed devices. Useful for pairwise envelopes and device repair, but encryption costs grow with recipients. Authenticated directory freshness and durable federation remain our responsibility; Sesame does not establish shared group membership. |
| [MLS RFC 9420](https://www.rfc-editor.org/rfc/rfc9420.html), [architecture RFC 9750](https://www.rfc-editor.org/rfc/rfc9750.html#section-6.9) | Protects arbitrary application bytes. Efficient group encryption is attractive, initially one client per endpoint. Epoch ordering, key packages and recovery complicate partitions. Eventually consistent delivery is allowed; conflicting commits require resolution. MLS supplies neither durable replication nor application merge rules. |
| [MLS virtual clients](https://datatracker.ietf.org/doc/html/draft-ietf-mls-virtual-clients-01) | Aggregates devices behind one member, but shared secret state and coordinated removal add complexity. Later experimental option. |
| Personal mailbox/broker, [DTN](https://www.rfc-editor.org/rfc/rfc9171.html), [CRDTs](https://arxiv.org/abs/1805.06358) | Mailboxes simplify topology but concentrate availability; DTN informs custody/lifetimes; CRDTs supply selected merge rules. None alone solves addressing, authorization and durable synchronization. |

Outside producers that must not read the group need a separate ingress profile:
independent recipient envelopes, or an authorized member accepting and
republishing submissions. The latter sees plaintext and becomes a dependency.
Preserve original producer authentication. MLS external senders can send certain
proposals, not ordinary application payloads; joining grants membership.
[RFC 9420](https://www.rfc-editor.org/rfc/rfc9420.html#section-12.1.8).

Keep OpenMLS as a host reference experiment; select production security profiles
and implementations only after embedded feasibility, security and
transactional-storage evaluation. MLS is the preferred private protocol; its types remain outside
generic addressing and synchronization.
Adapters must expose
epoch/session dependencies and recovery limits. Transport encryption to a relay
alone is insufficient for group end-to-end confidentiality; subscriptions and
namespaces alone cannot isolate key holders.

Re-encrypting historical content into a newer MLS epoch, or adding a recipient
key wrapper, may expose it to newly added readers. The initial contract therefore
preserves the original reader
audience or uses recipient-specific authenticated repair/history transfer.
Surviving ciphertext alone need not remain decryptable after key deletion.

## Policy defaults and next proof

Freshness, approval, protection, storage assurance and retention are independent
axes with a limited set of supported combinations. Group policy establishes
minimums; negotiation cannot silently weaken them. A high-security live stream
can be transient, and a long-lived archive can hold opaque content without keys.

The initial messaging bundle uses cached membership, one fixed owner, optional
new-device history, 14-day delivery obligations
from initial durable admission, one best-effort durable custodian and
oldest-content-first eviction. Strict reserved custody instead needs capacity
reservation, completed handoff/release or rejected admission. Last-copy eviction
can defeat delivery in the best-effort profile. See the
[matrices and recovery rules](policies.md#storage-assurance-and-retention).

Bound payload, receipt, authority and key history separately. After necessary
history expires, use authenticated checkpoint/rejoin recovery while preserving
unsent work. Restored devices need new incarnation/recovery handling. Delivery
requires surviving content and keys, authorization and sufficient future contact;
semantic convergence additionally requires a merge rule.

Prove reuse with two independent consumers: chat and a headless telemetry client
with archive and snapshot subscriptions, plus an optional opaque relay. Preserve PRNS's
[borrowed network](../../../applications/services/lxmf/src/direct.rs) and
[caller-owned commit](../../../applications/services/lxmf/src/mailbox.rs) boundaries,
not its LXMF-specific schema. Keep runtime dependencies outside the mechanism;
bounded/no_std-friendly design does not itself qualify embedded support.

Validate partition/rejoin, crashes around storage/cryptographic state, lost replies,
removal, storage pressure and checkpoint expiry. Exercise both consumers through
Python transport nodes; measure fan-out, reconciliation, memory and airtime before
selecting limits, packaging or a core extension. Follow the
[embedded support and release gates](embedded.md) and
[contract gates](v0.md#9-work-that-makes-this-implementable) for the allocation-free
core, profile selection, recoverable flash, wire specification and transport slice.

Source reviews: Python Reticulum 1.5.5 (`e40191b3`), RFed `14716b9`, PRNS
`b7a0aa3` for the architecture and `4a2d22d4a` for current host/announce seams.
Static design findings do not validate the proposed protocol.
