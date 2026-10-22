# LAN discovery and TCP peer implementation plan

Status: implemented for iOS and Android. The [implementation guide](ip-connections.md)
separates source checks, development-device evidence and remaining distribution
acceptance. Apple multicast approval was requested on October 21, 2026 and remains
pending. Configured TCP and declared-service Bonjour operate independently of
that approval.

Enable automatic local network discovery and saved outbound TCP peers in
prns-chat on iOS and Android. Both must use the existing app-owned Prns host,
survive native runtime replacement, and work without Metro or an external radio.
Keep the settings and presentation reusable for later desktop and browser
providers, while respecting the different transports those platforms support.

The [product scope](product-scope.md) owns architecture and experience rules;
the [roadmap](roadmap.md) owns priority; [validation](validation.md) owns results.
This plan adds connection support without changing identity, membership,
messaging protocols or node-management authority.

## Intended behavior

- Automatic LAN discovery finds compatible local peers without entering their
  addresses. Support stock Reticulum AutoInterface discovery on native platforms;
  identify PRNS-specific rendezvous and Bonjour discovery separately.
- Saved TCP peers accept a hostname or IP address and port. Users can name, add,
  edit, enable, disable and remove them. A peer can be unavailable when saved.
- Bluetooth, LAN discovery and configured TCP operate together. Failure of one
  connection does not prevent the node or other connections from running.
- Native startup restores desired settings. Opening a screen, changing the
  selected messaging identity or replacing JavaScript does not recreate sockets.
- Connection state, current routes, destination announcements and message
  receipts remain distinct. A connected interface is not delivery evidence.

Use one automatic LAN configuration initially, with stock defaults: discovery
group reticulum, link scope, discovery port 29716 and data port 42671. Offer LAN
discovery enabled during new-install setup, with the platform's local-network
explanation and permission handling. Existing installations preserve Bluetooth
settings and start with no configured TCP peers; a missing LAN setting defaults
to disabled until enabled from Connections. Do not rerun identity onboarding.
Do not silently import the development TCP fixture or add a public server.

Expose advanced LAN configuration only when the selected discovery backend
supports it. LAN and Bluetooth discovery settings are independent. Arbitrary
groups or ports must not silently disable the only active discovery mechanism.
Keep standalone inbound TCP-server configuration outside the first mobile
increment; the underlying host already supports it.

## Existing functionality reused

The table records the baseline when this plan was approved. The linked
[implementation guide](ip-connections.md) owns the
completed work and its current validation limits.

| Area | Existing implementation | Remaining work |
| --- | --- | --- |
| Native LAN discovery | AutoWifi, exposed as AutoLan through the host, implements stock IPv6 multicast discovery and UDP peers, plus additional rendezvous paths | Prepare mobile discovery/inventory providers, attach it from saved settings and qualify compatibility |
| Configured TCP | Native TCP clients support DNS, IPv4/IPv6, framing, keepalive and reconnect | Persist profiles, reconcile them onto the app host and expose useful failure states |
| Host access | AttachInterface, AttachTcpClient and DetachInterface already exist; the app exposes its retained shared host | Keep product mutations under the native settings owner and add acknowledged removal |
| Inspection | HostSnapshot reports logical interfaces, health and traffic; native inspection retains detailed inventory | Expose physical connections and parent relationships without double-counting logical totals |
| Application services | Existing direct/group services, announce scheduling and durable recovery already share one host | Feed restored interface availability through those existing mechanisms |
| Future browser networking | PRNS supports WebSockets and browser gateway discovery | Add the app browser provider later; native AutoLan and raw TCP remain unsupported there |

Source anchors: [LAN protocol](../../prns-core/src/interfaces/wifi_auto/protocol.rs),
[LAN construction](../../prns-interfaces/impls/tokio/src/from_plan/wifi_auto.rs),
[host commands](../../prns-host/core/src/command.rs),
[native host](../../prns-host/impls/native/src/lib.rs),
[app startup](../prns/native-composition/src/lifecycle.rs) and
[browser discovery](../../prns-js/src/browser/auto_wifi/index.ts).

## Ownership and saved settings

Keep transport mechanics in public Prns runtime/host APIs and reusable platform
adapters. Application Rust owns connection preferences, operation admission and
reconciliation. Generated bindings carry typed commands and projections.
React renders state and submits user intent.

Store a versioned connection-settings record in the existing installation
database through DevelopmentStoreOwner. Connections belong to the installation
and shared node, not the currently selected messaging identity. Preserve the
database format and existing identities, contacts, groups and pending work.
Use the existing [Bluetooth settings transaction pattern](../prns/native-composition/src/lifecycle/bluetooth_discovery.rs).

| Saved value | Meaning |
| --- | --- |
| Schema version and settings revision | Validate upgrades and reject stale edits |
| Stable profile ID, label and desired enabled state | Preserve user intent across runtime generations |
| Explicit connection kind | AutoLan or TcpClient initially; later WebSocketClient and browser gateway discovery remain distinct |
| Typed connection parameters | LAN settings, or TCP hostname/IP, port and supported bitrate policy |

Keep runtime interface IDs, socket addresses resolved from DNS, connection
state, counters, permission observations and errors out of durable preferences.
Map a saved profile to its current host attachment and configuration revision
inside each native generation.

Define finite profile-count and field-size limits in the settings contract.
Validate endpoints through shared Prns types, including IPv6 syntax and nonzero
ports; saving must not require successful DNS resolution or a reachable peer.
Reject duplicate effective configurations without treating two labels as two
independent sockets. Preserve unknown future profiles as unavailable. Reject
unsupported settings versions without overwriting them; surface malformed
records for repair rather than resetting the installation.

Add typed read, save, enable/disable and remove operations with settings-revision
checks. Distinguish persistence success from application to the running host.
Expose saved, applying, disabled, unavailable, connecting, connected, retrying
and failed states from durable intent plus actual runtime observations.
Cancellation of a UI wait must not cancel an admitted durable change.
Return Saved only after its database transaction commits. Process death before
confirmation leaves an uncertain outcome; read the stored revision on recovery.
Settings remain editable while the node is stopped; saving does not start it.
After an uncertain write result, refresh the saved revision rather than replaying
a stale mutation. A failed save must leave the running configuration unchanged.

Keep production settings out of DevelopmentNodeStartInput. Android persists
that input's encoded bytes for service recovery; adding profiles there would
create a second settings authority and a startup-serialization migration.

## Native reconciliation and recovery

Use one serialized connection reconciler per installation owner. Its work is
bound to the exact owner ID, native generation and settings revision.

1. Read the latest saved preferences on native startup and prepare platform
   inventory/discovery resources for that generation.
2. Start the existing host and services, attach each supported enabled profile
   once, and publish profile-to-attachment associations. Node readiness does not
   wait indefinitely for an absent peer or a denied optional transport.
3. For an edit, persist validated desired state first, then reconcile the
   captured host. Serialize conflicting changes and retain admitted work after
   caller loss. Re-read desired revisions before acting on queued work.
4. Await retirement of an old attachment before reusing its identity, port or
   discovery provider. Report uncertain or incomplete removal honestly.
5. On generation replacement, discard old runtime mappings and rebuild from
   saved intent. Fence late callbacks and completions from the retired owner.
6. On shutdown, close mutation admission, drain admitted operations, stop
   discovery/browsing and child connections, release platform resources, then
   complete joined host shutdown.

Project saved and applied revisions separately when an edit has not taken effect.
Keep a retiring connection visible until removal is acknowledged, even after its
saved profile has been removed. Recovery reconciles the latest desired state;
it does not blindly replay an old edit against a new generation.

The original DetachInterface completion only queued ordinary driver shutdown.
The implementation adds acknowledged retirement in the shared runtime and host,
including supervisor children, socket owners, the manifold barrier and
platform-provider shutdown. Connection replacement awaits that receipt rather
than manufacturing completion with a delay. See the checkpoint for the separate
operating-system listener-rebind observations.

Reuse core TCP backoff and AutoWifi discovery/rebind behavior. Platform network
callbacks update inventory and wake existing reconciliation; React must not
add independent connection timers. Coalesce network changes, bound in-flight
work, and avoid repeated permission prompts or reconnect loops while blocked.
Preserve hostname-based targets and re-resolve after connectivity changes.
Exercise multiple DNS answers: the current resolver selects one address, so
add bounded address fallback if dual-stack acceptance exposes that gap.

| Recovery event | Required behavior |
| --- | --- |
| Peer absent at startup or server restart | Retain configuration, show retry state and recover through core backoff |
| Wi-Fi loss, new SSID/address/index or Wi-Fi to cellular | Refresh eligible LAN inventory; retire stale LAN peers; reconnect TCP using current OS routing |
| Permission denial/revocation | Keep desired settings, stop blocked activity and show the relevant settings action; other transports remain usable |
| JavaScript reload or navigation | Replace subscriptions only; native connections continue |
| Native/process restart | Restore enabled profiles without a screen or JavaScript replay |
| Android Bluetooth listener replacement | Restore LAN/TCP after the existing full native-generation restart; do not claim uninterrupted TCP |
| iOS suspension and foreground return | Preserve pending work; reconcile current sockets, discovery and service availability when execution resumes |
| Explicit Stop or disabled profile | Remain stopped/disabled according to existing node policy; retries cannot override user intent |

Reuse existing service availability and announce scheduling so a recovered
connection becomes useful without manual Sync/Announce. Coalesce announcements
when several interfaces recover together, retain immutable message IDs and
pending intent, and avoid creating a second delivery queue.

## Mobile discovery integration

### iOS

Start the multicast entitlement request before building the iOS multicast lane,
in parallel with shared lifecycle, saved TCP and Bonjour work:

1. Submit the [multicast entitlement request](https://developer.apple.com/contact/request/networking-multicast)
   for the production App ID rs.reticulum.prns.chat. Include development-app
   provisioning needs separately where applicable.
2. Explain the interoperability need: existing Reticulum AutoInterface peers use
   custom multicast discovery and do not necessarily advertise Bonjour services.
   Describe bounded discovery beacons, unicast message traffic, user-visible
   controls and local-network permission handling.
3. Record the submission date, reference and approval status. Once approved,
   verify the enabled distribution channels include App Store Connect, then
   regenerate matching profiles and inspect the signed distribution artifact.
   Development approval or a working local build is not that verification.
4. Prove stock discovery on a signed physical iPhone before closing its
   compatibility gate. A simulator or unsigned build is insufficient.

Approval is a dependency only for the stock iOS multicast lane. Configured TCP
and declared-service Bonjour discovery must build, run, pass acceptance and be
releasable without that entitlement. Keep the raw multicast path separately
gated by build/provisioning capability; builds without approval omit the
entitlement and do not start raw multicast discovery. Both independent paths
still respect ordinary local-network permission.

Make stock AutoInterface interoperability an explicit acceptance target. Raw
UDP multicast requires Apple's entitlement and matching signed provisioning,
in addition to local-network access. See
[Apple local-network requirements](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)
and [managed-capability provisioning](https://developer.apple.com/help/account/reference/provisioning-with-managed-capabilities/).

Reuse the existing Apple Bonjour adapter as the independent PRNS discovery path.
Its current with_platform_discovery mode suppresses raw multicast, so it can
operate without the multicast entitlement when using declared Bonjour services.
It does not establish stock multicast-only peer discovery.
If both lanes are enabled, add the smallest reusable construction/ownership
extension needed to run them together safely and deduplicate their observations.
While approval is pending or unavailable, label Bonjour discovery's narrower
compatibility and keep stock-iOS acceptance open. Neither that open gate nor a
multicast startup failure may disable configured TCP or Bonjour discovery.

The generic AutoLan factory does not currently select the Apple adapter. Add a
reusable platform construction seam used by startup and later re-enable, rather
than a special boot-only attachment. Reuse the existing Apple service-discovery
implementation and platform composition patterns. Declare both
`_reticulum._tcp` and `_reticulum._udp` when using that provider, and update
NSLocalNetworkUsageDescription to cover normal LAN connections.

Supply eligible interface indices, address prefixes and scoped IPv6 link-local
addresses through HostLanInventory where native enumeration is insufficient.
Refresh after network changes, stop browsing/advertising on disable, and fence
callbacks by generation. Report permission failure only when observed; an
unreachable peer alone does not prove denial.

Existing Bluetooth background modes do not grant continuous TCP/LAN execution.
Use current lifecycle admission and durable recovery; qualify foreground return
and natural suspension separately. See [Apple background limits](https://developer.apple.com/forums/thread/685525).

### Android

Retain the native foreground-service owner and existing INTERNET,
ACCESS_NETWORK_STATE and connectedDevice service setup. Add generation-aware
ConnectivityManager/LinkProperties inventory updates; do not hardcode wlan0.
Reuse or extract the relevant Hopspot NSD adapter, session fencing and bounded
discovery handling into the reusable platform layer.

Handle raw multicast reception and any required multicast lock as native
lifecycle resources, released on disable/stop. Verify the behavior on older
supported phones as well as current Android; NSD's newer foreground lock
handling does not by itself establish the raw-socket path. See
[Android NSD requirements](https://developer.android.com/reference/android/net/nsd/NsdManager).

Inspect the actual packaged target SDK. For target 37 and later, declare and
request ACCESS_LOCAL_NETWORK for broad automatic LAN access. For targets 36
and earlier, follow the current implicit-INTERNET model and do not request that
new permission. Test denial/revocation independently of Bluetooth permission.
See [Android local-network permission](https://developer.android.com/privacy-and-security/local-network-permission).

Restore connection profiles within every native startup, including the existing
Bluetooth PSM/listener restart path. Service survival and foreground return are
separate from long-idle or background-delivery qualification.

## Connection presentation and setup

Use one connection view with Bluetooth, Local network and Saved TCP peers.
Local network offers an enable switch, discovery status and discovered physical
peers. Saved TCP peers offer Add peer, editable name/address/port, enable/disable,
remove, retry and actual connection state. Keep advanced settings and technical
identifiers under Details. Setup must accept LAN or TCP and allow offline
identity setup; Bluetooth permission/readiness must not be a universal gate.

Do not model saved profiles, logical interfaces and physical connections as the
same list. The current HostSnapshot folds fleet members into their logical
supervisor and cannot supply those relationships. Add a reusable detailed
connection/topology projection from existing native inspection, carrying runtime
ID, logical attachment/parent, role, origin/attachment epoch and available
endpoint metadata. The app adds the saved-profile association. Preserve the
existing logical summaries; never infer parentage from hashes or display names.

Show physical peers in connection details and logical totals once. Exclude
loopback/shared-instance rendezvous from nearby-device counts. Keep routes and
announcements in Network inspection, and message receipts in conversations.
Use backend capabilities, interface kinds and platform admission outcomes for
controls; a compiled Wi-Fi capability does not prove usable discovery.
Expose the effective LAN discovery method and its compatibility in Details.
Unavailable stock multicast support must not disable the independent Bonjour
mode or saved TCP controls.

Improve TCP status at its source: publish Reconnecting while retrying and retain
classified DNS, refusal, timeout and network/permission errors. The host already
has health and failure-detail fields. Extend core error storage only as needed
for useful detail, regenerate affected bindings, and avoid logging message
content or credentials.

## Delivery sequence and acceptance

Each increment records exact source/builds and its own checks. Start the
entitlement request immediately, alongside increment 1; do not wait for its
decision to implement or qualify saved TCP and Bonjour. Track acceptance per
connection path. A release containing independently qualified TCP and Bonjour
can proceed with stock iOS multicast marked pending. Declare the full native
compatibility target complete only after the gated multicast checks also pass.

| Increment | Work | Exit criterion |
| --- | --- | --- |
| 0 Early Apple entitlement request | Submit the interoperability rationale for the production App ID and track the decision in parallel | Request is recorded; approval and App Store Connect provisioning are required only for increment 4b |
| 1 Shared lifecycle and inspection | Acknowledged detach/replace, truthful TCP retry/errors and detailed connection projection | Socket/child shutdown is observed; stale completions cannot affect replacement attachments; summaries do not double-count |
| 2 Settings and native reconciliation | Versioned installation settings, typed operations, generation-fenced reconciliation and startup restore | Old stores open intact; admitted edits survive caller loss, committed settings survive restart; one active attachment per enabled profile |
| 3 Saved TCP user journey | Production peer controls, hostname/IP validation, node-independent failure display and setup support | Both standalone phones save, connect, message, restart and reconnect without development environment variables |
| 4a Independent LAN user journey | iOS declared-service Bonjour, Android discovery/providers, default LAN profile and discovered-peer UI | iOS Bonjour and saved TCP pass on a build without the multicast entitlement; Android stock discovery and PRNS Bonjour interoperability are qualified separately |
| 4b Stock iOS multicast compatibility | Approved entitlement, matching distribution provisioning and stock AutoInterface discovery | Signed physical iPhone interoperates with stock peers; pending approval blocks only this lane |
| 5 Combined mobile recovery | Coexistence, platform transitions, retained-data updates and diagnostics for each supported lane | Recovery passes on matching standalone builds with pending work preserved; 4b results are added when entitled |
| 6 Desktop provider later | Native aggregate/provider, window/process ownership and native networking integration | Same settings/reconciler work across windows, sleep/wake and shared-instance ownership |
| 7 Browser provider later | App browser storage/worker provider plus existing WebSocket/gateway discovery | Supported browser modes recover truthfully; raw TCP/AutoLan settings remain unavailable rather than being converted |

### Expected change locations

| Boundary | Main source areas |
| --- | --- |
| Runtime shutdown and TCP diagnostics | prns-runtime/impls/tokio interface lifecycle and driver; prns-interfaces/impls/tokio TCP and AutoWifi |
| Shared topology and generated types | prns-host/core inspection, native inspection and snapshot projection; canonical host schema/bindings and prns-react-native generated adapters |
| Saved policy and reconciliation | applications/prns/native-composition development_store, lifecycle and contract; add focused connection-settings/reconciliation modules |
| Reusable mobile networking | prns-react-native platform lifecycle and Android/iOS adapters; public native/plan construction hooks where needed |
| App bridge and platform metadata | applications/prns/platform facade/effects and native module ownership; app.config.ts and platform manifests |
| Connection and setup screens | features/connections, features/network, features/setup and existing interface detail/edit routes; update route/catalog tests together |

### Automated checks

Extend existing boundaries with focused regressions:

- Store upgrade/reopen, unsupported schema, bounded inputs, duplicate endpoints,
  failed persistence, stale revisions and installation-wide identity behavior.
- Concurrent save/toggle/remove, caller cancellation, stop during each transition,
  late old-generation callbacks and actual socket release/rebind.
- Real TCP sockets and stock AutoInterface interoperability; LAN provider
  replacement, expired peers, no eligible interfaces and address/index changes.
- Core discovery central/satellite changes, legitimate shared owners, unrelated
  rendezvous-port conflicts, owner loss and port release/reclaim.
- UI saved-versus-live states, parent/child totals, capability and permission
  blockers, offline setup, stale results and unchanged delivery semantics.
- Builds with no multicast entitlement: saved TCP and declared-service Bonjour
  remain available, raw multicast does not start, and a failed multicast lane
  cannot cancel unrelated connection work.

Run affected Rust/runtime, SDK/platform, generated-contract and app checks first,
then the workspace gates required by the touched boundaries. Use the existing
api:check, native:test, native:ios:test and verify entry points. A native change
requires rebuilt iOS/Android binaries; a web export or JavaScript update does
not establish mobile behavior.

### Physical mobile acceptance

Build identifiable standalone apps with embedded JavaScript and test in place
against populated installations. Use synthetic traffic and preserve existing
identities, contacts, groups, receipts and pending records.

1. With Bluetooth and saved TCP disabled, use an iOS build without the multicast
   entitlement to discover compatible PRNS Android/native peers through Bonjour
   and exchange direct/group messages without entered addresses. Separately
   prove configured TCP works on that build with Bonjour and Bluetooth disabled.
2. Test a Python RNS AutoInterface peer separately for compatible discovery,
   announces and direct LXMF exchange. Run Android's stock lane independently;
   gate the iPhone stock lane on entitlement approval and distribution signing.
   Group-profile compatibility is not implied.
3. With LAN discovery and Bluetooth disabled, connect each phone to configured
   PRNS and Python RNS TCP peers. Cover hostname, IPv4, IPv6, unreachable startup,
   peer restart and invalid form input.
4. Enable Bluetooth, LAN and TCP together. Verify stable counts and routes,
   supported message delivery/receipts and removal of only the chosen connection.
5. Repeat edits, enable/disable, node stop/start, process restart, Android
   BLE-triggered restart, Wi-Fi changes, cellular transitions and foreground return.
6. Deny/revoke/regrant local-network access. Test no Wi-Fi, IPv4-only networks,
   multicast filtering and AP isolation; report unavailable paths without
   claiming rendezvous bypasses every network restriction.
7. Exercise both legitimate and unrelated rendezvous-port ownership. Verify
   recovery and ensure a local relay never appears as a nearby device.
8. Record natural suspension, locked idle and Android service/device behavior
   separately from foreground acceptance; no always-on iOS claim follows.

Use the existing [LAN discovery capstone tests](../../prns-interfaces/impls/tokio/tests/wifi_auto_discovery.rs)
and [mobile validation workflow](validation.md) as starting points.

## Later desktop and browser providers

Desktop should link the native aggregate and reuse the settings service,
reconciler and UI. Keep one installation/host owner per process; closing a window
releases subscriptions rather than independently stopping networking. Follow
existing shared-instance election before attaching physical interfaces. Joining
another process does not grant authority to change its saved connections; expose
that ownership boundary. Qualify multiple windows/processes, firewall prompts,
network access, sleep/wake and joined shutdown.

Browser should first implement the app's currently unavailable web provider,
including storage and worker/session ownership. Reuse PRNS WebSocket clients
and its browser AutoWifi gateway discovery/recovery. Present that as finding
compatible local gateways, not raw LAN multicast discovery. A native daemon's
BrowserRendezvous gateway is a separate capability and is not automatically
enabled by mobile AutoLan.

Keep explicit profile kinds. Native TCP/AutoLan profiles stay preserved but
unavailable in a browser; offer a distinct supported WebSocket/gateway
configuration without reinterpreting an IP:port target. Qualify secure-origin
and local-network policies, unavailable gateways, tab suspension/resume, offline
return and browser persistence on supported browsers at that time.

Schema portability does not imply settings synchronization, identity sharing or
identical background behavior. Browser and desktop app providers remain future work.
