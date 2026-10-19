# Native group runner

`prns-groups-runner-native` owns application scheduling, authenticated enrollment
and projections for native groups. See [authenticated enrollment](enrollment.md)
for creation, invitations, joining, security boundaries and recovery. It borrows an existing Reticulum host
and runs storage jobs on the caller's existing serialized database lane. It creates
no node, database, executor or thread. It is a native `std`/Tokio adapter; the
[text profile](../../profiles/text/README.md) and core remain allocation-free
`no_std`. The existing MLS and generic service retain their bounded `no_std + alloc`
implementation and are not made allocation-free by this adapter.

The [generic record API](generic-records.md) supplies publication, retained record
queries, durable changes and delivery progress beneath text helpers. The
[shared synchronization progress API](progress.md) exposes bounded scheduling
observations without confusing them with authenticated device receipts.

## Ownership and recovery

The database owner holds `GroupManager` (or the legacy single-group `StoreSession`),
accepts bounded `Job`s through `Submit`,
and executes each to completion. `Client` is a borrowed command interface; it has
no authority to stop the node. `admit_send_text` validates and queues synchronously
under the application's lifecycle admission guard; its returned future only waits
for the result. Once a send job is admitted, dropping its response future does not
cancel the durable operation. The storage owner wakes delivery after commit even
if the caller has dropped that waiter. The caller retries the same
32-byte request ID and body to recover the original message. New IDs come from
`create_request_id`: `PGR1`, an eight-byte big-endian creation time in seconds,
and 20 bytes of OS entropy. Reusing the ID with different content fails. Restart
opens the persisted MLS state and exact output. An expired/unknown legacy ID
returns `RetryExpired`; callers must not silently mint a replacement. Exact
retained legacy sends remain replayable.

The legacy single-group `run` is a caller-owned future. It polls peers with bounded
requests/bytes and backoff, persists inventory cursors, repairs pending device receipts and serves
incoming requests concurrently. Every database operation finishes before the next
network wait, allowing reciprocal synchronization to progress without holding a
service lock. Cursors are performance checkpoints; losing one safely rescans
durable deduplicated envelopes. A full database lane delays configuration and
checkpoint work with bounded retry intervals; it does not terminate the runner.
The caller can stop during startup or retry waits. Stop/drop closes owned group
links, not the shared host. The composition must fence new admission and drain accepted store jobs before
destroying or resetting its database.

`run_multi` serves enrollment and multiple groups on the same borrowed host.
`run_multi_scoped_with_announcements` accepts a `NativeServiceScope` and verifies
the store identity against it, allowing multiple independent identities to share
one host. Its sync/enrollment transports authenticate using that explicit local
identity. The caller still owns catalog admission, scoped storage and the complete
Reticulum retirement barrier; stopping the runner alone is not that barrier.
Enrollment and content admission alternate with round-robin peer selection.
Two caller-owned outbound futures can progress concurrently, with at most one
active attempt per physical identity across all groups and enrollment. One
unavailable peer cannot fill both slots. Incoming enrollment and content requests
progress independently. No additional worker or executor is created.
Storage calls are fenced to the locally committed control head. Established
members can synchronize using cached membership while the owner is offline;
this is not a global membership-freshness guarantee.

Both runners use the allocation-free core's `SyncPolicy`. The carrier reads the
selected interface's configured bitrate, route hops and measured Link RTT.
Unknown or routed paths remain conservative even with a fast local Bluetooth
hop. Small constrained batches and bounded, byte-aware waits replace the former
ten-second whole-pass cutoff. `run_multi_with_policy` supplies caller-selected
policy; `run_multi` uses defaults. The legacy runner also preserves its caller's
hard work budgets and timing floors.

Default fast direct paths allow up to four envelopes per pass and two-second
active spacing; constrained paths allow one envelope and fifteen-second spacing.
Idle spacing backs off to ten seconds on healthy fast direct paths and five
minutes on constrained/unknown paths. Failures back off to five minutes on either.
Positive jitter spreads recovery contacts. Fresh durable work can shorten an
idle or failure delay to the contact-spacing floor without resetting failure
history or opening another active attempt for the same peer. Unchanged membership replies remain idle. The multi-group runner announces its two fixed
service destinations every six hours plus jitter after startup, following the
[announce strategy](../../transport/reticulum/docs/announce-strategy.md).

For live preferences and manual announcements, create `announce_control(interval)`
and pass its request receiver to `run_multi_with_announcements`; retain the
returned `AnnounceControl` in the application owner. `set_interval(None)` disables
automatic announcements, including startup. Changing an interval does not
announce immediately. `announce().await` requests both existing destinations
through the same bounded scheduler and returns local admission, cooldown, busy
or unavailable status. Startup, periodic and manual attempts share a 60-second
cooldown, including failed/uncertain attempts. The caller owns preference
persistence; the app chooses one hour by default. The cooldown is per runner
lifetime and is not a durable restart-storm fence. The named-identity app disables
these per-runner timers and uses the [durable shared scheduler](../../../prns-identities/runtime/README.md#shared-announce-scheduling)
across its service sets; it is a separate composition responsibility.

Both runners share prompt delivery over negotiated group Submit requests. A
bounded scan of durable authored-record obligations selects exact retained MLS
ciphertext for each intended peer. The receiver commits the record and its MLS
receipt before returning that exact receipt on the submitting Link; the sender
validates and stores the receipt before updating delivery. Pull-created receipts
also return on the existing fetch Link when the request/byte budget permits.
Transport response admission alone never means durable device delivery.

There is no second durable message queue: process-local scheduling holds only
inventory positions, backoff and a temporary capability observation. Restart
reconstructs pending work from records and receipts. New records take precedence
over retry rotation; old unacknowledged records rotate fairly. After at most four
completed Submit exchanges for a peer/group, the next eligible attempt performs inventory
repair even if authored work remains. Incoming requests and active outgoing
Links continue progressing while storage queries wait. Both modes retain two
outgoing slots; multi-group admission also serializes each physical peer across
enrollment and all its groups.

An unsupported or unanswered read-only capability probe temporarily selects
legacy pull for five minutes before probing again. A legal envelope that cannot
fit Submit's larger framing overhead also falls back to pull. Successful push
returns promptly without opening a second Link for an unnecessary poll. A new
exchange still establishes a new Link; a persistent cross-exchange Link pool is
not implemented. Failed delivery retains its obligation and uses normal backoff.
No per-message announce is sent. These bounds do not establish a LoRa duty-cycle
guarantee or retain knowledge of downstream bottlenecks beyond current path
observations. See the [isolated LoRa trial procedure](../../transport/reticulum/docs/lora-qualification.md).

Completed inventory scans retain their covered head. An unchanged poll needs
only a snapshot request and does not rewrite the identical stored checkpoint.
Both record and receipt passes scan only a new suffix. Changed remote generation
or regressed head falls back to a full scan. Data added beyond a frozen head
waits for a later snapshot; the idle bound is not a delivery-latency bound.

Native cursor rows bind progress to the local control head, authorization era
and store incarnation in addition to the existing namespace/peer key. A changed
context invalidates coverage without accumulating old rows. Earlier unbound
rows trigger one conservative rescan on upgrade. The new local row wrapper is
not readable by older native binaries; downgrade requires discarding only these
disposable cursor rows, never group records or keys. Accepted records, receipt
repair/output and cursor persistence retain their transaction order. Ordinary
expiry preserves remote coverage; arbitrary deletion of live local records
would need explicit repair.

`Client::maintain(clock)` runs one bounded pass on the existing storage lane.
The sync loops invoke it, and send/receive admission performs a pass before new
work. Each group pass selects at most one expired record and four service
operations. The initial text policy retains content for 14 days; expiry may
remove undelivered content, but never fabricates a delivery receipt. Cleanup
advances a durable time floor, removes record/retry/output facts atomically,
and invalidates inventory snapshots. The oldest pinned operation is prioritized
when eligible, allowing the native receipt prefix to advance. New work reserves
two backend receipt slots for cleanup; the final slot may only be spent when
retiring the oldest outstanding output. Existing completely exhausted stores
may require a separate recovery procedure; an upgrade cannot invent free slots.

`StorageFull` preserves accepted work and the caller's immutable request. Retry
after maintenance frees capacity, using the same ID and body. Limits constrain
concurrent retained work rather than lifetime sends. The 14-day request fence
prevents retired sends from becoming new publications after clock rollback.
These policies reclaim logical objects and reusable database pages; they do not
shrink the redb file, reserve filesystem space, encrypt phone storage or guarantee
physical erasure of old secrets. The core retry-token policy remains allocation-free.

Messages have durable local insertion order and receipt projections. Queries
return bounded pages plus a durable change revision. Callbacks and polling can
refresh projections; they are not the only record of changes. Delivery targets
exclude the publishing endpoint. All members remain MLS readers.

Legacy fixed-membership fixtures and the headless regression peer are documented
in [development tools](development.md). Normal app enrollment uses authenticated
creation, invitations and joining; it requires no fixture transfer.

## Remaining limits

Authenticated additions are supported by the new native enrollment path. Removal,
blind custodians and mobile key protection remain separate work. Storage exhaustion
backpressures new work; only expired content is reclaimed. The service reserves logical envelope
capacity for recipient receipts when accepting a record. An authenticated expired
record leaves a tombstone so dependent receipts can be consumed without stalling
synchronization; these bounds do not reserve filesystem or physical flash space.
Automatic polling does not imply background execution under mobile OS suspension.

Host tests cover simultaneous automatic exchange between independent Reticulum
nodes, transient database-lane saturation, offline catch-up through a member while
the author is unavailable, receipt convergence and persistent reopen. Separate
checks drop an admitted caller, exit a process immediately after acceptance, and
reopen a received record before its pending receipt was emitted. These establish
native recovery and scheduling over the test carrier, not phone/radio
qualification, Python-router interoperability or physical power-loss durability.
The prompt-delivery regression parks reciprocal polling behind a 300-second
floor, drops an admitted send's result waiter, and requires the message and
validated receipt within 12 seconds without any new receiver request reaching
the author. It then reopens both stores and delivers another already committed
record with the same ID. Scheduler tests cover failure-backoff preservation,
rotation, mandatory repair after four pushes, temporary capability expiry and
active Link progress during blocked storage queries.

## Announcement control

`run_multi_with_announcements` accepts the receiver from `announce_control`.
The controller updates a bounded periodic interval at runtime (`None` disables
startup/periodic announces) and admits manual requests through a one-slot queue.
Manual results distinguish local `Requested`, `Throttled`, `Busy`, and
`Unavailable`; none establishes peer receipt. Startup, periodic, and manual
batches share a 60-second cooldown, and configuration changes emit no traffic.
The existing `run_multi`/`run_multi_with_policy` APIs retain automatic announces
and the six-hour generic default. See the [announcement policy](../../transport/reticulum/docs/announce-strategy.md)
for the mobile app's persisted one-hour default and optional intervals.

## Bounded retry acceleration

Both native runners inspect durable outbound work before applying a failed-peer
wait. A record newer than the peer's attempted-position watermark may shorten
that wait; replaying an application request or rotating older records cannot.
After a successful repair, retained outbound work resumes at active spacing
instead of receiving an idle polling delay. Every four push attempts still yield
to an inventory/receipt repair turn, including when fresh work is waiting.

Acceleration preserves the failure exponent and ceiling. It cannot precede the
last attempt's completion plus the interface's active interval (normally 15
seconds for constrained/unknown paths, two seconds for a measured fast direct
path). `RunnerOptions.retry_initial` can raise this floor; `poll_interval` is the
idle recovery floor, so it does not park a fresh send. Repeated hints coalesce by
shortening an existing deadline once, never moving it later. Ordinary application
wakeups and announces only prompt another durable-state check.

An identified inbound request may also shorten a failed wait **when outbound
work remains**. The runner checks complete framing, reader membership, transport
identity, group, control head and store incarnation, then rechecks configuration
after the response and before applying the hint. This establishes peer
reachability; an error response does not establish content acceptance. Empty
outboxes do not produce reciprocal repair loops. The native hint queue holds at
most 32 distinct authorization scopes; overflow/cancellation falls back to normal
polling. Multi-group contact state remains capped at 256 entries, with one active
exchange per remote identity and two outbound exchanges overall. Control-head
changes preserve attempted-position watermarks and contact spacing; a different
store incarnation invalidates cursor positions without erasing the contact floor.
These are disposable hints, never ownership of ciphertext, records or receipts.

The generic `PeerSchedule` acceleration rule remains `no_std` and allocation-free.
Durable writes, request IDs, authenticated receipts, retention and crash recovery
are unchanged. Host timing tests establish scheduling behavior, not LoRa latency
or a promise that an unavailable peer will receive a message within a fixed time.

## Opt-in transfer diagnostics

For a controlled foreground investigation, set `PRNS_GROUP_DIAGNOSTICS=1` in the
process environment before startup. The native runner then emits metadata to the
`prns.groups.diagnostics` log target: static operation/error categories and frame
lengths. It never formats payloads, keys, identities, group IDs or arbitrary error
strings. A zero length means no frame length was available at that stage.
Enrollment admission, storage handling and response queueing are separate events;
queueing a response does not confirm delivery. Request Resource and response
Resource failure categories remain distinct.

Native embeddings can instead call `enable_metadata_diagnostics()` before
starting the runner and install a logger accepting the exact group targets. The
app supports explicit iOS launch environment flags and Android startup property
`debug.prns.groups=1`, with bounded stderr/logcat sinks. Default launches remain
silent. Producer stage limits and general, delivery and schedule sink limits are
independent; restarting the node does not reset a process's budgets. A third-party
logger can filter events, so a heartbeat/readiness check is required before
claiming complete observations.

The separate `prns.groups.delivery` target carries observed local send results,
push attempts and durable record/receipt transitions. Send results have a
128-event budget; pushes and receipt transitions have independent 256-event
budgets, so idle discovery and inventory setup cannot exhaust them. Their group,
record and peer correlation uses process-salted opaque tags matching the existing
local-send diagnostic; no original identifier, ciphertext or application content
is logged. The app reserves 1024 sink lines for this exact target separately from
the general diagnostic cap. The sink still requires opt-in.

The independent `prns.groups.schedule` target emits completed attempts,
shortened deadlines, admissions and recovery-hint dispositions, capped at 1024
events in total. The `stage=schedule` categories are
`completed`, `fresh_work`, `pending_work`, `peer_recovered` and `admitted`.
`due_in_ms` and `floor_in_ms` are remaining monotonic durations; `failures` retains
the exponent history. Candidate decisions include `pending`, `fresh` and `repair`;
completion has `pending=unknown` and no new candidate decision (`fresh=false`,
`repair=false`). These events correlate a group/peer scheduling scope, not an
exact application message or physical radio transmission.

`stage=recovery` records use only `category`, `sequence`, `group` and `peer`.
They distinguish service revalidation rejection, queue offer/coalescing/fullness,
stale or unavailable scope, missing candidate, an already active peer, no failed
wait, granted credit, and the credit's eventual consumption. Consumption can
report no pending work, push probe suppression, fresh work taking precedence,
actual acceleration, or no shortening. A raw offered or credited hint is not an
accelerated deadline, admitted exchange, accepted message or receipt. Hints are
still scoped to the exact group, control, store incarnation, endpoint and
authenticated identity; an exchange in another group does not change the target
group's wait. Local interface restoration alone does not grant recovery credit.
For the multi-group runner, `peer_active` now means the exact-scope evidence is
held behind that physical peer's occupied slot; it does not accelerate or admit
work while the slot is occupied. A subsequent control/store change clears the
held credit. The standalone runner still discards a hint for its already-active
attempt, whose completion determines the next schedule.

Sequence assignment and emission are serialized. The nonlogging
`metadata_schedule_snapshot()` returns the last emitted sequence and producer
health, which becomes false on exhaustion, entropy failure or poisoned ordering.
The app exposes this watermark/readiness in its opt-in trial heartbeat; its
independent `prns.schedule` sink reserves 1026 lines for the startup probe, 1024
events and the producer cap marker. A complete scheduling window requires the
capture to match surrounding heartbeat watermarks, including both record stages,
with no relevant producer or
sink cap, missing sequence, logger failure or heartbeat gap. Delivery evidence
has separate budgets and must still use exact durable record/receipt mappings.
