# Continuous phone trial observation

`observer.py` records bounded native diagnostics independently of an agent's
polling schedule. Every accepted event and host heartbeat is appended and
`fsync`ed. A separate gate process continuously updates an atomic result file;
missing coverage makes a gate **unqualified**, not passed or a claimed network
failure. The tool does not send messages, operate the UI, read private app data,
clear log buffers, modify board settings, or create development groups.

## Enable and start capture

Use a freshly installed, source-qualified build with the opt-in diagnostic sink
and the independent native heartbeat. Both apps must be foreground for this
trial. The native heartbeat is exactly:

```text
[prns.trial] stage=heartbeat session=<process-opaque-token> sequence=1 elapsed_ms=0 groups_ready=true bluetooth_ready=true routes_ready=true schedule_ready=true schedule_sequence=0
```

The heartbeat is emitted every five seconds, with its own bounded budget and a
`category=heartbeat_limit` exhaustion marker. `session` changes on ordinary app
restart. Heartbeat sequence counts only heartbeats. Normal group and Bluetooth
budgets cannot consume this reserve.

On Android, set the exact diagnostic properties **before the planned ordinary
app launch**:

```sh
adb -s DEVICE shell setprop debug.prns.groups 1
adb -s DEVICE shell setprop debug.prns.bluetooth 1
adb -s DEVICE shell setprop debug.prns.trial 1
adb -s DEVICE shell setprop debug.prns.routes 1
```

After launch, verify the current PID with `adb -s DEVICE shell pidof
rs.reticulum.prns.dev`, then attach capture with that PID:

```sh
python3 tools/device/group-trial/observer.py capture \
  --android DEVICE --pid APP_PID --output RUN/android.jsonl
```

The capture filters exact tags `prns.groups`, `prns.bluetooth`, `prns.trial`, `prns.routes`, and `prns.schedule`
and pins the app PID. It does not clear the log buffer. By default, at most one
buffered line can precede attachment. For scheduling gates, attach promptly with
`--android-replay-lines 5000` to retain the startup probe and sequence prefix.
The explicit replay count is bounded to 1–10,000 lines and recorded in the
capture header; the existing total event/time bounds still apply. Buffer
eviction or a larger startup backlog can still lose the prefix, which makes
scheduling unqualified. Do not clear logs to obtain a prefix.

Replayed events have host attachment-time observations, not their original
event times. Finish replay and require fresh native heartbeats before starting
a case; do not use replayed startup events as latency evidence. The scheduling
gate still requires the complete numbered prefix and matching watermark.
An app restart requires a new capture and new mapping; the old capture must not
silently follow a different process. Record the original values before opting
in, then restore all four properties to those values after the trial (including
an empty value when originally unset). Follow that with an ordinary app restart
to remove the startup opt-in when the original values were disabled.

On iPhone, this starts a console-attached process with all four diagnostic
environment flags. Include `--terminate-existing` only at the explicitly planned
ordinary app restart; it preserves the app's data:

```sh
python3 tools/device/group-trial/observer.py capture \
  --ios DEVICE --terminate-existing --output RUN/ios.jsonl
```

The iPhone flag names are `PRNS_GROUP_DIAGNOSTICS`,
`PRNS_BLUETOOTH_DIAGNOSTICS`, `PRNS_TRIAL_DIAGNOSTICS`, and
`PRNS_ROUTE_DIAGNOSTICS`, all exactly `1`.
The launch metadata is saved alongside the capture. Do not signal the iPhone
capture or its `devicectl` child to stop it: a console signal can reach the app.
End it with the coordinated ordinary app restart without diagnostic flags.
The recorder never terminates `devicectl` at its duration/event cap; it records
the fault and drains output without keeping further native events until the
app closes the stream. Android logcat can safely be detached at its cap.

Before acceptance, require fresh native events and an all-ready heartbeat from
the captured process. A successful launch response with the requested flags is
not proof that an already-running process adopted them or that its console was
attached. If a replacement launch produces no native events, preserve that
failed capture. During setup only, stop the exact app process and independently
confirm its absence before starting a fresh flagged capture. If it immediately
relaunches, do not assume the next launch is fresh. The October 16 trial obtained
a clean launch after temporarily disabling the app's Bluetooth interface,
confirming zero peers and then confirming process absence; this does not
establish why the earlier process relaunched. Restore the interface and verify
the isolated topology before starting any acceptance window. Never use this
recovery procedure inside a delivery or outage gate.

The default capture bound is two hours and 20,000 accepted native lines.
Unrecognized console output is discarded rather than copied to an unfiltered
log. Source producers must still uphold the metadata-only contract: a syntactic
key/value filter cannot decide whether an arbitrary value is sensitive.

## Bind the exact message

Do not equate group or record tags from different phones: these are process
salted. Submit each unique marker once and persist the UI action's UTC window,
local saved result, exact unique row and subsequent receipt view. The marker
hash can be saved in derived evidence without copying its text. The native
`local_send_result` event is a caller-observed durable result, not an exhaustive
commit journal; a dropped waiter can omit it.

Bind a narrow single-send window to one opaque record:

```sh
python3 tools/device/group-trial/observer.py bind \
  --capture RUN/android.jsonl --output RUN/case1-sender-map.json \
  --case-id case1 --kind local-send \
  --start-utc ACTION_START --end-utc SAVED_OBSERVATION \
  --marker-sha256 MARKER_SHA256 --ui-evidence RUN/case1-send-ui.json
```

The command requires exactly one distinct native group/record in that interval
and one native process session. Duplicate observations of that record are
allowed. Ambiguity fails rather than selecting the first record. `--kind
incoming` binds `durable_record_received` on the other phone, with an independent
exact-row UI observation and narrow window; if several records arrive, keep the
arrival measurement unqualified until its mapping is resolved. The sender's
exact authenticated receipt alone can establish the receiver's durable receipt
semantics, but cannot supply the receiver's exact arrival time.

The UI evidence is read only to hash it. Its path and hash are retained in the
mapping, alongside the original capture ID and event sequence. Do not edit a
mapping or its UI evidence after a gate has started.

## Run a fixed deadline gate

Make a JSON case specification using the following structure. Times come from
the actual send/On action; taking longer to prepare the observer does not move
the deadline. Paths are relative to the specification. A sender-only receipt
gate is shown; for a complete trial add the receiver stream, its mapping and an
exact `durable_record_received` check. Every declared stream must have continuous
coverage. A stream that requires route or Bluetooth evidence also declares
`required_tags` containing `prns.routes` or `prns.bluetooth`; exhaustion of those
budgets then invalidates its gate. Declare `required_readiness` as a list of
`groups_ready`, `bluetooth_ready`, and/or `routes_ready` as appropriate. Group
readiness is required by default: a live console with an unavailable group
logger cannot qualify a group timeout. Keep the specification immutable while its watcher runs.

The sync-attempt and local-send-result correlation counters are independent.
Only an explicit `prns.groups` limit with `family=sync_attempt` may leave a
delivery-only gate qualified; its result still reports
`sync_attempt_correlation_exhausted` under `diagnostic_limits`. Supported gates
are exact durable send, arrival and authenticated receipt checks. This does not
qualify a complete scheduling/attempt timeline. The stream option
`require_sync_attempt_correlation: true` adds a conservative attempt-budget guard;
checks outside the three durable categories enable that guard automatically.
It does not make arbitrary diagnostic checks supported: general group
`event_limit`, `line_limit` and `ingress_limit` exhaustion is not fully evaluated
for those checks. Keep broader timelines as positive supporting evidence, with
their own limits, until that coverage is implemented. Unlabelled legacy limits,
unknown families, and
`family=local_send_result` always invalidate the gate; never relabel an older
captured event by inferring which counter it probably represented.

```json
{
  "schema": 1,
  "case_id": "case1",
  "start_utc": "ACTUAL_ACTION_UTC",
  "deadline_utc": "ACTUAL_ACTION_PLUS_480_SECONDS_UTC",
  "max_observer_gap_seconds": 15,
  "streams": {
    "sender": {"capture": "android.jsonl", "session": "PINNED_NATIVE_SESSION"}
  },
  "correlation_evidence": [
    {"stream": "sender", "path": "case1-sender-map.json", "sha256": "MAPPING_FILE_SHA256"}
  ],
  "checks": [
    {
      "name": "exact authenticated receipt",
      "stream": "sender",
      "kind": "authenticated_receipt",
      "where": {
        "stage": "receipt_transition",
        "category": "authenticated_receipt_applied",
        "group": "SENDER_GROUP_TAG",
        "record": "SENDER_RECORD_TAG"
      }
    }
  ]
}
```

```sh
python3 tools/device/group-trial/observer.py evaluate \
  --spec RUN/case1.json --output RUN/case1-result.json --watch
```

The watcher writes `observing` until it can qualify success or until the fixed
deadline plus a short coverage grace expires. The grace only allows the next
heartbeat to prove coverage; it **never** extends the message deadline. Exit
codes are 0 passed, 1 observed deadline failure, 2 unqualified, 3 invalid input
or execution error. Results record the frozen specification hash. Once final,
keep that result immutable; a later recovery is a separate evidence result.

A pass requires exact checks before the deadline, verified mapping and UI hashes,
continuous host and native heartbeats, an unchanged native session, consecutive
heartbeat sequences, and no relevant capture fault or delivery/heartbeat budget
cap. A deadline failure requires those same coverage guarantees through the
actual deadline. Both missing native events and an absent observer can otherwise
look like silence, so incomplete evidence stays unqualified. Repeated successful
receipt observations are not unique state-transition counts.

UTC values are host complete-line observations; buffering can delay them. Native
push `elapsed_ms` measures the in-app attempt. Do not subtract timestamps from
independent phone clocks. The host's monotonic clock is recorded too; wall-clock
discontinuities invalidate the gate. Route destinations and interface IDs remain process salted. A public random
request nonce can use a domain-separated unkeyed SHA-256 tag for correlation
across routers; this intentionally links only the observed request, not stable
endpoint identities. Diagnostic coverage does not establish RF
isolation, unique UI rows, board configuration, or power-loss durability: retain
those independent readbacks and checks.

Run the host tests with:

```sh
python3 -m unittest tools.tests.test_group_trial_observer -v
```

## Qualify scheduling separately

Use `gate_kind: "schedule"` in a separate immutable specification. Keep the
existing delivery result and its deadline independent. Scheduling uses the
`prns.schedule` phone tag (Rust target `prns.groups.schedule`), enabled by the
existing group diagnostic opt-in. Its producer has 1,024 numbered events; the
independent sink reserves 1,026 lines for a startup probe, those events, and the
producer exhaustion marker. It does not consume the delivery sink budget.

The heartbeat adds `schedule_ready=true|false` and `schedule_sequence=0..1024`.
The watermark is published after serialized event emission; health becomes
false on producer exhaustion, unavailable correlation entropy, or poisoned
ordering state. Each schedule gate requires a captured startup
`stage=logger category=schedule_probe`, pinned process session, live host/native
heartbeats, healthy scheduling output, and every numbered record from 1 through
the first native heartbeat at or after the checked window. Thus loss of the last
event is detected even without a subsequent scheduling event. Concurrent output
may include consecutive events newer than the heartbeat snapshot; the watermark
is a lower bound. Missing new fields in an older capture makes scheduling
unqualified without changing that capture's delivery result.

Schedule records describe a **group and peer**, using the same process-salted
correlation domains as synchronization attempts. Fields are `category`
(`completed`, `fresh_work`, `pending_work`, `peer_recovered`, or `admitted`),
`sequence`, `group`, `peer`, `activity`, `due_in_ms`, `floor_in_ms`, `failures`,
`pending` (`true`, `false`, or `unknown`), `fresh`, `repair`, and `accelerated`.
Unknown means the decision did not query the durable outbox. Events are emitted
for completion, admission, and actual due-time shortening; silence is not a
claim that no polling or unchanged scheduling decision occurred.

The same ordered stream also carries `stage=recovery`, with exactly `category`,
`sequence`, `group`, and `peer`. It records service revalidation rejection; queue
offer, coalescing or saturation; stale/unavailable scope, missing candidates or
active peers; granted credit; and consumption with or without actual deadline
shortening. A credit may instead find no outgoing work, push probing suppressed,
or fresh work taking precedence. These observations share the 1,024-event cap and
heartbeat watermark with schedule decisions. A missing raw hint record therefore
invalidates stream coverage. Recovery checks may select these exact categories
but cannot claim delivery or add invented due/failure fields. Older complete
captures remain valid under their existing schema; they do not retrospectively
prove whether unlogged raw hints were offered or discarded.

Reuse a hashed message mapping to establish the process-local **group**. It
cannot bind a particular scheduling decision to that record. Every check must
name an exact category, group and peer; delivery/receipt fields are rejected.
For example, keep the existing `streams` and `correlation_evidence` and replace
the check list with:

```json
{
  "gate_kind": "schedule",
  "checks": [
    {
      "name": "fresh work shortened the retry",
      "stream": "sender",
      "max_after_start_seconds": 10,
      "where": {
        "stage": "schedule",
        "category": "fresh_work",
        "group": "PROCESS_GROUP_TAG",
        "peer": "PROCESS_PEER_TAG",
        "accelerated": "true"
      }
    },
    {
      "name": "pending attempt admitted",
      "stream": "sender",
      "max_after_start_seconds": 20,
      "where": {
        "stage": "schedule",
        "category": "admitted",
        "group": "PROCESS_GROUP_TAG",
        "peer": "PROCESS_PEER_TAG"
      }
    }
  ]
}
```

Those timing targets are examples, not library guarantees. Freeze targets and
host action boundaries before the case; a case must first exercise the relevant
backoff/floor state to test acceleration. The usual `evaluate --watch` command
works unchanged. It reports host observation delay and the selected scheduling
fields. Missing checks remain failures only when complete coverage extends to
the original deadline; a missing producer event, watermark, probe, unhealthy
readiness, schedule cap/unavailable/truncation marker, or observer interruption
makes evidence unqualified. Schedule markers unknown to this tool also fail
closed. Unrelated delivery, synchronization-attempt, Bluetooth and route limits
are reported separately and do not invalidate the independent schedule gate.

This proves bounded scheduler decisions, not RF transmission time, a particular
record's admission, or delivery. Preserve exact receipt/arrival gates, settings
and isolation evidence independently. Run both phone suites with:

```sh
python3 -m unittest tools.tests.test_group_trial_observer tools.tests.test_group_trial_schedule_observer -v
```

Android replay argument/filter regressions run with
`python3 -m unittest tools.tests.test_group_trial_android_capture -v`.

## Optional Link handshake diagnosis

Add `--link-diagnostics` to an iOS capture to opt into
`PRNS_LINK_DIAGNOSTICS=1`. On Android, record the original `debug.prns.links`
property and explicitly set it to `1` before an ordinary app restart; restore it
and restart normally afterward. The existing four opt-ins remain independent.
This requires rebuilt phone native code; earlier captures cannot supply Link
coverage. It does not enable a board observer.

The `prns.links` stream correlates core request/proof/deadline phases and host
egress admission using hashed public ephemeral Link tokens. These tokens can
be joined across captures; interface tags cannot. No payloads, private keys or
stable endpoint IDs are logged. Native callbacks use a bounded 64-entry
nonblocking handoff; the independent writer publishes complete records and a
heartbeat watermark. Core/egress producer limits are 4,096/2,048 data events
plus terminal notices; the sink limit is 8,192. Backpressure, gaps, resets,
unknown records or missing tail invalidate complete coverage.

Analyze one fixed, timezone-qualified interval:

```sh
python3 tools/device/group-trial/link_timeline.py RUN/phone.jsonl \
  --start 2026-10-16T14:00:00Z --end 2026-10-16T14:01:00Z
```

It reports positive phases and independently checks the capture prefix, one
native session, ordered producer/sink counters and published watermarks. It does
require independently closing host and native heartbeats, which may arrive in
either order; their phase does not change the checked interval. It does
not bind a Link to a group message, infer missing RF transmissions, or qualify
delivery. `Enqueued`/`Flushed` precedes driver output; committed-ring retirement
and BLE/RF outcomes remain unobserved. Raw `stage=recovery` scheduling records
likewise describe hint disposition separately from actual retry acceleration.
See the diagnosis checkpoint (historical local record).


The E290 can supply a separate CRC-framed Link/core/LoRa stream. See
[the board observer contract](BOARD-OBSERVER.md#crc-framed-board-link-observations)
for setup-only reset, bounded capture, heartbeat reconciliation and independent
coverage rules. `handshake_timeline.py` joins positive phone and board Link tokens
from existing captures without attributing a handshake to a group record or
turning a missing software observation into a missing RF transmission. The
normal firmware and default capture mode keep diagnostics/reset disabled.
