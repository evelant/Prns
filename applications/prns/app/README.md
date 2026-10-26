# Prns app

Prns brings together direct messages, group conversations, node management and
network inspection. Named identities keep contexts such as Personal and Work
separate on one phone. The app runs on iOS and Android; the web build previews
the interface without a native messaging or node runtime.

Start with the [maintainer quickstart](../../docs/group-maintainer-alpha.md) to
try a group demo. Developers should use the [workspace setup](../../README.md),
[iOS](../../docs/ios.md) or [Android](../../docs/android.md) guides.
[Validation](../../docs/validation.md) records what has been tested, and the
[roadmap](../../docs/roadmap.md) lists the remaining work.

## Find your way around

| Area | What it contains |
| --- | --- |
| Inbox | Direct and group conversations, new messages/groups, invitations and saved actions |
| Contacts | Saved contacts, signed QR/link sharing, contact import, public messaging name and discovery |
| Nodes | This phone, paired nodes, connection setup and node controls |
| More | Named identities, network inspection, app settings and About |

Established installations open Inbox. The identity control selects the default
for Inbox and Contacts; an existing conversation keeps its original owner.
Network inspection is also reachable from Nodes and Connections.

## Messages and identities

**More → Identities** creates, names, pauses and resumes independent identities.
Contacts, messages, groups and durable actions belong to their original identity.
Other Active identities continue receiving while a different one is selected.
Incoming activity appears in the identity picker; opening a conversation records
local viewed state. Selection alone does not clear unread messages, and viewed
state is not a receipt sent to the other phone.

Contacts keep private aliases separate from names people publish. **My contact
QR** and **Share contact link** carry the same signed portable contact card.
**Scan or paste contact** explicitly asks which identity should save it; scans
started from a group remain bound to that group. A valid card establishes an
identity binding, not reachability or group membership.

Direct messages use LXMF. **New message** accepts saved/discovered contacts or a
manual messaging address. A bounded lookup can resolve missing recipient details
before a send is accepted. **Finding contact…** is not durable acceptance.
Incoming sender verification is separate from delivery evidence: an unverified
sender's name is shown as a claim, even when the address is saved.

Groups use the reusable group library with authenticated enrollment and device
receipts. **New group → Invite a device** selects a contact or scanned card;
the recipient must accept and finish joining. **Group details** contains members
and invitation history; **Delivery details** contains recent synchronization
observations. Follow the [quickstart](../../docs/group-maintainer-alpha.md) for
receipt meanings, recovery and current group limits.

Pressing a group action stores its exact input and request ID before execution.
Restarting or leaving a screen does not lose accepted work. An uncertain outcome
must use the saved action or **Retry same…**, not a replacement message/invitation.
Text not yet submitted is still a volatile draft. Changing membership can
interrupt unfinished delivery from the previous membership.

## Connections, announcements and network inspection

**Nodes → This phone → Connections** shows Bluetooth permission/radio state and
actual physical peers. Enable/disable preserves identities and messages and does
not stop the shared node. A ready radio, a saved route and an authorized pairing
are different from a current connection to the intended recipient.

**Advanced Bluetooth discovery** accepts one to four shared discovery-group names.
The default is `reticulum`; phones need a matching name to discover one another.
Changing names can disconnect peers and requires confirmation. Saved and applied
settings are shown separately. Discovery names are not encryption or authorization.
Use the [LoRa qualification procedure](../../../prns-groups/transport/reticulum/docs/lora-qualification.md)
for isolated phone/board paths; seeing a board does not prove a message crossed LoRa.

**Announce yourself** advertises the current identity's messaging contact.
**Inbox → Group connection settings** controls group-service announcements,
including a manual announce and a saved interval. The app default is hourly with
jitter; **Manual only** disables periodic/startup announcements. Announcements
consume network bandwidth and aid discovery; they are not message delivery.

**Network** separates physical connections, current routes and recent accepted
announcements. It shows traffic and route age/expiry with optional technical IDs.
Observations reflect the latest native snapshot. Current routes cannot prove the
path of an earlier message. Announcement history is session-local; clearing it
preserves contacts, routes and messages.

**More → About → Build details** shows configured version/build metadata and an
optional build label. The API contract identifies the interface, not the exact
source revision. **Diagnostics** reads a compact status snapshot that can be
reviewed, copied or shared. It excludes message content, names, addresses,
identity/device IDs, keys and raw errors. A connected-peer count is not delivery
evidence; include the relevant receipt or error separately when reporting a stall.

## Manage a node

Pairing authorizes control; it is not a group invitation or a live connection.
Use **Nodes → Pair a node**, then **Manage node** for controls supported by that
node and your grant. The [node management guide](../../docs/node-management.md)
covers pairing, connection checks, settings, disruptive changes and uncertain
results. Headless boards must use their supported pairing method; the app does
not require a board display or infer identity from signal strength.

## Connection readiness and cancellation

Managed-node commands check current authority and route/interface readiness.
The app can make a bounded path lookup before connecting; a cached route alone
is insufficient. Leaving a screen cancels caller-owned reads, but cannot undo an
accepted durable write. An unknown write result is checked before another attempt;
it is never automatically replayed. See the
[binding ownership guide](../native-composition/bindings/README.md#cancellation-and-ownership)
for the reusable API contract.

## Routes and implementation

The [navigation catalog](src/navigation/catalog.ts) owns screen IDs, route
parameters and supported deep links. Update it together with route files, links
and tests. Contacts/direct conversations use destination hashes, groups use group
IDs, invitations use invitation IDs, and managed nodes use target identity
fingerprints. Out-of-scope routes are removed rather than advertised as future
features. Unsupported or malformed links must not execute a mutation.

The app renders native projections; Rust owns enrollment, synchronization,
receipts and recovery. Generated bindings are outputs. Keep protocol behavior
in reusable libraries, not screens or navigation. The
[product scope](../../docs/product-scope.md) describes these boundaries and the
[validation index](../../docs/validation.md) separates host checks, packaged
builds and physical trials.
