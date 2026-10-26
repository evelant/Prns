# LAN discovery and saved TCP peers

The mobile implementation restores saved LAN and TCP settings on the existing
native host. Configured TCP and iOS Bonjour work without the multicast
entitlement. Apple has approved the production app's multicast capability, and
the production configuration enables stock multicast alongside Bonjour.
Development builds retain Bonjour only. Matching distribution provisioning and
physical stock-peer qualification remain required. See the
[architecture](ip-connections-architecture.md) and
[maintainer quickstart](group-maintainer-alpha.md).

## Implementation

Installation-owned versioned settings retain a LAN preference and up to sixteen
named outbound TCP peers. Native code validates host/IP and port offline,
assigns stable identifiers, rejects duplicate endpoints and fences stale edits
with saved revisions. Malformed or unsupported saved records are preserved.
When no connection settings have ever been saved, the app starts with LAN
disabled and one enabled `PRNS public node` peer at `prns.fly.dev:4242`. Existing
saved settings keep their exact intent, including an empty peer list or a
disabled default peer; upgrades do not reinsert or reenable it. Enabling a
connection does not rerun identity setup.

A native reconciler owns attachments and retries across JavaScript and native
runtime replacement. Accepted saves survive canceled UI waiters. Replacement
awaits acknowledged retirement of sockets, supervisor children and discovery
providers. Snapshots distinguish saved and applied revisions, retain closing
attachments and expose physical peers/endpoints separately from logical totals.
TCP retries resolve hostnames again and race bounded alternate IPv4/IPv6 answers.

Interfaces → LAN & TCP includes LAN enablement, discovery details and permission recovery;
TCP add/edit/enable/disable/remove/retry; and physical endpoint/traffic details.
Setup accepts LAN, TCP or offline continuation independently of Bluetooth.

Authenticated LXMF announcements without a published display name are accepted
as unnamed discoverable messaging peers. Identity and destination checks still
apply; malformed nonempty metadata remains invalid. Each named conversation observes
its captured identity's LXMF service, so incoming messages and delivery changes
update the mounted screen without a radio poll or manual refresh. Identity and
host replacement cancel the old subscription; foreground reattachment rereads
the durable projection while keeping the selected route and loaded history.

Android uses lifecycle-owned NSD, authoritative network inventories and a
multicast lock. Empty inventories retire stale peers; unchanged notifications
preserve healthy discovery. iOS declares both Reticulum Bonjour service types.
Its production configuration explicitly enables stock multicast alongside the
managed Bonjour provider; development builds keep raw multicast disabled. The
native SDK also defaults to multicast disabled until its embedding app selects
that capability before host creation. Startup and later LAN re-enablement use
the same process policy. A path observer wakes reconciliation; replacement
subscriptions ignore historical network-change notifications. Provider failure
observations do not invent a permission diagnosis. Desktop and browser providers
remain later work.

## Validation and distribution

Run the [mobile validation workflow](validation.md) and the LAN discovery
capstone tests for the source revision being packaged. Record automated results,
package checksums and physical observations in CI/release artifacts or ignored
validation output. A native contract change requires rebuilt binaries; a
JavaScript-only update cannot qualify it.

Before distribution, test fresh installation and in-place updates with populated
identities, contacts, groups and pending work. Keep each transport's result
separate:

- Configured TCP: hostname, IPv4/IPv6, unavailable startup, peer restart, saved
  settings after app restart, and two-way verified messaging on separate networks.
- Android LAN: stock AutoInterface and PRNS Bonjour discovery, Wi-Fi loss/return,
  stale-peer retirement and permission denial for the packaged target SDK.
- iOS Bonjour: discovery and two-way messaging on a signed build without the
  multicast entitlement, including local-network denial and foreground return.
- iOS stock multicast: verify the production app's signed entitlement, embedded
  distribution profile and Info.plist policy using the
  [release guide](ios.md#release-configuration), then check discovery and two-way
  messaging with stock peers on a physical iPhone. Qualify Bonjour coexistence,
  local-network denial, network return and TestFlight installation/update on the
  same candidate.
- Combined paths: Bluetooth/LAN/TCP coexistence, direct and group messages,
  stable logical/physical counts, and removal of only the selected interface.
- Lifecycle: native restart, Android listener replacement, network changes,
  natural suspension and locked idle, with durable pending work retained.

The bundled public peer supplies an initial TCP endpoint, not an availability or
background-delivery guarantee. Confirm its availability for a trial or configure
another reachable endpoint. Foreground checks do not establish always-on iOS or
long-idle Android delivery. Multicast approval and source configuration do not
establish signed-device acceptance. Desktop and browser providers remain later
work.

Use synthetic traffic and preserve existing pending work when reporting a
failure. A connected interface, route or announcement is not a delivery receipt.
