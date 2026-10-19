# Prns product scope

The product focuses on direct messaging, group messaging, node management and
network inspection. Everyday communication should be simple, with technical
details and supported controls available when needed. A phone must be useful
without an external board. Meshtastic/MeshCore users are an audience, not a
promise of radio-network or wire interoperability.

This checkout's app boundary provides direct LXMF messaging, a local identity,
contacts, node controls and network inspection. Group messaging and named
identity UI are not integrated here. Group libraries, named identities and their app integration are separate
work. This boundary does not establish an app group journey or a board acting
as a complete group participant.

The [app guide](../prns/app/README.md) describes existing behavior,
[roadmap](roadmap.md) prioritizes remaining integration, and
[validation](validation.md) defines acceptance. Legacy shell routes do not add
features to scope. Explore, NomadNet browsing, location/social feeds and native
browser/desktop app providers are outside this app increment.

## Experience and evidence

- Keep conversations and pending work easy to find. Put routine explanation and
  identifiers behind Details; show failures and consequential actions clearly.
- Distinguish sender authentication, transport proof, durable device receipt and
  human read state. One receipt does not establish receipt by every group member.
- Treat physical connections, routes, contacts and management grants separately.
  A current route cannot reconstruct the path of an earlier message.
- Bind accepted work to its original identity and immutable request. Cancellation
  cannot undo a durable mutation or justify a replacement after an unknown result.
- Present node controls from live capabilities and authority. Headless nodes must
  remain usable without requiring a board display.

## Reusable architecture

| Layer | Responsibility |
| --- | --- |
| Reticulum / host SDK | Network access, interfaces, runtime and platform lifecycle |
| Reusable services and group mechanisms | Authentication, bounded records, delivery/recovery and storage contracts |
| Protocol/storage adapters | Concrete transport, cryptography and persistence with declared resource limits |
| App composition | Product policy, service ownership, storage and generated application APIs |
| UI | Presentation, explicit decisions and platform interaction |

Dependencies stay one-way. Rust owns protocol behavior and durable recovery;
JavaScript renders state and submits commands. The generic group core remains
allocation-free `no_std`; MLS and storage adapters have their own bounded
allocation requirements. Text messaging is one use case alongside telemetry,
shared state, files and commands. App policy must not become a dependency of the
generic mechanisms.

Direct messages retain LXMF and its interoperability checks. The
group design (separate integration) defines application-layer
security and synchronization over existing Reticulum primitives. Preserve core
Python Reticulum compatibility; a core extension needs a concrete reason and
compatibility evidence. Group profile support is not ordinary LXMF interoperability.

The existing LXMF mailbox is useful groundwork, but its product storage coupling
still needs review before describing the full direct messaging stack as an
independent library. Each reusable boundary needs an independent consumer with
explicit runtime, storage and capacity requirements.

## Separate identity and lifecycle decisions

A managed board, Reticulum destination, person and group are different concepts.
Importing the same private key on several phones is not device enrollment or
state synchronization. A linked-device product still needs authority for adding
and removing devices, offline/key-loss behavior, new-device history policy and
conflict rules for messages, receipts, read state, drafts and contact aliases.

Protected storage, backup/restore, guaranteed background delivery, attachments
and complete embedded group custody require their own design and acceptance.
Use explicit test data for the development app; do not infer those properties
from host tests, source exports or foreground phone trials.
