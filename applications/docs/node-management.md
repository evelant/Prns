# Node management

The **Nodes** tab controls this phone and authorized remote nodes. A phone does
not need a board for nearby messaging. A paired board is not automatically a
group member or another device sharing the phone's identity.

## Pair and identify a node

Choose **Pair a node** and follow the node's supported pairing procedure.
Discovery names are optional hints. The temporary **Pairing session** reference
distinguishes app entries but cannot identify a physical board. Already-paired
status becomes available after the authenticated exchange supplies the stable
node identity; signal strength is not used to guess it.

Confirmation shows the authenticated node ID, existing-pairing state and an access
summary. **Show pairing details** exposes the exact grant. New pairings currently
use one full-control Administrator preset with the node's supported request set;
there is no granular permissions picker.

After success, **Pair another node** returns to discovery without deleting saved
grants. Failed or expired attempts offer **Search for a node**. These controls do
not silently reset an active native attempt. Headless nodes must provide their
supported confirmation/approval method; complete headless provisioning is still
a separate gap.

## Read and change settings

Open **Nodes → Manage node**. Only supported and authorized controls appear.

| Area | Purpose |
| --- | --- |
| Interfaces | Select an interface; inspect peers and connection/radio settings |
| Device | Supported Wi-Fi, display, positioning, sleep and power actions |
| Information | Firmware, battery and power observations |
| Access | Devices authorized to control the node |

Settings load automatically; longer lists use bounded pages. Refresh reads the
node again. **Show node details and diagnostics** exposes identifiers and
connection checks. A snapshot is an observation, not a permanent online state.

Controls include interface power/mode/groups, LoRa settings, Bluetooth discovery,
positioning, display behavior, system sleep/wake and station/hotspot functions
where supported. A command without a current-state getter is presented as an
action, not a switch with an invented value. Positioning settings control node
hardware; they do not add a location feed to the app.

Rust checks authority and capabilities again when admitting each operation.
Results distinguish applied, unchanged, scheduled, failed and uncertain. Leaving
the screen does not undo an accepted change. A lost response must be checked
before another write; there is no automatic replay. Disruptive changes require
confirmation. A wake command cannot reach a radio over a connection that is
already offline.

## Try a Wi-Fi network

Use **Try network**, then **Keep this network** or **Restore previous network**.
The node owns the confirmation deadline and restores an unconfirmed trial.
Only the node can confirm Wi-Fi readiness; Bluetooth reachability is not proof.

If a response is lost, check status rather than resubmitting credentials.
Passwords are not stored in operation records or retry queues. Reopening reads
the node's current state, not a durable phone-side history of the trial.

## Share an address and review access

**Share node address** is available after a successful check confirms permission.
It asks the node to announce its configured address, which can differ from its
control endpoint. The latest operation result survives screen changes and local
Stop/Start in the same process, but not process termination. An unknown outcome
is not automatically retried; the announce may already have happened.

Access inventory reports device IDs. This phone cannot remove its own access,
and protected administrators cannot be removed remotely. Adding a new controller
requires verified recipient onboarding that is not yet an implemented app flow.
See the [remote-control plan](remote-control-expansion.md).

## Connection and evidence limits

Commands check the authorized target and whether its selected interface can
transmit. A ready route skips discovery; otherwise the app can request a path
for that exact target and recheck readiness after the response. Describe uses
one 20-second budget spanning queueing, discovery, connection and response.
Address sharing has a separate five-second readiness wait.

Navigation, target changes, cancellation or Stop end caller-owned reads. Returned
control Links are closed when their scope ends. Dropping a JavaScript Promise
alone is not cancellation; use the supported cancellation contract described in
[the bindings guide](../prns/native-composition/bindings/README.md#cancellation-and-ownership).

The app represents the integrated protocol's RemoteControl request kinds, but
implemented controls are not all physically qualified. The
[validation index](validation.md) links bounded board/phone trials. Successful
Wi-Fi Keep, broader iOS settings coverage, differing capabilities and complete
headless onboarding remain follow-ups. Preserve original settings and record
uncertain results when testing disruptive changes.
