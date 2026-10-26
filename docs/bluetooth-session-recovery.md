# Bluetooth session recovery

This SDK boundary implements native Bluetooth ownership and bounded Android
platform I/O. Retained post-handshake controls, correlated control completion and
negotiated peer liveness belong to the following Bluetooth integration. A
foreground restart trial does not qualify long idle or OS restoration.

## Current ownership

- The allocation-free connection policy preserves a healthy incumbent. A BLE
  greeting claims a transport identity; it does not authenticate an LXMF sender,
  and a competing Hello alone cannot authorize eviction.
- Tokio close notifications carry the admitted status instance. A late close
  cannot remove a replacement member or clean up its backend session.
- Android Rust links retain one physical-connection lease through the handshake
  and both data halves. The final owner retires only that connection and checks
  queue identity against connection-ID reuse. Address-wide cleanup cannot close
  unrelated owners.
- Android platform operations retain callback ownership and deadlines. Rust
  queue admission is not a completed GATT write. These bounds cannot establish
  whether an idle remote application is still responsive.

The SDK owns generic Android platform mechanics; Hopspot consumes the same
transport helpers. Applications own contacts, mail and product policy through
public host APIs. Authenticated LXMF key retention uses the existing core API and
does not substitute for Bluetooth recovery.

## Following integration

Keep a native control owner through settlement, with a shared source/sink/control
split used consistently by all backends. Columba has no native control channel.
Do not advertise a liveness capability until the runtime drives the protocol and
the backend supports its bounded completion contract. Missing or unsupported
capabilities must preserve legacy behavior.

Queued control writes need exact-session operation receipts, a retained result
across caller cancellation, and one deadline covering admission, Busy retries
and completion. Native completion remains distinct from remote receipt.
Callbacks without operation identities require serialization and quarantine after
uncertain completion; they must not release an ambiguous lane for new work.

Negotiated probes must be bounded, driven by shared allocation-free policy and
scoped to the actual connection. Wrong or late replies, ordinary data traffic and
competing greetings must not extend an outstanding probe deadline. Only
established failure evidence may retire an incumbent. Legacy peers cannot be
declared dead from a competing greeting alone.

Cooperative shutdown also needs a supported bounded close before dropping session
owners. Radio or permission loss still requires immediate local teardown; do not
invent a new shutdown reason for legacy peers.

## Qualification

Host regressions cover ownership, stale callbacks, callback deadlines and recovery
contracts. The next controls/liveness slice must additionally cover both legacy
directions, stalled capability reads, silent incumbents, cancellation, slot reuse
and backend builds. These checks do not establish RF or mobile lifecycle results.

Retain exact-source and exact-binary receipts for physical restart/send trials in
both native roles. Verify unchanged identities and queued messages, duplicate
counts and actual reconnection behavior. Separate system-radio changes, permission
loss, silent-peer expiry, long idle and natural suspension. A periodic app reset
or manual announcement is not evidence that transport recovery works.
