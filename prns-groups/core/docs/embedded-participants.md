# Embedded group participants

**Design and integration plan, not shipping board support.**
Make a resource-qualified ESP32-S3/PSRAM board a complete, independently enrolled
group endpoint. It should publish opaque application records, receive and retain
them, reconcile receipts, and repair another member while the original author is
offline. Text, telemetry, shared state and commands use the same mechanisms.
Start with E290; smaller boards receive an explicit supported role and profile
only after measurement. Keep Reticulum's core wire protocol unchanged.

## Roles and the first milestone

| Role | Keys and durable state | What it proves |
| --- | --- | --- |
| Transit router | Reticulum routing state; no group keys or group record store. | Packet forwarding only. A BLE-to-LoRa bridge does not acknowledge group custody. |
| Enrolled member with retained replicas | Independent endpoint/MLS credential, group control state, records, exact ciphertext, receipts and recovery fences. | Its own durable acceptance; it can serve retained authorized ciphertext to another member. |
| Blind custodian | No MLS decryption keys; bounded opaque object store and authenticated storage authorization. | A separate custody promise, if implemented. It cannot issue an MLS recipient receipt or validate application plaintext. |

Build the **enrolled member** first. This applies the planned record service semantics and
gives capable embedded nodes the same protocol role as a phone. Owner operations
are a separately qualified workload, not a permanent phone-only requirement.
Phones may initiate a board's invite approval through authenticated device
management, but the board must generate and retain its own secrets and decision.
BLE pairing or proximity alone must not enroll it.

The planned inventory filter serves only
the original author or selected delivery targets. Initially enroll the board as
an ordinary visible device and include it in the intended delivery targets.
Its receipt means **the board accepted the record**, never that a phone received
it or a human read it. Merely adding an MLS reader outside that target set does
not create a replication service. A future authenticated replication scope can
select caches independently of delivery targets; do not silently alter either
the encryption audience or user-facing delivery evidence to approximate it.

This first member retains and forwards within the existing record deadline and
authorization era. It does not promise an extra replica count or a custody lease.
The selected Add policy ends old-era forwarding, so it can interrupt delayed delivery;
it must not re-encrypt old content for an expanded audience. Removal/rejoin,
ownership transfer and delayed-content repair retain their existing open design
and implementation gates.

## Reuse and missing seams

The reusable service (separate integration),
MLS adapter (separate integration) and
[coordinator/secure store](../../store/docs/secure-flash.md) are designed around
`no_std` with bounded allocation; the service is a separate integration. The [core](../README.md) is allocation-free; the text codec is a separate
integration. A complete firmware member must compose the service, authenticated enrollment
and network reconciliation; the separate local storage fixture is insufficient.

| Boundary | Next implementation |
| --- | --- |
| Portable policy | Keep capacity accounting, retry/retirement fences, clock admission and interface-aware scheduling decisions in the allocation-free core. Inputs are scalar bounds/observations; outputs are bounded work and deadlines, with caller-owned state/time. |
| Enrollment | Implement bounded owner/member transitions and signed control validation. Stage control state, exact Welcome/Commit replay and MLS changes through `ApplicationTransaction`; use bounded codecs and explicit caller-provided entropy/time. Keep native catalog persistence in a separate adapter. |
| Synchronization | Implement bounded framing, cursor transitions and inventory validation. Separate storage calls from network waits, and portable commands/events from native `HostClient`, Tokio timeouts and channels. Native carrier integration is separate from the embedded adapter. |
| Embedded execution | Add a board runner using the existing Embassy Reticulum node/Links/requests/Resources and one serialized crypto/storage worker. Supply finite work queues and caller-owned buffers. Do not bring Tokio, redb, a second node or the native text projection into firmware. |
| Platform services | Supply owned flash geometry, protected key access, hardware-seeded entropy, qualified time and recovery status. Reuse the secure backend; replace the two-peer qualification scenario with one real endpoint. |
| Application effects | Expose committed generic records/change cursors. An actuator needs its own durable execution/result protocol; a group receipt cannot mean a command ran. Text decoding remains an optional profile. |

A shared scheduling policy is useful before the full transport extraction: both
native and board executors can apply the same byte/request limits, idle backoff,
retry spacing and announcement cadence without sharing an executor. Maintain
separate monotonic scheduling time and trusted record-expiry time. A fast local
BLE interface does not establish the capacity of a path continuing over LoRa.
Use explicit route capability evidence where available and conservative unknown
path policy otherwise; remote capability claims never relax local hard limits.
Account for enrollment/control frames as well as record transfers. Timer changes
alone do not reduce repeated Link setup or control-message airtime.

The allocation-free [`core::sync` policy](../src/sync.rs) now provides
`PathCapabilities`, `SyncPolicy`, `SyncTuning`, `PeerSchedule` and `SyncActivity`.
Native runners are a following integration. They must bound concurrent
outgoing attempts and preserve durable cursors separately from disposable
scheduling state. This is shared policy, not yet a portable
enrollment or network execution state machine.

The first embedded carrier can reuse bounded whole envelopes; there is no need
to introduce file streaming to prove short records. Its buffers must accommodate
the selected profile's largest ciphertext and control frame. Larger application
objects need a later chunk/object-transfer profile and separately bounded decode
work; Reticulum Resource segmentation does not make the current whole-envelope
MLS/service APIs stream automatically.

## Resource and admission contract

Publish a versioned board/profile contract before promising membership. It must
cover per-device and per-group limits, required security features, supported
control operations and content retention. This stage does not provide enrollment
capability negotiation; initially select a common explicit test profile at
provisioning, then bind capability negotiation to authenticated enrollment. A low-capacity node must reject an incompatible invite or record
before committing acceptance; it must not silently truncate targets or lower
security to fit.

| Budget | Required accounting |
| --- | --- |
| RAM/stack | One active MLS job; bounded snapshots, staged changes and old/new catalogs; ingress/egress buffers and queues; internal stack/radio reserve separately from PSRAM. Preflight control operations as well as application data. |
| Crypto state | Groups, members, KeyPackages, proposals, retained epochs, forward/skipped-key distance and exact pending outputs. These limits must cover worst-case admitted concurrent senders, not only consecutive messages from one peer. |
| Durable capacity | Bodies and exact envelopes, per-target receipt reservations, operation facts/deduplication, control chain/Welcome replay, retry fences, encrypted catalog and transaction overhead. Bound all counts and bytes; body bytes alone are insufficient. |
| Recovery/maintenance | Spare arena fits the complete live set plus the largest admitted publication; reserve catalog/capsule and receipt space to retire the oldest output. Never accept work that consumes the only cleanup path. |
| Airtime/work | Bounded requests, envelopes and bytes per contact, one peer transfer initially, progress checkpoints, control priority with data fairness, yielding between flash/crypto jobs, and measured LoRa duty-cycle/backpressure behavior. |
| Time | Trusted time source with uncertainty plus a persisted monotonic expiry floor. An unauthenticated peer clock cannot expire keys/history. Reboot without a usable clock returns time uncertainty; it cannot invent elapsed offline time. |

Admission must atomically reserve the work needed to finish an accepted record's
receipt and recovery path. Logical service quotas and spare MLS output slots do
not reserve every physical flash object or byte. The board adapter must account
for the backend too. Enrollment must likewise
reserve unresolved Add/Welcome work; an expired invitation cannot discard an
already-committed Add. Restore uncertain commits before any new work, return
exact output after recovery, and retain permanent retry floors when old results
are removed. Every native or embedded consumer must preserve these guarantees.

Under pressure, retire definitely expired application content in bounded batches;
reject new promises if capacity still cannot be guaranteed. Never evict live
crypto state, uncompleted control transactions or replay fences oldest-first.
If applications want best-effort early eviction, make that a separate declared
retention policy with explicit loss, not a change to an existing custody promise.

For the strict secure backend, only release plaintext/output or emit acceptance
after publication, required predecessor capsule erasure/verification and recovery
complete. Logical record retirement and secure key retirement are different
events. Offline/unpowered nodes cannot guarantee wall-clock erasure. A plaintext
fixture wrapping key gives no protection against physical flash inspection;
saved full-flash rollback and archived key wrappers remain outside the current
security guarantee even with a protected device key.

## Initial participant profile

Use the [embedded guide](embedded.md) for current support, resources and fixture
limits. Start full-member integration with one group, up to four members, short
records, one network transfer and one crypto/storage job. These are trial bounds,
not qualified capacity. Derive retained-record, output, receipt and catalog limits
from maximum encoded state and reserved maintenance capacity before freezing a
profile. Two local fixture peers do not establish networked custody or runtime
headroom alongside loaded radios.

## Build and qualification sequence

1. Extract the portable enrollment and reconciliation seams; host-test the same
   profile with the NOR emulator and native backend. Keep allocation-free core
   target checks, bounded decode/work checks and mixed-profile rejection tests.
2. Run one simulated board member plus two independent native endpoints. Enroll,
   publish from both kinds of endpoint, disconnect the author, catch up the third
   from the board, and reconcile receipts after restarts. Exercise full capacity,
   clock uncertainty, skipped-key exhaustion and membership interruption. Inject
   crashes across admission, receive, receipt, cursor, compaction and retirement.
3. Integrate one real E290 endpoint on owned durable flash alongside existing
   radios, with protected provisioning as a separate security qualification.
   Measure maximum admitted state, stack/heap fragmentation, flash latency/wear,
   watchdog margins and sustained BLE/LoRa load. Then repeat with electrical cuts.
4. Qualify phone → BLE → board → LoRa → board/phone paths with alternative routes
   disabled or isolated and radio counters captured. A successful group receipt
   does not by itself prove which interfaces carried it. Exercise catch-up after
   the author is gone, not merely two live phones forwarding through boards. The
   LoRa trial procedure (separate integration)
   describes reversible Bluetooth isolation and a radio-disabled negative test.
5. Add independent replication scope and optional explicit custody leases when
   needed. Define bounded authenticated admission, byte/expiry promises,
   renewal/release, loss reporting and membership revocation before a blind
   custodian issues evidence. Keep custody evidence separate from member receipt
   and application execution. This is a new protocol slice, not a firmware flag.

The separate E290 secure fixture can supply useful building blocks; full embedded
participation remains a further integration
after the portable seams and combined resource contract are implemented.
