# Group development tools

These tools support fixed-membership regression trials. They are not the normal
app onboarding or production backup flow; use [authenticated enrollment](enrollment.md)
for new groups and invitations. Fixture snapshots contain private MLS state.

## Fixed-membership fixtures

Rebuild the app's native client with the updated generated bindings before trying
the screen; a JavaScript reload cannot add native group APIs to an older binary.
Use the [application build guide](../../../applications/README.md).

No public test identity or globally shared private group key is baked in. Obtain
the **primary Reticulum identity hash** from each intended device's node details
(not its LXMF destination). Generate one private fixture per endpoint:

```sh
cargo run --locked --manifest-path prns-groups/Cargo.toml -p prns-groups-runner-native --bin dev-group-fixture -- NEW_OUTPUT_DIRECTORY "Development group" FIRST_IDENTITY_HEX SECOND_IDENTITY_HEX
```

Identity hashes are 32 hexadecimal characters. The directory must not already
exist. The generator uses OS entropy, independently enrolls 2–8 MLS members and
derives the `prns.groups` destinations from their supplied Reticulum identities.
It emits `member-1.json`, `member-2.json`, etc. in the same order as the arguments,
with restricted Unix file permissions. Each fixture contains only that endpoint's
private MLS state plus the authenticated group and peer configuration.

On each development device, place its corresponding file at
`prns/development/group-fixture.json` under the app's prepared private data root,
then restart the native application owner. This is deliberate developer
provisioning, not an invitation or file-import UI. The native owner reads at most
4 MiB, validates the primary identity and imports into its existing
`application.redb` database. Import is atomic and refuses any existing namespace
table. Later starts verify the fixture configuration and open current state;
they never restore the original snapshot over current ratchets. A corrupt or
mismatched artifact fails closed. Keep the file available for subsequent starts.

Fixtures contain plaintext secrets. Transfer them privately, install each on
exactly one device and do not commit or publish them. Cloning a fixture or
restoring it after deleting a used database can fork/reuse MLS ratchet state.
Generate a fresh group for a deliberate reset. Fixture snapshots are enrollment
artifacts, not recovery backups. Keeping enrollment snapshots also keeps their
initial ratchet secrets: this development path does not establish at-rest forward
secrecy. It does not yet qualify private production conversations or protected key
retirement on phones.

The first development policy is fixed membership, text up to 2048 UTF-8 bytes,
256 retained records and a maximum 14-day delivery window. Bounds are part of the
fixture format; changing them requires an explicit migration or fresh enrollment.
They are logical capacity limits, not memory/file-size guarantees.

Record, output, receipt and backend object limits are independent ceilings.
Admission and maintenance must respect each bound; a record-count ceiling is not
a guarantee that every possible receipt pattern fits. Retire only eligible work
through the runner/store APIs, preserving replay floors and exact pending output.

## Headless development peer

The optional `dev-group-peer` executable is a native consumer of the same runner,
store lane and shared host used by the app. Its `headless` feature adds TCP and
executable host ownership; it does not change the reusable
runner or embedded dependencies. Build from the repository root:

```sh
CARGO_INCREMENTAL=0 cargo build --locked --manifest-path prns-groups/Cargo.toml -p prns-groups-runner-native --features headless --bin dev-group-peer
prns-groups/target/debug/dev-group-peer init /private/tmp/group-peer
```

`init` prints the public `identity` and `destination` as JSON. It generates and
persists a private identity using OS entropy only on the first invocation; later
invocations return the same public identity. Include this identity hash along with
the phones' primary identity hashes when running `dev-group-fixture` above. Copy
**only this peer's** fixture to `/private/tmp/group-peer/group-fixture.json`, then
start it:

```sh
prns-groups/target/debug/dev-group-peer run /private/tmp/group-peer --listen 0.0.0.0:4242
```

Connect the phones' TCP interfaces to this computer's reachable LAN address and
port 4242. The listener enables Reticulum Transport, allowing connected members
to establish links through it as well as exchange stored group envelopes with it.
Use a private test network and allow the port through the host firewall. A second
headless endpoint can use `run ITS_ROOT --connect HOST:4242`. This is ordinary
Reticulum TCP traffic; the command interface remains on local stdin. No remote
command or fixture-download server is exposed.

The process emits JSON lines to stdout, diagnostics to stderr. A `ready` event
reports its public identity, destination and actual listening address (including
an OS-assigned port when listening on port 0). Enter one command per line:

```json
{"op":"status"}
{"op":"request_id"}
{"op":"send","request_id":"<copy the returned request_id>","text":"Hello from the headless peer"}
{"op":"history","before":null,"limit":50}
{"op":"stop"}
```

Use `request_id` to generate a fresh timestamped token from OS entropy for each
new send. Copy the returned hexadecimal `request_id` into the send command;
arbitrary random 32-byte strings are no longer valid new tokens. Keep that ID and the
exact text until an `accepted` response is reconciled. Retrying the same pair,
including after process restart, returns the original `record_id`; changing the
text under the same ID returns `idempotency_conflict`. Results have a 14-day retry
window; `retry_expired` means the old token must not be resubmitted as a fresh
send under a replacement ID without checking history. `storage_full` preserves
the original retry identity and requires capacity to become available.
Acceptance means durable
local storage, not delivery. History reports `received_count` and
`recipient_count`, never read status. IDs are hexadecimal; revisions, sequences
and timestamps are decimal strings to preserve their exact values. Pass the
returned `next_before_sequence` as the integer `before` value for an older page.

Commands are capped at 16 KiB and history at 100 rows. Invalid commands return an
`error` event without shutting down the peer. `status` reports whether the sync
worker is running or unavailable; running does not confirm connectivity or
recipient delivery. Durable reads and send acceptance remain available after a
sync failure, which requires a process restart to resume synchronization. EOF and
`stop` both stop and join synchronization, stop the host, then fence and drain
accepted database work. Force termination is recovered from the durable database
on the next run; an ambiguous send still requires retry with the same request ID.

Keep `identity`, `group-fixture.json`, `application.redb` and `network/` together in
the private root. On Unix, the root and `network/` directories must have no
group/world permissions (normally 0700), and identity/fixture files must be private
regular files (normally 0600). Symlink roots, directories and private files are
refused; existing permissions are never silently changed. `init` creates new
roots with 0700 permissions, and `run` requires an existing identity and fixture
and never mints a replacement. The development fixture/key limitations described above apply.
The default poll interval is 15 seconds, with bounded retry backoff; development
measurements can set `--poll-ms 100..60000`. Fast polling is for controlled tests,
not a radio or battery policy.

The `headless_process` suite runs three independent executables over real TCP:
initial delivery, receipt convergence, offline catch-up through a remaining
member after the author exits, force termination and exact-ID retry after reopen.
It also checks stable initialization, wrong-identity fixture rejection, public or
symlinked private path rejection, and bounded invalid command recovery:

```sh
CARGO_INCREMENTAL=0 cargo test --locked --manifest-path prns-groups/Cargo.toml -p prns-groups-runner-native --features headless --test headless_process
```

These process tests require permission to bind loopback TCP sockets. They do not
establish physical phone, LAN firewall or radio interoperability results.
