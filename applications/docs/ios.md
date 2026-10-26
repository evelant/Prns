# iOS development

The iOS app uses the generated bindings and shared Rust image described in the
[binding guide](../prns/native-composition/bindings/README.md). Start with
[workspace setup](../README.md#setup) and [validation and limits](validation.md).
Qualify foreground operation, natural suspension and restoration separately on
the packaged build; results from an older framework do not qualify a new binary.

The app uses Expo's single-scene lifecycle, required when building with Xcode 27
for iOS 27. On SDK 57 this is enabled through `expo-build-properties` with
`ios.enableSceneSupport: true`; Expo 57.0.23 contains the scene runtime. Clean
prebuild declares `EXExpoAppSceneDelegate` and makes AppDelegate provide the React
factory without starting the window itself. Scene creation owns the UI only,
not the native node. See [Expo's SDK 57 guidance](https://github.com/expo/fyi/blob/main/ios-scene-lifecycle.md#staying-on-sdk-57-with-xcode-27).

## Release configuration

The production iOS bundle and scheme are `rs.reticulum.prns.chat`, matching the
existing App Store app; development remains `rs.reticulum.prns.dev`. Android
identifiers are unchanged. The app version is `0.1.0`; iOS uses the checked-in
1024-pixel RGB icon and restoration identifiers derived from its own bundle ID.

Set `PRNS_APP_VARIANT=production` and `PRNS_IOS_BUILD_NUMBER` before generating or
archiving a release. The build number accepts a positive decimal safe integer;
it defaults to `1` for local builds. Select a fresh number for the target App Store
version rather than reusing a historical candidate. For example, inspect the
chosen configuration from `applications/prns/app` before prebuild:

```sh
PRNS_APP_VARIANT=production PRNS_IOS_BUILD_NUMBER=37 npx expo config --type public
```

The example does not reserve or establish that build number's availability.
Rebuild native output after changing these inputs. Before distribution, inspect
the package's bundle ID, version, build number, icon and restoration metadata.
Test an in-place upgrade of an existing `.chat` installation with populated
identity, contacts and pending messages; do not uninstall or reset it. An older
production variant installed as `rs.reticulum.prns` has a different iOS container
and is not automatically migrated by this identifier change. Packaging alone
does not establish device or TestFlight acceptance.

Apple has approved the multicast capability for the production App ID. The
production configuration requests `com.apple.developer.networking.multicast`
and sets `PRNSMulticastEnabled = true` in Info.plist. Native startup reads that
explicit policy before host creation to enable stock Reticulum multicast
alongside Bonjour. Development builds omit the multicast entitlement and retain
Bonjour discovery. Shared SDK consumers default to multicast disabled unless
their embedding app configures the capability before creating a host; the first
selection or host creation fixes that policy for the process.

Refresh production provisioning profiles after enabling the capability. Before
distributing an IPA, verify that both the exported app's signed entitlements and
its embedded App Store provisioning profile contain
`com.apple.developer.networking.multicast = true`, and that its Info.plist contains
`PRNSMulticastEnabled = true`. The configuration flag does not grant permission
by itself. Local-network access is still required, and multicast adds no
background execution time. Stock-peer interoperability on a physical iPhone and
TestFlight installation/update acceptance remain separate checks.

## Build without installation

After [workspace setup](../README.md#setup), install Xcode, CocoaPods and the
`aarch64-apple-ios-sim` Rust target. From the repository root:

```sh
npm --prefix applications run native:ios:build
```

This selects the existing build helper's `--build-only` mode. It regenerates the
app-selected SDK/framework, creates the Expo iOS project, installs Pods and
compiles an unsigned Release app for `generic/platform=iOS Simulator`. It checks
the development bundle identifier, iOS minimum version, scene/Bluetooth metadata,
embedded JavaScript and the single `prns_app.framework`. It stops after build
verification without booting a simulator, installing an app or starting Metro.

The default output is
`applications/target/ios-development-client/Build/Products/Release-iphonesimulator/prnsdev.app`.
Set `PRNS_IOS_DERIVED_DATA` to place Xcode outputs elsewhere. A usable CocoaPods
executable is required; `PRNS_POD_EXECUTABLE` selects one explicitly when needed.
The `application-ios-build` suite runs this command in the
`application-mobile-build` CI matrix. It also checks that generation leaves the
canonical SDK and committed product bindings unchanged.

This builds the app's workspace composition. The detached aggregate iOS consumer
and physical lifecycle journeys remain separate qualification. For the Debug
simulator install and runtime smoke, use `native:ios:client`; for a physical
Debug install, follow the [device instructions](#physical-ios-development-client).

### Expo framework cache after an Xcode SDK update

After an Xcode SDK update, a cached ExpoModulesJSI slice can be incompatible even
when the Swift compiler version has not changed. If an `__ObjC::expo`
Swift-interface import failure points to that mismatch, rebuild the disposable
framework with Expo's own script:

```sh
PODS_ROOT="$PWD/applications/prns/app/ios/Pods" PLATFORM_NAME=iphoneos \
  bash applications/node_modules/expo-modules-jsi/apple/scripts/build-xcframework.sh --clean
```

Also remove the copied ExpoModulesJSI intermediate and embedded framework from
the selected Xcode build products before rebuilding, so CocoaPods copies the
fresh slice. Keep application data and the Rust framework. Use
`PLATFORM_NAME=iphonesimulator` for a simulator build. A successful rebuild still
needs the normal package and device checks.

## iOS native lifetime and Bluetooth restoration

The implemented direct-LXMF slice retains one native node for the iOS process,
including React provider remounts and shell routes that do not render network
state.
Inbound Link DATA is asynchronous and must continue to reach the aggregate's
sole Rust event consumer before the user opens Inbox; moving between shell
routes or unmounting the Expo module therefore does not stop and restart the
node. Explicit stop and development reset still use the bounded native
shutdown path.

Qualify UI reload by checking native uptime continuity and a fresh authenticated
peer exchange. Natural suspension, background wake and process restoration
require separate physical checks. See [validation and limits](validation.md).

Native lifetime is process-owned rather than scoped to a consuming screen.
Low-frequency development snapshot refresh runs only on `/nodes` and `/inbox`
routes and their descendants. Other shell routes keep the node and event owner
alive without periodic snapshot reads.

The app uses ordinary CoreBluetooth authorization, not AccessorySetupKit or a
per-accessory chooser. Explicit foreground Start creates the existing Rust-owned
Bluetooth managers and can trigger the normal app-level permission prompt. No
external board, OS bond or RemoteControl grant is needed to run the phone's node.
Permission denial and a powered-off radio do not prevent the local node from
starting or hide saved Inbox/Contacts data; Bluetooth remains unavailable until
the OS permits it. The native authorization status is refreshed on foreground.

The Apple Bluetooth Auto interface uses the public dual-role backend: it scans
and connects as a central, and advertises and accepts connections as a peripheral.
Configuration declares both `bluetooth-central` and `bluetooth-peripheral`
background modes and stable, distinct, variant-specific restoration identifiers.
The existing central identifier is retained. The same prepared manager owner is
handed to the native runtime; permission observation does not create another
manager or node. RemoteControl pairing remains a separate board-administration
operation, not permission to join the network.

Scene-based apps receive nil AppDelegate launch options, so `.bluetoothCentrals`
cannot be used to decide whether to recreate the managers. Each configured
process launch requests one restoration attempt with the same stable identifiers.
When Bluetooth permission is already allowed, the app delegate admits startup
and schedules preparation on the native queue without waiting for a scene or
React. Automatic startup never requests an undetermined permission in the
background. Under the supervisor lock, automatic
preparation first requires an existing storage directory and a valid primary
identity; fresh onboarding, malformed identity and reset-required state cannot
create a Bluetooth owner. Explicit user-driven Start remains unchanged.

Storage access and manager preparation do not block the main actor; authorization
and startup generation are checked before preparation and again before the full
native start. Stop/reset cancels the queued process-launch request and any unlock
retry. Unavailable protected storage can defer one recovery attempt until the
availability notification; ordinary relock does not preemptively block readable
storage. Scene disconnect, foregrounding and React reload do not request another
native start. Diagnostic `restorationAttempt`/`restorationAttemptRequested` fields
describe this attempt, not a confirmed Bluetooth wake or restored state.

That start consumes only the prepared owner with the same storage root,
restoration identifier, and Bluetooth identity instead of creating a duplicate
manager or Host. Restored central-role peripherals attach their delegates
immediately and retain bounded early callbacks while the Prns session is
rebuilt. An optional iOS Debug TCP fixture is embedded in the development
variant's `PRNSDevelopmentTcpTarget` metadata from
`EXPO_PUBLIC_PRNS_LXMF_TCP_TARGET` during prebuild. Rebuild the client to change
it; changing Metro's environment alone cannot reconfigure the process-owned
node. Native foreground and restoration attempts use the same target. Release
builds ignore this fixture. Non-Bluetooth interfaces still receive no additional
background execution entitlement.

The minimum deployment target remains iOS 18.0; removing AccessorySetupKit does
not lower or requalify it. Apple's
[TN3115](https://developer.apple.com/documentation/technotes/tn3115-bluetooth-state-restoration-app-relaunch-rules)
ties its iOS 26 AccessorySetupKit condition to specific relaunch cases; an
[Apple engineering clarification](https://developer.apple.com/forums/thread/806013)
limits the ASK-specific exception to user force-quit and Control Center Bluetooth
toggle cases. Ordinary background event delivery and state restoration remain
available without ASK. The app accepts losing those additional ASK recovery
cases to support automatic, unpaired phone networking. Qualify ordinary CoreBluetooth on the actual packaged build, separately from
any earlier ASK-based installation. The [maintainer quickstart](group-maintainer-alpha.md)
covers foreground messaging; long idle, natural suspension, process restoration,
system-radio recovery and permission changes need separate physical checks.
Source, simulator and foreground results do not establish those lifecycle cases.

Automatic connections still require ordinary app-level Bluetooth permission.
They do not require an OS bond, per-peer system picker or RemoteControl pairing.
Treat discovered peers as untrusted network access; Reticulum/LXMF authentication
and board-management authorization remain separate. The Apple backend avoids
central-initiated L2CAP where it would introduce bonding.

iOS background peripheral advertising omits the local name and puts service UUIDs
in an overflow area discoverable only by scanning iOS devices
([Apple documentation](https://developer.apple.com/documentation/corebluetooth/cbperipheralmanager/startadvertising(_:))).
For mixed-platform background discovery, test iOS scanning an Android advertiser
separately from maintaining an existing connection. Two-iPhone behavior needs its
own physical acceptance.

During an iOS-granted Bluetooth background window, the process-owned Host and
Bluetooth interface can run without React. This is bounded, event-driven iOS
background behavior, not a continuously scheduled daemon. iOS still decides
when the process may run, suspend, wake, terminate, or relaunch, and a matching
pending Bluetooth operation and corresponding event are required for
restoration relaunch. Non-Bluetooth interfaces do not gain background execution
from `bluetooth-central`; reliable background message delivery remains
unadvertised until the complete physical lifecycle has passed.

Apple documents that a passcode-protected device will not
restoration-relaunch an app until after the first unlock after reboot. The
protected-data deferral remains defense in depth for any other pre-unlock launch
path; it is not an expected CoreBluetooth restoration path.

Debug iOS builds include a private Bluetooth log probe for that qualification.
It maps existing scan, connection, subscription, disconnection, and central
restoration milestones to bounded, payload-free event codes tagged
`PRNS_IOS_RESTORATION` in the device console and unified-log messages. Filter by
that message tag, not a custom log category. It does not expose peer addresses,
messages, pairing codes, accessory identifiers, or restoration identifiers, and
Release builds do not enable it. It adds no polling or background keepalive.
Scan events distinguish requests, queued work, state queries, decisions, and
completed calls. An already-scanning decision records a framework state query,
not fresh discovery progress. Sightings occur after admission, so these events
do not prove that every raw discovery callback was observed. Greeting events
distinguish a completed Hello write from a received Welcome. In a central-role
session, Hello means the acknowledged GATT write completed; neither
event alone proves a validated, settled handshake. Closed-session reaping records
the local cancellation path, not its cause or a measured handshake timeout.
Only static event codes leave the probe; control fields are not exported.
The readiness events `bluetooth_managers_ready` and
`bluetooth_managers_timeout` cover the dual-role backend without exposing its
dynamically assigned PSM.
Reset/reconnect request markers identify the one-shot restored-native recovery
path, not a guarantee that the physical connection has closed or reopened.
The classifier and Swift
allowlist tests run in the explicit macOS `native:ios:test` gate without enabling
radio logging; portable checks also verify their source-level integration.

For startup capture on an already installed Debug build, start a PID-independent
stream before the lifecycle action:

```sh
idevicesyslog --udid <device-udid> --no-colors --exit --match PRNS_IOS_
```

Add `--network` (`-n`) when using the paired network transport. Verify the
connection and the new PID's launch plus `sequence=1` probe markers before
treating the startup capture as complete. Prefer USB for qualification and start
a fresh capture for each trial. A connected header or a still-running logger
does not prove continued reception. Verify that the reader remains live across
the lifecycle action; a lost reader requires a new capture. Treat gaps as missing
evidence, not silence from the app, even when the stream is PID-independent.
`devicectl --console` alone is insufficient: it connects standard streams only
when launching a new process, and signals sent to it can reach the app. Logging
and a Home-screen observation do not establish natural suspension or exclude
observer effects.

The app requires ordinary CoreBluetooth authorization for both native roles.
Apple explains that [declaring ASK changes the app-wide authorization model](https://developer.apple.com/forums/thread/797137)
and prevents peripheral-manager authorization. Keep ASK adoption keys and imports
out of this dual-role composition; bypassing ASK calls at runtime is insufficient.

Peripheral service restoration is not a restored Prns peer session. The backend
can recover its GATT service/characteristics, but a surviving central must
reconnect or repeat the Prns handshake before sending session data. Qualify
central-role and peripheral-role recovery separately; neither a restored service
nor foreground connectivity proves process-death message delivery.

The [validation guide](validation.md) defines automated checks and the physical
lifecycle acceptance required for each build. Keep per-run observations with
the tested source and package hashes.

## Physical iOS development client

Build and install a Debug development client on one explicitly selected iPhone:

```sh
PRNS_IOS_DEVICE_UDID=<device-udid> \
PRNS_IOS_DEVELOPMENT_TEAM=<team-id> \
PRNS_IOS_METRO_PORT=8088 \
  npm --prefix applications run native:ios:device
```

Automatic signing with permission to update profiles is the default. To limit
Xcode to profiles already installed on the Mac, provide the expected profile
UUID. The helper omits developer-portal permission and refuses to install the
app unless that exact profile is embedded:

```sh
PRNS_IOS_EXPECTED_PROVISIONING_PROFILE_UUID=<profile-uuid> \
PRNS_IOS_DEVICE_UDID=<device-udid> \
PRNS_IOS_DEVELOPMENT_TEAM=<team-id> \
PRNS_IOS_METRO_PORT=8088 \
  npm --prefix applications run native:ios:device
```

The helper does not launch the app. It stops before installation unless the
compiled app has bundle identifier `rs.reticulum.prns.dev`, the requested Metro
port, and a nonempty `ip.txt`. Start Metro on the same LAN and port before
launching the installed app:

```sh
npm --prefix applications/prns/app run start -- --lan --port 8088
```

This Debug build has no embedded JavaScript bundle, so it requires that Metro
session to load the UI. Open the installed `prns dev` app after Metro reports
that it is ready. This custom Debug app does not include `expo-dev-client`;
opening an `expo-development-client` deep link does not configure its bundle.
Expo reads the built `RCTMetroPort` and the host in the app's `ip.txt` instead.

If the app already shows **No script URL provided**, starting Metro alone does
not retry the failed launch. Once Metro is ready, reload from the developer
menu or restart the failed process:

```sh
xcrun devicectl device process launch \
  --device <device-udid> \
  --terminate-existing --activate \
  rs.reticulum.prns.dev
```

Use that restart only for launch troubleshooting. During a same-process
background/resume check, preserve the running process and foreground the app
normally without a terminating launch command.

### Standalone development build

To qualify startup without Metro, use the physical Release mode. It retains the
same development bundle identifier and installs over the existing app; it does
not uninstall the app or clear its identity, messages or pairings:

```sh
env -u EXPO_PUBLIC_PRNS_LXMF_TCP_TARGET \
  PRNS_IOS_EXPECTED_PROVISIONING_PROFILE_UUID=<profile-uuid> \
  PRNS_IOS_DEVICE_UDID=<device-udid> \
  PRNS_IOS_DEVELOPMENT_TEAM=<team-id> \
  npm --prefix applications run native:ios:client -- --device-standalone
```

This builds the Rust framework and app in Release mode, validates a nonempty
`main.jsbundle` and the compiled metadata, verifies the code signature, then
installs only on the selected phone. It uses
`applications/target/ios-standalone-device` unless `PRNS_IOS_DERIVED_DATA` is set.
The existing profile check remains available; omitting it permits automatic
signing with developer-portal updates. This is a locally signed development
build, not a production-distribution build.

Release uses the embedded JavaScript and does not use the Debug-only development
TCP peer. Qualify startup with Metro stopped and no inspector attached. Preserve
and compare the public identity, contacts, pairings and existing group histories
before and after installation; then exercise a synthetic group exchange, device
receipts and a process restart. Record the actual transport used separately:
successful standalone startup does not establish Bluetooth delivery or background
execution. Never clear app data or replace an enrollment fixture to make a
retained-data qualification pass.
