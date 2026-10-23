# LAN discovery and saved TCP peers

The mobile implementation restores saved LAN and TCP settings on the existing
native host. Configured TCP and iOS Bonjour work without the multicast
entitlement. Apple multicast approval was requested on October 21; stock
multicast-only discovery on iOS remains a separate approval, provisioning and
physical-device qualification lane. See the [plan](ip-connections-plan.md)
and [maintainer quickstart](group-maintainer-alpha.md).

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

Connections includes LAN enablement, discovery details and permission recovery;
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
preserve healthy discovery. iOS declares both Reticulum Bonjour service types
and uses its existing managed provider with raw multicast disabled. A path
observer wakes reconciliation; replacement subscriptions ignore historical
network-change notifications. Provider failure observations do not invent a
permission diagnosis. Desktop and browser providers remain later work.

## Development-device evidence

The earlier containing working tree was tested in standalone development apps.
These observations do not qualify this extracted PR's distribution packages.
Detailed workstation logs, phone captures and account-request records are kept
outside the published source tree.

- Android 10/API 29, target SDK 36: in-place development-app update retained
  sampled identity, contacts, group and Bluetooth state. Configured TCP passed
  two-way verified LXMF, saved-profile restart and automatic reconnect after
  server return. With saved TCP disabled, stock Reticulum AutoInterface passed
  two-way LXMF. PRNS Bonjour resolution was checked separately. Wi-Fi loss
  retired LAN peers to zero; the same screen recovered peers and a verified
  message on return without Retry. The final wording-only package received a
  narrower in-place smoke check.
- iPhone 14 Pro: a separate development installation used allowed Local Network
  access and denied Bluetooth. Configured TCP passed saved-profile restoration,
  automatic network-return recovery and strict two-way LXMF. The Python fixture
  used a previously verified public sender-key cache; this does not establish
  fresh ordinary identity discovery. With configured TCP disabled, Bonjour
  rediscovered physical LAN peers after Wi-Fi return without Retry, with
  correlated endpoint and outbound traffic. Two-way messaging over Bonjour was
  not established. The final signed binary had no multicast entitlement.

No existing production app was erased or replaced. The separate iOS development
installation does not establish an upgrade of the populated production bundle.
No always-on iOS or long-idle Android delivery claim follows from these checks.

## Source qualification and alpha handoff

The [public TCP and live-refresh follow-up](ip-connections-followup-2026-10-22.md)
records the later two-phone foreground trial and its source boundaries.

The extracted PR is checked separately from the containing working tree.
CI and the PR description record exact source checks and the publication-gate result.
The native contract requires rebuilt binaries; existing phone packages are not
updated by a JavaScript-only release.

Before maintainer distribution, package the selected containing revision with
stable signing and visible build identification. Check fresh installation and
in-place updates with populated identities, contacts, groups and pending work.
The bundled public peer provides an initial TCP path, not an availability or
background-delivery guarantee. Confirm its operating window for a maintainer
trial, or configure another reachable endpoint. Repeat ordinary contact/announce
discovery, direct and group messages, mixed Bluetooth/LAN/TCP paths, and recovery on those
exact packages. Keep API 37 permission-denial, iOS stock multicast and natural
background/locked-idle acceptance distinct from the completed foreground checks.

Use synthetic messages and preserve original pending work when reporting a
failure. A connected interface, route or announcement is not a delivery receipt.
