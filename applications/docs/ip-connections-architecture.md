# Mobile LAN and TCP architecture

The [connection guide](ip-connections.md) describes supported behavior and
acceptance. This document records the ownership and recovery contracts behind
that behavior. The [product scope](product-scope.md) owns application boundaries;
the [roadmap](roadmap.md) tracks future providers.

## Ownership and saved intent

The existing app-owned native host carries Bluetooth, LAN and TCP concurrently.
Shared runtime/host APIs own transport mechanics, while reusable platform
adapters own network inventory and discovery. Application Rust owns preferences,
operation admission and reconciliation. React submits intent and renders typed
projections; screens and JavaScript reloads do not own sockets or retry loops.

Connection settings belong to the installation, independently of the selected
messaging identity. The versioned database record contains a settings revision,
LAN preference and up to sixteen TCP profiles with stable IDs, labels, endpoints
and desired enabled state. Native validation rejects invalid ports, malformed
hosts and duplicate effective endpoints without requiring DNS or connectivity.
Runtime IDs, resolved addresses, counters, permissions and connection state are
observations and are not durable preferences.

Missing connection settings initialize LAN disabled and the public TCP preset.
An existing empty list, disabled profile or edited endpoint remains authoritative.
Unknown or malformed versions are preserved for repair instead of resetting the
installation. Connection policy stays out of `DevelopmentNodeStartInput`, whose
serialized bytes also serve Android service recovery.

Saves check the expected settings revision and complete after the database
transaction commits. Accepted mutations survive caller cancellation. After an
uncertain result, reread saved state instead of replaying stale intent. Saving
while the node is stopped does not start it; failed persistence must not replace
the live configuration.

## Reconciliation and retirement

One reconciler is bound to the installation owner, native generation and settings
revision. Startup reads saved intent, prepares platform resources and attaches
supported enabled profiles. An absent peer or denied optional transport does not
block unrelated connections or node startup.

An edit persists desired state first. Replacement waits for acknowledged
retirement of the old attachment: driver shutdown, supervisor children, socket
owners, the manifold barrier and discovery-provider shutdown. A queued shutdown
request or elapsed delay is not a retirement receipt. Late work from a retired
owner or generation cannot change its replacement.

Saved and applied revisions remain separate while reconciliation is pending.
Closing attachments stay visible until retirement completes, even after their
profile is removed. Restart rebuilds mappings from the latest saved state.
Shutdown closes admission, drains accepted work and joins resource retirement.

Platform network changes update inventory and wake existing reconciliation.
Hostnames are resolved again during TCP retry, with bounded alternate IPv4/IPv6
connection attempts. Core backoff, availability and announcement scheduling are
reused; the app does not create another delivery queue or timer system.

## Platform boundaries

Android owns NSD sessions, ConnectivityManager/LinkProperties inventory and the
multicast lock within the native lifecycle. Inventory changes carry real
interface indices and addresses; empty inventories retire stale peers. Resources
are released on disable/stop. The packaged target SDK determines the required
local-network permission behavior, which must be tested separately from Bluetooth.

iOS declares `_reticulum._tcp` and `_reticulum._udp` and uses the managed Bonjour
provider. Production configuration requests the approved multicast entitlement
and sets `PRNSMulticastEnabled`; native startup passes that explicit policy to
`configure_apple_multicast` before creating a host. The first selection or host
creation fixes the policy for the process, including later LAN re-enablement.
The enabled factory uses `with_host_discovery` to retain Bonjour alongside stock
multicast. Development builds and unconfigured SDK consumers use
`with_platform_discovery`, which keeps raw multicast disabled.

Path observation wakes reconciliation; generation fencing rejects stale
callbacks. Either discovery path can report readiness independently. Saved TCP
and Bonjour do not depend on the multicast entitlement. Configuration is not an
entitlement query: production distribution still requires matching signed
entitlements, provisioning and physical-device qualification as described in
the [iOS release guide](ios.md#release-configuration). A failed peer connection
alone is not evidence of permission denial.

An Android Bluetooth listener replacement may restart the native generation;
LAN/TCP then recover from saved intent. iOS Bluetooth background modes do not
grant continuous TCP/LAN execution. Foreground recovery and natural background
behavior require distinct acceptance results.

## Inspection and presentation

Saved profiles, logical interfaces and physical connections are separate
projections. Host inspection supplies actual parent/child relationships and
endpoint metadata; the app associates them with profiles. Logical traffic totals
are counted once, and local shared-instance rendezvous is excluded from nearby
peer counts. Network routes and announcements remain distinct from message
delivery receipts.

Interfaces → LAN & TCP exposes saved and live states, discovery details and
permission recovery. Setup can continue with LAN, TCP or offline identity setup
independently of Bluetooth. Permission failures are reported only when observed.
Native TCP/AutoLan settings do not imply that a future browser provider can use
raw sockets; browser WebSocket/gateway support needs explicit capabilities.

## Source and regression entry points

- [Saved settings and lifecycle](../prns/native-composition/src/lifecycle.rs)
  own admission, startup and recovery.
- [Native host](../../prns-host/impls/native/src/lib.rs) and
  [host commands](../../prns-host/core/src/command.rs) own shared attachment APIs.
- [LAN construction](../../prns-interfaces/impls/tokio/src/from_plan/wifi_auto.rs)
  and [discovery capstones](../../prns-interfaces/impls/tokio/tests/wifi_auto_discovery.rs)
  cover inventory, discovery, peer retirement and network return.
- [Validation](validation.md) covers native contracts, app/platform checks and
  physical acceptance requirements.

Regression coverage must preserve stale-revision rejection, caller cancellation,
failed writes, stop/restart races, provider replacement, socket retirement,
unsupported records and truthful saved-versus-live UI states.
