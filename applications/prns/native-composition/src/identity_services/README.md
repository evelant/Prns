# Native identity service composition

`MessagingDriver` connects the portable identity controller to live Reticulum,
direct LXMF and group services. It is a native Rust integration surface; the
phone's generated `NamedIdentityClient` captures an immutable owner reference
and revalidates admission on every call. The app actor owns coordinator buffers,
runtime leases and accepted work across canceled foreign calls.
The portable identity core, codec and runtime retain caller-owned storage and
tables, `no_std` and allocation-free production code.

## Embedding order

1. Keep one `NativeStore`, durable coordinator, `RuntimeState` and driver for the
   shared host lifetime. Install `MessagingEvents` in the host's synchronous event
   and authenticated-announce callbacks before creating service registrations.
2. Publish/restore each catalog key through the vault. Open `MessagingIdentity`
   with its immutable `StorageScope` and verified key reference. Choose explicit
   create or reopen; missing/corrupt existing stores never become new identities.
3. Supply `MessagingDriver::plan()` to the controller. Poll each controller step;
   on `Pending`, call `driver.advance(&store.runtime_authority(&coordinator))`,
   then retry the same step. Retain the driver and runtime through cancellation.
4. Acknowledge activation in the catalog and confirm it in the runtime. Only then
   call `start_network` with a current `NetworkPermit`. Registrations are staged
   with ingress and announcements closed until this call enables them.
5. Serialize lifecycle changes, admission and dispatch under the embedding's
   exclusive owner gate. `groups()` and `direct()` return trusted Rust clients,
   **not authorization grants**. Validate core admission and retain a runtime
   mutation lease around fresh accepted service work. IDs and scopes are not
   credentials for an untrusted caller.
6. Persist stop intent, fence admission, settle mutation leases, then drive scoped
   quiescence. Acknowledge and confirm its final dependency report before pruning
   the operation or beginning another transition.

`advance` retains its actual asynchronous future when its waiter is cancelled.
After a catalog session reload, use `recover_pending` with the newer action to
settle and drain that old work before runtime reconciliation. Reuse the existing
runtime tables and leases. Replacing a controller or dropping a task handle does
not establish shutdown. A genuine process restart opens the original catalog,
key references and scoped stores, then drives the fresh recovery actions.

## Scope and shutdown

Each identity owns three SINGLE destinations: LXMF delivery, group sync and group
enrollment. Group Link authentication uses that explicit identity, independently
of the shared host's default key. The reusable `NativeServiceScope` bounds new
host operations and outgoing Links; canceled waiters cannot discard operation
ownership. Existing unscoped constructors remain available for other consumers.

Pause fences new application mutations and outgoing work, joins the group worker,
drains admitted host operations, retires owned destinations and Links, cancels
their pending announcements, and waits for retained interface buffers to be
released. Direct callbacks stay alive until the host acknowledges ingress
retirement; their accepted queue then drains into storage. A final storage fence,
lane drain and dependency observation precede store closure. Old live service
handles fail after their generation is fenced. An app `NamedIdentityClient`
retains its immutable catalog owner and acquires the new service generation after
resume; it never follows a different selected identity.

The egress barrier measures host-buffer release/handoff, **not RF transmission,
peer receipt or recall of emitted traffic**. Native crypto jobs may retain secret
clones until completion, but generation checks reject their stale results.
Shutdown is not secure erasure. LXMF transport proofs retain their existing
meaning; draining accepted callbacks does not turn a proof into a durable remote
application receipt or eliminate bounded ingress-queue overflow.

## Durable data and retirement

The optional native `redb-store` adapter binds each database to catalog
incarnation, local sequence and public hash. Contacts, signed card revisions,
mailbox history/outbox, group enrollment/MLS state, exact retries and UI recovery
intents stay in that database. Installation Bluetooth and management settings
remain outside it. There is no migration or fallback to the legacy primary key.

Dependency observations include queued mail, groups, invitations, enrollment work
and unresolved accepted action intents. A durable digest/revision record changes
only when these facts change. Pending and expired unresolved intents block
retirement even before a group exists. Until generic group departure/retention
hooks are implemented, any retained group or invitation conservatively blocks
retirement. Pause preserves them and remains supported.

## Deliberate limits

- This profile uses `NoRatchets` for Reticulum SINGLE delivery; durable MLS state
  remains in the group store. The endpoint API can snapshot/restore receive
  ratchets, but active rotation persistence is not qualified for this composition.
- Outgoing operations/Links have per-owner bounds. Incoming Links/requests also
  consume shared engine capacity; per-identity inbound quotas and adversarial
  resource isolation remain open. Reserve management headroom separately. Actual
  registration capacity failures require scoped cleanup before retry. If Link
  closure history overflows while establishment results are still pending, the
  scope reports `RecoveryRequired` and closes fresh admission; retire it and
  create a new activation rather than recycling uncertain Link slots.
- Driver/event slots have a caller-selected lifetime bound of 1–64 identities.
  Removing/recycling these slots is not implemented. This is not an unlimited
  saved-identity implementation.
- Individual group runners stay manual-only; the app's shared announce owner
  schedules unsolicited LXMF/group announcements across identities. It reserves
  attempts durably before dispatch and retains cooldowns across restart. Requested
  Reticulum path responses keep their existing separate protocol pacing.
- Pause closes the live store owner. Guarded local access may reopen its existing
  database and a paused direct worker for history, contact edits and offline card
  signing. It does not register destinations or activate queued sends. Resume
  fences that local generation before preparing fresh live service handles.
- Protected phone storage, group departure/retirement UI, per-identity inbound
  quotas and embedded storage/runtime adapters remain separate work. Forwarding
  boards need no copy of a phone's identity or MLS keys.

See the [identity library checks](../../../../../prns-identities/README.md#checks)
for host and portable-target coverage, and [app validation](../../../../docs/validation.md)
for packaged-build and device acceptance. The [named-identity design](../../../../docs/named-identities.md)
defines app scheduling and lifecycle policy.
