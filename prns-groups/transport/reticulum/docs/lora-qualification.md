# Isolating a phone–LoRa–phone trial

This is an acceptance procedure for the following phone integration. This
carrier boundary provides the Reticulum transport, not the group app or its
phone controls. Use consumers implementing the controls and diagnostics below;
a carrier test alone does not qualify this physical journey.

The desired path is phone A → Bluetooth → board A → LoRa → board B → Bluetooth → phone B. A successful receipt alone does not prove this path: nearby phones and boards can form shorter Bluetooth routes, and ESP-NOW, Wi-Fi, TCP or USB may also carry traffic.

## Existing isolation mechanism

Bluetooth discovery groups already provide a reversible partition, without changing Reticulum routing. Configure **only** `trial-a` on phone A and board A, and **only** `trial-b` on phone B and board B. Do not retain the shared default `reticulum` group. Both ends of the native Bluetooth handshake reject peers with no group in common before creating a Reticulum data interface. Group replacement tears down existing peer lanes and in-flight handshakes before acknowledging completion. Discovery groups are transport selection hints, not secret credentials or group-message authorization.

The board exposes this through the paired RemoteControl connection. Verify the
loaded firmware supports durable discovery-group replacement before isolating
the trial:

```rust
let groups = DiscoveryGroupSet::singleton(DiscoveryGroupId::parse("trial-a")?)?;
connection.replace_interface_discovery_groups(
    bluetooth_supervisor_id,
    RemoteControlDiscoveryGroups::new(groups),
).await?;
```

The phone consumer's remote interface card exposes the same operation as **Discovery groups (one per line)**. Read the actual Bluetooth supervisor ID from interface inventory. The board management layer persists this change and attempts to restore both prior durable and runtime state after a failed apply. Verify runtime readback after replacement and restart; successful host tests do not establish the applied settings on a particular board.

**Required phone controls:** Connections → Advanced Bluetooth discovery → Change discovery groups
now exposes a persisted set of one to four names and an explicit Restore default
action. The default remains `reticulum`. Saving confirms the connection change;
the UI reports durable saved names separately from currently applied names.
Stopped or disabled Bluetooth saves for its next start. Invalid stored settings
fail closed and can be repaired with an explicitly confirmed default restore.
This uses the existing Bluetooth discovery protocol, with no Reticulum wire or
routing-preference change. Physical qualification requires a separate trial receipt.

The native settings lane retains admitted writes and live replacement across
caller cancellation, fences storage owner/node generation, and serializes
replacement against Bluetooth enable/disable. Tokio Bluetooth reloads the chosen
set before its first handshake and after re-enable, before acknowledging the
new configuration. Discovery groups are not a route-selection proof: verify
actual peers, alternate-interface state, LoRa counters and the negative control.

Host regressions check radio preparation before discovery-group replacement,
startup preservation of restored groups, failure cleanup and both handshake
roles. They use controlled or mock backends and do not establish phone/board
readback, RF delivery or physical isolation. Those observations need a separately
retained trial receipt for the exact app and firmware builds used.

Before isolation, boot each board with its existing settings and verify normal
Bluetooth, network, display and watchdog operation. Preserve any startup failure
and its logs; do not erase settings to make a board appear healthy. A clean boot
capture or static call-frame analysis does not establish worst-case runtime stack
headroom. Record the tested binary and configuration with the physical trial.

## Observation limits

The downstream phone composition exposes selected routes, aggregate active Links and interface
byte counters. Opt-in native diagnostics (`PRNS_GROUP_DIAGNOSTICS=1`) now add
inbound/outbound synchronization start, phase and completion events, elapsed
time, and opaque group/peer tags. Tags use a random process-local salt and cannot
be compared across restarts or phones. A `local_send_result` event associates a
group and record tag when the caller observes a durable send result; it is not
an exhaustive commit journal or proof that an exchange carried that record.
A cancelled caller may omit it, and an idempotent retry may repeat it.
No identifiers, text, keys or salt are logged. **Refresh group** reads local projections;
**Announce groups now** advertises service destinations without forcing a sync.

Attribute delivery to LoRa only with verified isolation, correlated board TX/RX
observations and the radio-off negative control.
For the radio-off control, distinguish a directly observed attempt from the
weaker observation that a queued message remained absent with no receipt during
the interval, then converged after restoration without resending. Record that
timing limitation when attempt evidence is unavailable. Push traces additionally
carry the same opaque group, record and peer tags, start/finish, elapsed time and
count of authenticated receipts applied. Durable record-receive and receipt
transitions use matching tags within that process. A push-start event means a
delivery attempt began; path resolution or Link establishment can still fail
before Submit, so it alone does not prove RF transmission. A completed push with
an accepted MLS receipt establishes that the intended device accepted the record
through that exchange.

Limits are 128 general attempts per process, eight phase events plus completion
per attempt, 128 local send observations, 256 push attempts and 256 receipt
transitions. Legacy event categories retain their 64-event limit. The app logger
admits 2,048 general events and separately reserves 1,024 lines for local-send,
push and receipt events under the exact `prns.groups.delivery` target. Setup
traffic cannot consume that reserve. Exhaustion emits a marker, so silence after
a cap is not evidence of inactivity. Enable diagnostics only for bounded trials.

For Apple Bluetooth readiness failures, the separate
`PRNS_BLUETOOTH_DIAGNOSTICS=1` launch flag enables fixed categories and counts
under `[prns.bluetooth]`. It covers manager, scan, discovery, dial, GATT and
handshake admission; it contains no peer identifiers or group names. Six FFI
families and one supervisor family each allow 64 events, followed by a cap
marker. A separate rejection family allows 32 events plus its cap marker;
it reports static native input/reply/outcome labels and the actual Columba
policy rejection, without identifiers or recomputing handshake decisions.
The app's separate Bluetooth sink allows 512. Neither a validated
handshake nor transport readiness proves message delivery. Incompatible is a
coarse handshake result, not unique proof of a discovery-group mismatch.

Record the process boundary and preserve caps. Discovery `local_owner=false`
means no central session-map entry; a restored registry owner can still exist.
Do not use it alone to justify canceling a connection. No discovery callbacks
cannot distinguish absent advertisements from an OS-retained connection.
This diagnostic build adds no system-connected inventory query or recovery
policy change, and synchronous logging can affect measured timing.

## Reversible procedure

Both phones need management grants for their respective boards before partitioning
discovery. On the E290, select the top/status card, hold the button for more than
500 ms, select **Pair remote**, then hold. In the app use Nodes → Pair a node,
select that board and enter its eight-character invitation (valid for 120
seconds). Compare the six-digit codes. The board initially selects Reject;
select Approve and hold, then choose **Codes match — approve** on the phone.
The confirmation attempt lasts 60 seconds. Wait for **Paired** and the phone's
durable paired entry: the intermediate board status **Approved** is insufficient.
Confirm the pairing controls in the loaded firmware and record successful
authenticated pairing in the trial evidence. A source description is not proof
that either phone currently manages the intended board.

**Pair another node** reopens discovery after success,
and terminal failures can restart the search. **Already paired** compares the
authenticated target identity during confirmation. Temporary session labels
only distinguish app rows: they cannot identify a physical board or match a
saved grant before authentication. Headless boards must not require a new
display for these app improvements, but a complete headless invitation and
security-confirmation channel is still a provisioning gap. Do not bypass code
comparison or node approval.

1. Capture each board's complete interface inventory, discovery groups, radio profile and power states, and each phone's discovery sets and enabled interfaces. Bind this baseline to the actual authenticated devices and loaded app/firmware hashes. Preserve phone identities and group stores. Keep a board-local UI or independent administration path available during setup.
2. Transition one pair at a time: first give the phone its existing set plus its new side-specific group, then replace the board's Bluetooth set with only the new group. Reconnect and read back the board's singleton before removing the old set from the phone. For the default configuration, side A moves through phone `{reticulum, trial-a}` → board `{trial-a}` → phone `{trial-a}`; repeat with `trial-b` on side B. The final trial must have **no common set between sides**. A reply can be lost when the management lane is intentionally removed; reconnect and inventory the applied state instead of assuming failure.
3. Disable the boards' unused ESP-NOW, Wi-Fi, TCP and USB Reticulum interfaces with the existing `set_interface_power(id, RemoteControlInterfacePower::Off)` operation. In the app, choose **Turn interface off**, then confirm **Turn off** on the intended interface card. Keep Bluetooth and LoRa enabled. Read back settled inventory after scheduled changes. These board power choices are volatile: reapply and verify isolation after any board reboot. Phone builds must have no development TCP target or other network interface; nearby third-party bridges must be excluded. LoRa frequency, bandwidth, spreading factor, coding rate and framing must match.
4. After the settled replacements, restart only node runtimes if necessary to remove old Links/routes; retain application data. Confirm each phone sees only its own board as a Bluetooth peer and the boards have no cross-side Bluetooth lane. Record fresh route/interface inventory and baseline counters.
5. Send a uniquely labeled group message and wait for the intended device receipt. Record both boards' LoRa TX/RX byte increments and the phones' selected routes. On each remote LoRa card, **Show connection details** exposes **Data sent / received** (`txBytes / rxBytes`); refresh interface inventory for each counter sample. This RemoteControl inventory does not expose frame counts. Capture the full interval, since enrollment, announcements and retries also consume airtime. Repeat in the other direction.
6. After healthy delivery and receipts in both directions, disable LoRa on one board and read back its state. Submit a fresh message once. It must remain undelivered and unacknowledged despite both phone–board Bluetooth connections staying up. Record route, Link and counter activity, but only an attributable request event establishes that this record was attempted. Choose and record an observation window from the configured retry/backoff and Link deadlines. If attempt evidence is unavailable, label the result as a bounded behavioral off/on observation rather than attempted-transfer proof. Re-enable LoRa and require that same message and its receipt to converge without another send. An observed delivery during the isolated interval reveals an alternate path; silence without an attempted exchange is inconclusive.
7. With LoRa restored and the partition still in place, disable Bluetooth on the recipient phone, submit another unique message once, and verify it remains pending. Re-enable the recipient's Bluetooth and require catch-up and its receipt without resending. Cold-restart a phone while preserving its store; verify the messages and receipts survive and no duplicate visible messages appear.
8. Keep Bluetooth management available while restoring **each actual board's captured original radio profile and interface power states**. Radio profiles are persisted, so rebooting is not restoration; re-inventory the interface ID after a profile change. Restore each board's original discovery set using temporary phone overlap, then restore each phone's original set. Compare fresh readback with the saved baseline, including authenticated identities and retained messages. Record any mismatch instead of substituting a remembered default. Do not reset group stores or regenerate identities.

Newly committed authored records now trigger bounded push on an identified
Link; a supporting receiver returns its durable MLS receipt on that same Link.
Inventory polling repairs missing records and receipts, including legacy peers
and envelopes too large for Submit. Pull can return a receipt on its existing
Link when budgets permit. There is no persistent Link pool across exchanges.
Multi-hop recovery uses a 15-second active polling interval, with idle and
failure delays growing to five minutes. A new local record can bypass healthy
idle polling delay, but does not erase failure backoff.
Several groups and an offline headless peer can also contend for the two outbound
slots, with work to one identity serialized. Individual network exchanges can
wait up to ten minutes, and a pass may contain several exchanges. No fixed short
wait proves that the selected group attempted delivery. Size the observation
window from the actual configuration and preserve attempt evidence; nominal
timer sums are not guaranteed delivery bounds.
Start with the healthy bidirectional check and correlate actual path activity
with the opt-in group/record/peer push trace. A generic failure event alone cannot
identify the trial group's attempt or prove transmission of a specific record.

## Link deadlines and route recovery

PRNS uses the pinned Python RNS 1.5.4 Link allowances: first-hop timeout plus
six seconds per hop for the initiator, with the outgoing interface's MTU airtime
allowance at a relay. Timers start before transport submission. A slow or queued
later hop can exhaust the initiator's allowance; the protocol does not advertise
that remote queue to the endpoint. Keep retries and path rediscovery viable
instead of assuming every Link establishment succeeds.

PRNS formerly withheld cached path replies for routes marked unresponsive by
a failed Link. Full-mode leaves could then become stranded, because Full mode
does not recursively forward unknown-path requests. The correction matches
Python's retained-cache response policy without marking the route responsive,
renewing its evidence, loosening replay/loop guards, changing deadlines, or
adding allocations. Deterministic tests cover actual proof expiry, cached
rediscovery and stale proofs that must not validate a new attempt. Registered
Python interoperability checks provide separate protocol evidence; neither those
checks nor the host carrier establish physical LoRa behavior.

The host regression carrier uses real Reticulum nodes, Links and Resources with
a controlled slow segment. It checks scheduling and routing under that model,
not RF range, interference, regulatory airtime, Bluetooth coexistence, board
buffering or physical power-loss recovery. Those need dedicated physical tests;
the isolated delivery procedure alone does not establish them.

## Evidence to retain

Keep physical receipts outside the shipping source tree, with an attached
source/binary manifest: exact commits and any local patch, toolchain/build flags,
app build identifiers, firmware image hashes, board/phone roles and trial time.
Bind configuration and inventory captures to the actual authenticated devices.
Do not include private identity keys, group secrets or message bodies in shared
reports.

Retain baseline, isolated, radio-off, restored and final-cleanup settings; observed
send/delivery/receipt timestamps; route/peer inventory; LoRa counters; diagnostic
process boundaries and cap markers. Record the observation window and whether a
specific attempt was directly observed. Separate pass/fail/inconclusive outcomes
for bidirectional delivery, radio-off control, offline catch-up and restart
recovery. Older trial results apply only to their recorded source, binaries,
devices and settings; this procedure and host tests do not renew that evidence.

## Source pointers

- `prns-core/src/interfaces/bluetooth_auto/handshake.rs`: matching discovery sets on both handshake roles.
- `prns-interfaces/impls/tokio/src/bluetooth_auto/runtime.rs`: settled replacement and old peer teardown.
- `prns-interfaces/impls/embassy/src/bluetooth_auto/runtime/mod.rs` and `runtime/tests/discovery_groups.rs`: radio restart, restored configuration and actual-supervisor regressions.
- `personal-hopspot/embedded/esp32/src/s3/remote_control.rs`: board persistence/rollback and per-interface power control.
- `personal-hopspot/embedded/esp32/src/s3/firmware.rs`: bounded startup phases for loading sealed Wi-Fi state before the main runtime.
- `applications/prns/app/src/features/nodes/remote-management/interface-card.tsx`: existing remote discovery-group editor.
- `applications/prns/native-composition/src/bluetooth.rs`: current local phone Bluetooth control boundary.
- `prns-groups/core/src/sync.rs`, `prns-groups/transport/reticulum/src/{push,sync}.rs`, and `prns-groups/runner/native/src/hub.rs`: push/pull exchanges, capability limits and per-peer scheduling.
