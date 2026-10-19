# E290 Link, route and queue observer

`board_observer.py` captures only the opt-in firmware's closed `prns.link.trace`,
`prns.path.discovery`, and `prns.lora.queue` schemas, boot-boundary hints, and host
capture health. It sends no serial data. By default it performs no explicit reset. The serial object is
constructed without a port; DTR and RTS are disabled before assigning and opening
the port. USB serial, VID, and PID must match before opening and every five
seconds thereafter. A mismatch stops capture. This requires exclusive ownership
of that serial port; stop other monitors first.

These settings do **not** guarantee that opening or closing a serial device leaves
hardware state unchanged. pySerial calls the OS open operation before applying
DTR/RTS through separate ioctls, and a driver can change control lines. In the
2026-10-14 trial both boards emitted ROM and firmware boot hints immediately after
capture opened, consistent with a reset or buffered boot output. No isolation or
message case had begun. Keep captures open, inspect boot hints, and verify fresh
board/phone readiness before starting an isolated case. Do not reopen a monitor
during a case and assume the operation is passive.

After installing the diagnostic firmware, use the previously verified USB
inventory. Do not substitute a guessed port or serial number:

```sh
validation/.venv/rns-interop-1.5.4/bin/python \
  tools/device/group-trial/board_observer.py \
  --port /dev/cu.usbmodemVERIFIED \
  --usb-serial VERIFIED_USB_SERIAL \
  --vid 0x303a --pid 0x1001 --label board-a \
  --seconds 900 --max-events 4096 --output RUN/board-a.jsonl
```

The output must be new. Each JSONL row is flushed and fsynced. Every row includes
host UTC and monotonic observation timestamps; route events also retain board
sequence, board observation time, optional core time, and the already-redacted
destination/interface/request tokens. No raw log line or arbitrary driver error
is persisted. A hash identifies the verified USB serial in capture metadata.
pySerial 3.5 purges pending host input inside its own `open()` method; capture
metadata explicitly marks pre-open input as unobserved. The tool does not invoke
another input purge. Initial hardware-buffered events can still predate capture.
Host timestamps describe observation, not when those events happened on the board.

The parser uses closed phase, field, and enum sets. Unknown fields, malformed
diagnostics, undecodable bytes, and oversized lines are discarded and counted.
Recognized panic/banner, stack-overflow, stack-guard, exception and backtrace
prefixes become a closed `firmware_fault` category. Messages, paths, task
addresses, registers and stack dumps are discarded. At most 32 such records and
one explicit fault-budget notice are retained per capture. This recognizes the
selected local ESP source formats; an absent category does not prove no panic.
A sequence gap is reported explicitly. A decreasing sequence adds an **inferred**
restart boundary; it does not prove a reboot. Recognized ROM/firmware banners add
explicit boundary hints without copying their contents. Attaching after events
already occurred records that initial observation gap.

Capture is bounded to at most one hour, 32 MiB of input, and 16,384 persisted rows
plus one terminal row. Defaults are 15 minutes and 4,096 rows. The row cap includes
health, boot, and gap records, not only route events. Partial final lines and
capture interruption are explicit. SIGINT and SIGTERM run the interruption
cleanup; forced termination can leave the terminal row absent. Read operations have a 250-ms timeout; host
scheduling and filesystem latency can extend wall-clock completion. Return code
zero means the requested duration completed, not that the route evidence is
complete or that any message was delivered. Inspect the terminal `problems` list.

`capture_health` says only that the host reader loop ran and the port remained
open. Legacy route/queue diagnostics have no board heartbeat contract. Their logging is synchronous and
may drop or truncate output; the firmware itself caps route output per boot.
Missing lines or silence never prove the absence of a route event or RF
transmission. `Dispatched` establishes a runtime handoff, not a physical send.
Use positive correlated phase evidence and retain these limitations in timing
claims. Install the normal image after the instrumented trial.

Boot qualification also demonstrated character-level interleaving between log
writers. The route parser rejects incomplete or malformed records, concatenated
records, unknown fields, and mixed free text. A damaged route prefix can be
counted only as an unrelated line, so `rejected_route_lines=0` cannot establish
complete route coverage. The log stream has no atomic-record or checksum
guarantee; corroborate valid records across sources rather than treating the
diagnostic stream as an authenticated transcript.

Host tests use fake serial devices and never open a port:

```sh
python3 -m unittest discover -s tools/tests -p 'test_group_trial_board_observer.py'
```

## CRC-framed board Link observations

The qualification-only `link-diagnostics` image adds a separate bounded worker
with a 32-entry nonblocking handoff in PSRAM. Core callbacks do no formatting or
serial I/O. Core and LoRa producers allow 4,096 and 8,192 ordinary callbacks plus
one terminal notice each. The worker publishes one boot probe, a shared sink
sequence over **every** record, and health every two seconds (at most 1,800 health
records, then a terminal notice). The 2026-10-16 trial image omits the legacy
route/queue log features; this is an observation-only change, not a radio or
queue-policy change. Normal firmware disables these diagnostics.

Each frame has the exact shape:

```text
prns.link.trace v=1 boot=<16hex> seq=<u32> observed_ms=<u64> <closed payload> checksum=<8hex>
```

The CRC32 covers the ASCII bytes after `prns.link.trace ` and before
` checksum=`, without newline. Total frame length is bounded to 768 bytes. CRC
catches corruption/interleaving, not malicious forgery. One formatting call does
not guarantee physical serial atomicity. The parser never repairs a damaged
line, saves raw content, or accepts unknown fields and enum values. Boot tokens
come from board entropy. Public ephemeral Link IDs use the same hashed token as
phone diagnostics; interface tokens stay boot-local. No payload or stable
endpoint identity appears in these frames.

`observed_ms` is the worker publication clock; the optional source `at_ms` is a
separate observation point. Core time resumes from the persisted logical epoch;
radio time uses absolute Embassy ticks. The probe therefore includes `core_ms`,
`radio_before_ms`, and `radio_after_ms`, bracketing a core-clock sample. The
analyzers retain that anchor and convert core timestamps/deadlines to radio-clock
intervals using `[radio_before - core - 1, radio_after - core + 1]` ms offsets.
The extra millisecond covers integer rounding. The sample must use the actual
runtime clock after persistence restore, since a copied boot timebase can become
stale. If a converted core `at_ms` or `requested_at_ms` interval falls wholly after
that event's publication, the analyzer reports `clock_anchor_inconsistent` and suppresses all
conversions for that boot. Clock conversion qualification is separate from
capture coverage; source timestamps and positive events remain available. No
replacement offset is guessed. A consistent sample alone cannot verify which
runtime clock was sampled. This aligns one board's clocks; it does not
synchronize different devices or equate publication with event time.

LoRa records identify queue admission/drop, TX
start/frame completion, RX/reassembly, and software ingress handoff. Driver TX
success does not prove peer reception; RX ingress handoff does not prove app
storage. Aggregate LoRa counters are independently atomic snapshots taken at
publication; they can include later or unrelated traffic. Do not subtract them
as if they were an atomic snapshot at the source event.

For complete-prefix qualification, capture the fresh probe after the reader is
open. The explicit `--setup-reset` option resets the verified ESP USB Serial/JTAG
board **once during setup**, using the local espflash 4.5.0 after-flash control-line
sequence. It requires VID `0x303a` and PID `0x1001`, journals the start/completion,
and sends no serial data. This is an active reset, not passive observation. Do it
before the trial acceptance window, then verify fresh board and phone isolation.
Never reopen or reset a capture during an acceptance case. A late attachment
without the probe remains useful positive evidence but cannot qualify absence.

```sh
validation/.venv/rns-interop-1.5.4/bin/python \
  tools/device/group-trial/board_observer.py \
  --port /dev/cu.usbmodemVERIFIED --usb-serial VERIFIED_USB_SERIAL \
  --vid 0x303a --pid 0x1001 --label board-a \
  --link-trace --setup-reset --seconds 3600 --max-events 16384 \
  --output RUN/board-a.jsonl
```

`board_link_timeline.py` qualifies a fixed interval from existing journals. It
requires one complete prefix from probe/sequence one, continuous host and board
health, and a closing heartbeat that reconciles each producer's seen, admitted,
published, and actually captured events. Pending writer output may reconcile in
a later health sample. Drops, caps, missing source/sink records, resets, clock
regressions, malformed output, a recognized firmware fault after the probe, or a
missing closing watermark fail closed. Observed faults remain explicit even if
they stop health output; missing health by itself is not labeled a panic. A
fault before the window remains relevant; a later fault does not rewrite a
closed earlier interval. Legacy route/queue caps do not consume Link budgets.

```sh
python3 tools/device/group-trial/board_link_timeline.py RUN/board-a.jsonl \
  --start START_UTC --end END_UTC
python3 tools/device/group-trial/handshake_timeline.py \
  --phone android=RUN/android.jsonl --phone ios=RUN/ios.jsonl \
  --board board-a=RUN/board-a.jsonl --board board-b=RUN/board-b.jsonl \
  --same-host --start START_UTC --end END_UTC
```

These tools take one bounded, hashed input snapshot (up to 32 MiB/50,000 rows per
file) and preserve each stream's coverage separately. The combined timeline joins
positive public Link tokens, including when coverage is unqualified. It never
binds a Link to one group message, infers physical absence, or produces a delivery
verdict. Cross-stream timestamps include reader buffering and scheduling; they
are not RF latency. Exact durable records and authenticated receipts remain the
message acceptance evidence. The frozen trial plan and artifact qualification
are separate from these diagnostic gates.

```sh
python3 -m unittest tools.tests.test_group_trial_board_link
```

## Positive queue-drop observations

The E290's separate `queue-diagnostics` feature is disabled in normal firmware.
It emits at most 512 queue drops per boot, then one explicit cap marker. Each
record contains a sequence, board observation time, queue source (`runtime_lane`,
`driver_queue`, or `active`), a closed reason, and optional age in milliseconds.
No payload, frame hash, message identifier, or stable endpoint identifier is kept.
Reasons are `expired`, `disabled`, `pre_resume`, `missing_time`, and `clock_rollback`.

Queue and route sequences and caps are independent. The observer records queue
gaps, regressions, malformed lines and exhaustion separately. Explicit boot hints
reset both sequence trackers. A drop is positive local evidence, not proof that
all old frames were suppressed or that no RF transmission occurred. Serial
interleaving can damage even a diagnostic prefix, so zero rejected queue lines
does not establish complete coverage. End-to-end delivery still requires the
phone's exact durable record and authenticated receipt checks.

```sh
python3 -m unittest tools.tests.test_group_trial_board_observer tools.tests.test_group_trial_board_queue_observer
```

## Read-only route timeline

`route_timeline.py` reads existing board and phone observer journals. It takes one
bounded, hashed snapshot per file and writes a new derived JSON file. Live files
may have an incomplete final append; that row is ignored and the condition is
recorded. It never opens a port or accesses a phone.

```sh
python3 tools/device/group-trial/route_timeline.py \
  --input android=RUN/android.jsonl --input ios=RUN/ios.jsonl \
  --input board-a=RUN/board-a.jsonl --input board-b=RUN/board-b.jsonl \
  --same-host --start-utc START_UTC --end-utc END_UTC \
  --output RUN/routes-case1.json
```

Use `--same-host` only when every input was captured on the same host; it enables
adjacent monotonic observation gaps. These include logging, scheduling, and
buffering delays and are not radio latency measurements. The shared random
request token joins positive request observations across devices. Responses do
not carry that token in this diagnostic schema: the tool shows their context
only within one capture/boot and local destination tag, without attributing a
response to one request. It does not map route tags to group-message records.
Phone timeout records also omit command/destination correlation and remain
unassigned observations.

Originating requests emit `local_request` before fan-out; per-interface
`request_egress` traces cover relayed requests only. Do not require a phone egress
event between its local request and the receiving board's ingress. Missing
phases remain unobserved. Inspect input errors, capture markers, caps and gaps
alongside positive events; the helper never produces a delivery/pass verdict.
Caps, faults, gaps and boot boundaries remain visible from the whole capture,
labelled `before`, `within` or `after` the requested window. A cap reached before
the case may have disabled further output; a fault after the window must not be
misreported as occurring during it. Ordinary health rows stay window scoped.
