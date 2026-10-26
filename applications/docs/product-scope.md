# Prns product scope

The app has four purposes: direct messaging, group messaging, node management
and network inspection. It serves people who want simple everyday communication
and access to full technical control when needed. A phone must be useful without
an external board.

This document owns product and architecture direction. The [app guide](../prns/app/README.md)
owns navigation, the [roadmap](roadmap.md) owns priorities, and
[validation](validation.md) owns evidence and limits. Historical plans and
checkpoints do not add features back into scope.

## In scope

| Area | Intended experience |
| --- | --- |
| Direct messaging | Find/save contacts, send, receive, inspect delivery and recover pending work |
| Group messaging | Create, invite, join and exchange authenticated messages with device receipts |
| Named identities | Keep separate contacts and conversations for contexts such as Personal and Work |
| Node management | Pair and control authorized nodes through their supported capabilities |
| Network inspection | Inspect connections, traffic, routes and announcements without confusing them with delivery evidence |

Contacts, public messaging names, QR/link sharing and connection setup support
these journeys. Node controls remain usable for headless boards. Board-management
authority, messaging identity and group membership are separate concepts.

## Out of scope

The app does not include Explore, a NomadNet client, general network browsing,
a location/map feed or a social feed. These are not placeholders for later app
work. Supported positioning controls on a managed node remain node-management
features. Meshtastic/MeshCore users are an audience, not a promise of wire or
radio-network interoperability.

Browser/desktop native providers and a general multi-protocol framework are not
part of the current app increment. The reusable SDK and libraries can support
other consumers independently of this product's UI scope.

## Experience rules

- Open established installations into messaging. Make conversations and pending
  work easy to find; keep node administration and diagnostics reachable.
- Put routine explanations and technical identifiers behind info or Details
  controls. Keep action consequences, genuine failures and recovery choices visible.
- Distinguish local acceptance, device receipt and human reading. A current route
  or connected radio does not prove a past message's path or delivery.
- Keep an operation bound to its original identity and immutable request. An
  identity switch or uncertain result must not silently resubmit it elsewhere.
- Present capability-driven node controls. Do not guess settings, replay an
  uncertain write or infer authorization from connectivity.

## Reusable architecture

| Layer | Responsibility |
| --- | --- |
| Reticulum / Prns host SDK | Network/runtime access, interfaces, lifecycle and generic host mechanics |
| Reusable group/identity/contact libraries | Authenticated records, membership/lifecycle mechanisms, synchronization, receipts and bounded formats |
| Protocol and storage adapters | Concrete transport, persistence and cryptography with declared resource/capability limits |
| App composition | Product policy, shared-host ownership, identity-scoped stores and generated app APIs |
| React Native UI | Navigation, presentation, explicit user decisions and platform integration |

Dependencies stay one-way: app code consumes public Prns APIs; core code does
not import product policy. Rust owns protocol correctness, durable work and
recovery. JavaScript renders projections and submits commands. The generic
identity/group cores and contact/text formats remain allocation-free `no_std`,
with caller-owned buffers/storage. Native runners use `std`; MLS and service
adapters use bounded allocation. Embedded integration must declare and measure
its own budgets rather than inherit phone assumptions.

The [group runner](../../prns-groups/runner/native/README.md) exposes
[generic records](../../prns-groups/runner/native/generic-records.md),
[enrollment](../../prns-groups/runner/native/enrollment.md) and
[progress](../../prns-groups/runner/native/progress.md) independently of the app.
Headless and telemetry consumers exercise that boundary. Group communication
must also serve telemetry, state, files and commands through application profiles;
text is one profile, not the definition of a group.

The [contact-card format](../services/contact-card/README.md) binds an identity to
an arbitrary Reticulum service and can be carried by QR, links or another app.
The app chooses LXMF contacts; that choice does not belong in the generic codec.
The general [Expo host SDK](react-native-sdk-implementation.md) is a separate
reusable boundary. Direct LXMF service/mailbox extraction still needs work before
calling the entire messaging stack an independent product library.

## Protocol choices and current alpha

Direct messages retain LXMF. Groups use MLS-protected generic records and device
receipts over existing Reticulum SINGLE Links and Resources. Core Python
Reticulum compatibility remains the desired network boundary; the group profile
is not an LXMF extension. Any future core change needs compatibility evidence and
a clear reason that an application-layer solution is insufficient.

The current group profile has one immutable owner, owner-ordered additions and
removals, independent endpoint credentials and no earlier-history transfer.
Members can request departure; the owner commits their removal when reachable.
The owner can cancel unused invitations and close the group. Local deletion is
separate from membership and closure; an owner must close before deleting their
local copy. Membership changes can interrupt unfinished delivery. The
[maintainer quickstart](group-maintainer-alpha.md#alpha-limits) lists current
capacities and user-visible limits. Durable actions, storage maintenance and
exact retry recovery are implemented; their evidence remains distinct from
protected storage or sustained phone reliability.

The maintainer alpha uses test data and foreground operation. Protected phone
storage, supported backup/restore, guaranteed background delivery,
ownership transfer, attachments and linked-device identity are separate increments.
These are not prerequisites for refining this bounded demo.

## Named identities and multiple devices

[Named identities](named-identities.md) are independent local identities on one
shared node. Generated owner-bound APIs, scoped contacts/groups, pause/resume,
paused history, shared announce scheduling and cross-identity activity are
implemented. Switching the selected identity does not move existing conversations,
merge contacts or change the phone's RemoteControl grants. Local lifecycle actions
do not revoke distributed contact cards or remove group membership.

This is different from one person using the same identity across several phones.
Private-key import is provisioning, not authenticated device enrollment or state
synchronization. Before adding that product, decide:

- Whether a stable logical identity authorizes separate device keys, a designated
  mailbox collects messages, or another explicit model fits the requirements.
- Who may enroll/remove devices, what each relay learns, and how offline delivery,
  key changes and device loss interact.
- Which state is shared: messages, receipts, read state, drafts, aliases and
  deletion/archive choices need separate semantics.
- Whether a new device receives history, and how concurrent/offline changes and
  bounded retention are reconciled.

Do not assume shared private keys, database replication or wall-clock
last-write-wins solve these questions. Removing a device cannot erase plaintext
or keys it already received. Keep reusable synchronization mechanisms separate
from each application's presentation and merge policy.
