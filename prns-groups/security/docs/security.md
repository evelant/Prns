# Group communication encryption and privacy analysis

Updated October 12, 2026. Security analysis of the proposed
[initial contract](../../core/docs/v0.md), compared with Reticulum and LXMF.
End-to-end private profiles can keep content secret from non-member custodians,
but do not hide all group metadata. Forward secrecy depends on key deletion at
every relevant endpoint; durable history and offline recovery deliberately retain
additional ways to recover content. The proposed maintenance defaults below do
not establish a bounded recovery time after compromise in cached/offline mode.

The group visibility, epoch, recovery and maintenance analysis describes the
**MLS private profile**. Current implementation limits are listed in the
[adapter README](../mls-rs/README.md#deliberately-incomplete-integration): proposed
time-based grace, membership removal and history repair must not be read as
implemented app guarantees. The [embedded guide](../../core/docs/embedded.md)
separates resource gates from protocol requirements.

The [secure flash lifecycle](../../store/docs/secure-flash.md) specifies
how to retire durable MLS snapshots without losing crash recovery. It separates
local erasure from hostile rollback and previously captured flash images, neither
of which a permanent device wrapping key alone solves.

In particular, static recipient encryption does not provide forward secrecy
against later compromise of that recipient key or automatic post-compromise
recovery. Producer, group, era, audience, recipient and key bindings must still
be authenticated. A new recipient key wrapper can disclose history and requires
the same explicit authorization as re-encryption. Generic authorization barriers
invalidate prior-era live admission; they do not universally purge a long-lived
recipient key used by other groups. The merge-then-purge rules below are MLS-specific.

## Alternatives considered

Addressing, durable records, custody and receipt convergence do not require MLS.
The private-profile choice concerns compromise guarantees and resource costs.

| Approach | Useful property | Tradeoff |
| --- | --- | --- |
| Signed public records | Small baseline for non-sensitive data and blind storage | No confidentiality; producer and admission policy still need authentication. |
| Reticulum Links only | Protects an online transfer | A terminating relay sees plaintext without inner encryption; this is not blind offline custody. |
| Recipient-encrypted objects | Independent asynchronous delivery; one body can have per-recipient key wrappers | O(N) wrappers and no automatic PCS; later recipient-key compromise can expose recorded objects. A new wrapper is authorized history disclosure, not retry. |
| Established pairwise ratchets | Independent device repair and FS after key deletion | Per-peer sessions, prekeys, skipped keys and fan-out. Healing needs fresh ratchet input; Sesame does not supply shared group governance. |
| Owner epoch keys or sender-key chains | Efficient repeated group ciphertext | Rekey distribution, sender authentication, skips and healing remain substantial protocol work; static wrapping does not heal a compromised recipient key. |
| MLS | Standardized asynchronous group key agreement, sender ratchets and FS/PCS mechanisms | Commit/Welcome catch-up and bounded retained state; owner/join peaks can dominate. Public tree state is O(N). It supplies neither custody nor application merge rules. |

[HPKE's limits](https://www.rfc-editor.org/rfc/rfc9180.html#section-9.7.4),
[Sesame](https://signal.org/docs/specifications/sesame/) and the
[Double Ratchet](https://signal.org/docs/specifications/doubleratchet/) informed
this comparison. Constrained-device group-manager patterns in
[ACE Group OSCORE](https://www.rfc-editor.org/rfc/rfc9594.html) also require
nonce/replay/rekey state; access-control “forward security” is not FS after later
state compromise. None is a silent fallback when MLS cannot fit.

The selected `mls-rs` backend supports `no_std + alloc`. The inspected OpenMLS
revision depends on `std`; changing its SQLite provider alone cannot make it
embedded. MLS++ adds C++/OpenSSL integration, and no suitable allocation-free
backend was established by the evaluation. Preserve this implementation choice
separately from the MLS protocol choice; the pinned RustCrypto provider still
requires security and target qualification.

## What each security property means

- **Content confidentiality:** which parties can decrypt a publication now.
- **Forward secrecy:** which previously recorded ciphertext remains protected
  after a particular later key/state compromise. Name the compromised material;
  stealing an identity key, a live session and a full backup are different cases.
- **Post-compromise security:** when future traffic becomes secret again after
  an attacker loses access and fresh, authenticated key material takes effect.
- **Metadata privacy:** what identities, group relationships, sizes, times and
  activity remain observable despite content encryption.
- **Retention and erasure:** what plaintext, keys, journals and backups remain
  recoverable. A delivery deadline or deleted database row is not key erasure.

Assume attackers can record, delay, suppress and replay network traffic. Consider
later endpoint/store compromise separately from a persistently compromised
member: a member that continues receiving plaintext can continue leaking it.
Credential verification, correct cryptography, secure randomness and the declared
storage assumptions are prerequisites. Encryption cannot ensure availability or
prevent a legitimate reader copying content.

## Reticulum protections and their limits

| Reticulum mechanism | Content protection | Later compromise and visibility |
| --- | --- | --- |
| `SINGLE`, recipient identity key | Fresh sender ephemeral X25519 key combined with the recipient's long-term public key. | Later theft of the recipient's private encryption key can decrypt recorded packets. Fresh per-packet sender keys alone do not give receiver-compromise forward secrecy. Destination and traffic metadata remain visible. |
| `SINGLE`, destination ratchet | Same construction, targeting an announced rotating recipient key. | Identity-key theft alone does not recover independently generated ratchet secrets. A stolen retained ratchet decrypts its recorded traffic; protection against full state theft develops only as those secrets disappear. This is a rotating destination-key mechanism, not a Double Ratchet conversation. |
| Link | Authenticated ephemeral DH establishes a symmetric key between the two Link endpoints, across intervening routers. | Later identity-key theft alone does not recover an earlier session. Theft of the session secret exposes recorded traffic protected by that session and further traffic until replacement. There is no message-by-message DH ratchet within a Link. |
| Resource | Bulk transfer protected with its Link's encryption. | Shares the Link's compromise boundary, with public proof information described below. A relay terminating the Link sees the transferred application bytes. |
| `GROUP` / `PLAIN` | Shared symmetric key / no payload encryption. | Shared-key theft exposes recorded GROUP traffic; PLAIN has no secrecy. Neither supplies evolving group membership or forward secrecy by itself. |

The reference uses X25519, Ed25519, HKDF and authenticated AES-CBC tokens; current
identity encryption and default Links use AES-256-CBC with HMAC-SHA256. Security
differences above primarily follow key lifecycle, rather than an AES strength
ranking. [Identity encryption](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Identity.py#L815-L908),
[Link exchange](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Link.py#L333-L373).

Destination ratchets are optional at the Reticulum layer. The Python default
retains 512 private ratchets and rotates on qualifying announces with a 30-minute
minimum interval during uninterrupted operation. At that rate, 512 generations
span roughly 10.7 days; sparse announcing retains secrets much longer and repeated
restarts can consume generations faster. This is not a fixed secrecy period.
Public-key cache expiry is a different policy.
Receiver-side ratchet enforcement cannot undo exposure after a sender has already
encrypted to the static identity key.
[Destination lifecycle](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Destination.py#L206-L237),
[defaults and enforcement](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Destination.py#L477-L538).

Encryption to a public SINGLE key does not authenticate the application sender:
anyone knowing that key can construct a packet. A Link authenticates its responder;
the initiator's durable identity is optional. Our producer signatures and role
checks remain necessary.

Reticulum omits an ordinary packet source address, and a Link initiator need not
identify itself. Explicit identification happens inside the Link. This reduces
protocol-level identity disclosure; it does not hide a nearby radio transmitter,
an IP connection's peer, timing correlation or an application's own identifiers.
Routing nodes observe destination/link identifiers, direction, packet context,
sizes and timing. Public signed announces expose public keys and reachability;
the core does not encrypt announce application data, although an application may
supply ciphertext. Reusing an identity across services permits correlation.
[Reticulum transport and identification](https://reticulum.network/manual/understanding.html#reticulum-transport),
[packet framing](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Packet.py#L179-L235).

IFAC/network-access protection can restrict who can interpret frames on a segment.
It is a shared network boundary, not anonymity from participating routers or
end-to-end protection from an application relay. Physical and carrier metadata
can remain visible. Later theft of shared interface credentials can expose
recorded masked frames; it does not recover the inner endpoint encryption keys.
[IFAC construction](../../../prns-core/src/interfaces/packet/ifac.rs).

Two implementation details prevent stronger blanket claims:

- Python Link closure clears several key fields but leaves its cached `Token`
  reference. A retained Link object can therefore retain symmetric key material.
  Session forward secrecy assumes effective disposal; closure is not evidence of
  secure memory erasure. [Link cleanup](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Link.py#L685-L694),
  [cached token use](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Link.py#L1166-L1183).
- Resource proofs carry `resource_hash || H(plaintext_segment || resource_hash)`
  without Link encryption. An observer able to read that proof can test guesses
  for the **entire** uncompressed segment, including its application framing.
  This does not reveal arbitrary content, but raw predictable data needs care.
  The Resource's internal random prefix is stripped before this calculation.
  MLS ciphertext as the transferred bytes avoids a simple plaintext-content
  dictionary. Direct repair/control transfers need protected unpredictable framing
  per segment. This is a source-derived consequence, not a demonstrated attack
  against LXMF or a live deployment.
  [Resource assembly/proof](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Resource.py#L694-L771),
  [unencrypted proof packing](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Packet.py#L199-L201).

PRNS implements the same Resource proof construction and clear proof emission.
Its host destination default is `NoRatchets`; it exposes stronger ratchet policies
explicitly. Python LXMF defaults must not be assumed for every PRNS consumer.
[Proof calculation](../../../prns-core/src/routing/links/resources/assemble_incoming.rs),
[proof emission](../../../prns-core/src/routing/links/resources/receive/conclude.rs),
[host defaults](../../../prns-host/core/src/config.rs).

## LXMF comparison by delivery mode

This comparison covers ordinary private `SINGLE` destinations in Python LXMF.

| LXMF mode | Who can read content? | Forward secrecy and relay information |
| --- | --- | --- |
| Direct | Sender and recipient; intervening Transport nodes cannot decrypt the Link. | Inherits Link session secrecy. The signed LXMF message is inside that Link; it is not independently encrypted for storage by an arbitrary intermediate application. |
| Opportunistic | Intended recipient; application sender identity/signature/body are inside destination encryption. | Inherits the static-key or destination-ratchet case. Destination address, size and timing are visible on readable network frames. Explicit PLAIN destinations are a public exception. |
| Propagated | Recipient can decrypt the inner envelope. A non-recipient propagation node stores ciphertext. | Inner secrecy depends on the recipient identity/ratchet key, independently of the outer Links. The node sees destination address, ciphertext equality/transient ID, size, propagation stamp and transfer/local receipt timing. Original sender, signed message timestamp and body remain inside encryption. |
| Paper/URI | Holder of the recipient decryption material. | Same destination-encryption key lifecycle. Someone copying the artifact gets destination plus ciphertext and can retain it indefinitely. |

LXMF signs the source/destination and message data. Its propagated representation
leaves the destination outside the encrypted remainder; the propagation timestamp
is not the original signed message timestamp.
[LXMessage packing](https://github.com/markqvist/LXMF/blob/c3ff2d6dc2f256daab896dadc044dd5a913ecbb7/LXMF/LXMessage.py#L359-L463).

Delivery destinations enable ratchets, but router `enforce_ratchets` defaults to
false. Sending uses a known ratchet or the static destination key. A policy that
requires forward secrecy must check the sender's selected method/key and refuse
an unacceptable fallback before emission; receipt-side rejection is too late.
[LXMF default](https://github.com/markqvist/LXMF/blob/c3ff2d6dc2f256daab896dadc044dd5a913ecbb7/LXMF/LXMRouter.py#L101-L105),
[registration](https://github.com/markqvist/LXMF/blob/c3ff2d6dc2f256daab896dadc044dd5a913ecbb7/LXMF/LXMRouter.py#L356-L372),
[Reticulum key selection](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Destination.py#L596-L610).

A recipient identifies itself to its propagation node when downloading. New
upload Links need not identify the uploader, but reuse of a previously identified
Link can associate later uploads with that identity; peer federation Links also
identify peers. None of this makes the encrypted original author field public,
but it supplies useful correlation.
[Download identity](https://github.com/markqvist/LXMF/blob/c3ff2d6dc2f256daab896dadc044dd5a913ecbb7/LXMF/LXMRouter.py#L510-L526),
[upload Link reuse/creation](https://github.com/markqvist/LXMF/blob/c3ff2d6dc2f256daab896dadc044dd5a913ecbb7/LXMF/LXMRouter.py#L2946-L2986).

Oversized opportunistic sends switch to Direct. Exhausted Direct delivery calls
an application failure callback; automatic Direct-to-Propagated fallback is not
a universal library behavior. Applications choosing that transition change the
secrecy/retention boundary and should expose the effective mode.
[Failure handling](https://github.com/markqvist/LXMF/blob/c3ff2d6dc2f256daab896dadc044dd5a913ecbb7/LXMF/LXMRouter.py#L2927-L2933).
Propagated retries normally reuse the cached inner ciphertext, so retrying upload
does not automatically switch it to a newer destination ratchet.

<a id="proposed-private-group-path-and-visibility"></a>

## Proposed MLS group path and visibility

There are two independent protections: a Reticulum Link to a service endpoint,
and an MLS publication encrypted for the group. A custodian terminates the first
and stores the second. The receiving member later fetches it over another Link.
Transport routers between these endpoints do not terminate either Link.

**An opaque custodian is content-blind, not metadata-blind.** Raw MLS
`PrivateMessage` exposes `group_id`, `epoch`, `content_type` and
`authenticated_data`; its sender data and application payload are encrypted.
Scoped storage handles do not conceal those headers or equality of the stored
ciphertext. Keep sensitive values out of AAD.
[MLS private framing](https://www.rfc-editor.org/rfc/rfc9420.html#section-6.3).

| Actor and moment | Information available in the proposed MLS profile |
| --- | --- |
| Passive observer / forwarding router during transfer | Reticulum/carrier metadata and public announces; not inner MLS content or headers protected by the Link. Public proofs remain an exception to blanket payload-hiding claims. |
| Non-member custodian at upload, fetch, deletion or later disk compromise | Raw MLS header, stored ciphertext, lengths, equality, access handles/capabilities and contact times; authenticated client identity if the service requires it. It can associate accesses with a group and infer participation. It does not automatically learn the full roster or plaintext author/body. |
| Group owner | Full roster, grants and control history; also group plaintext because this v0 owner is an MLS member. It can authorize new readers. It is not the hypothetical blind admission gate from the wider design. |
| Current member obtaining a publication | Group content, verified producer/device identity, relevant receipts and roster/control information. Subscription or delivery-target filtering provides no secrecy from an eligible key holder. |
| Newly added member | Current group state and new-epoch content; history only through explicit grants. Re-encrypting old content to the new whole group would disclose that history, so live repair must preserve the original audience. |
| Removed member | Everything previously learned and any stale-epoch content for which it retains keys. It may see the removal transition protected under the preceding epoch. Removal cannot erase copies; exclusion concerns the resulting epoch and senders using it. |
| Outside producer and ingress | Producer knows its submission and granted ingress information. The ingress member sees plaintext and original provenance before MLS publication. A blind custodian and a plaintext-reading ingress are different roles. |
| History source and receiving device | The explicitly imported old plaintext and its provenance. Import creates another recoverable copy and exposure under the new transfer's keys. |

Private rosters require more than encrypted application messages. Member
handshakes and accompanying governance records must be encrypted; KeyPackages
disclose credentials/capabilities to their directory, and public trees disclose
membership. Welcome's visible KeyPackage references can identify joiners to an
observer holding the package pool. Do not publish GroupInfo/trees for external
joins in v0. Use an encrypted Welcome tree or a protected owner-to-joiner transfer.
[MLS membership metadata](https://www.rfc-editor.org/rfc/rfc9420.html#section-16.4),
[KeyPackages](https://book.openmls.tech/user_manual/create_key_package.html),
[explicit OpenMLS configuration](https://book.openmls.tech/user_manual/group_config.html).

The public profile intentionally permits content disclosure. Link protection
still protects exchanges on the network, but gives no enduring secrecy from
readers or services permitted to distribute the public data. External objects are
outside v0's content-protection contract; encrypting a reference does not establish
the referenced object's protection.

## Forward secrecy across the lifecycle

| Event or compromised material | Consequence for the MLS group |
| --- | --- |
| Relay disk/capabilities stolen, no member secrets | Ciphertext and relay metadata exposed; no group plaintext solely from those assets. Availability, access privacy and deletion controls may be compromised. |
| Old Reticulum identity key stolen | Does not alone recover past ephemeral Link sessions or MLS application keys. It can enable future impersonation; reauthenticate/revoke credentials appropriately. |
| Live Link session secret stolen | Exposes that session's outer traffic. Inner MLS content remains protected unless group keys are also obtained. Direct plaintext ingress/history exchanges have no inner MLS protection. |
| MLS state snapshot after processing messages | Erased message keys can protect earlier ciphertext. Retained skipped keys, past epochs and unprocessed sender state enlarge the recoverable interval. A snapshot can expose future traffic until an effective healing update. |
| Offline member state stolen | The member may still hold old decryption state erased by active peers. One active device's deletion does not establish a group-wide forward-secrecy bound. |
| Plaintext archive, retry cache or readable backup stolen | All retained content in that copy is exposed, regardless of how well the original network keys were erased. At-rest encryption only helps against attackers lacking its unlocking keys. |
| Attacker loses state access, then fresh uncompromised key material is incorporated | Future secrecy depends on the relevant compromised member replacing its leaf secrets and the accepted transition taking effect. A persistent compromise or stolen credential permitting active impersonation needs additional remediation. |

OpenMLS deletes used sending/decryption keys from active state and provides bounded
out-of-order/past-epoch retention controls. Re-encryption requires a recoverable
content copy; exact retransmission reuses the persisted envelope without plaintext
or old message keys. Deletion from active state does not remove old copies in
storage journals, backups, crash dumps or application history.
[OpenMLS key lifecycle](https://book.openmls.tech/forward_secrecy.html).

Time-based past-epoch deletion also needs explicit application execution and
persistence; choosing a duration is not a background erasure guarantee.
[Past-epoch deletion](https://book.openmls.tech/user_manual/past_epoch_secret_deletion.html).

### Retaining delayed messages across routine rekeys

The [delayed-delivery policy](../../core/docs/policies.md#delayed-delivery-across-routine-rekeys)
now permits bounded old-epoch application delivery within one unchanged
authorization era. An add/remove, credential/role/policy change or declared
compromise recovery ends that era and starts a fresh MLS epoch. Even a role-only
barrier must advance keys so that purging past state removes pre-barrier receive
material. A routine Update is not an admission barrier.
This replaces the earlier recommendation to keep zero completed application
epochs for every MLS group.

Keep the evolving receive state, not old complete MLS snapshots or exported
epoch master secrets. In the inspected OpenMLS source, past `MessageSecrets`
contain the modified secret tree plus sender-data, membership/confirmation and
verification context. Used message keys can remain deleted while skipped keys
and unprocessed sender branches survive. A later theft can expose that remaining
content and sender metadata. Count/forward-derivation caps bound honest resource
use; someone holding a ratchet head can derive later generations independently.
[Retained state](https://github.com/openmls/openmls/blob/e0b21a70a55a9014b8de2772b36bf3d638adad51/openmls/src/group/mls_group/past_secrets.rs),
[message-key consumption](https://github.com/openmls/openmls/blob/e0b21a70a55a9014b8de2772b36bf3d638adad51/openmls/src/tree/sender_ratchet.rs#L274-L339).

Retention of old message-decryption state does not by itself recover a freshly
healed epoch's keys. Nevertheless, a routine update no longer ends all acceptance
of old-key traffic immediately. An attacker with old encryption state and the
necessary signing keys can create apparently delayed records during grace. An
intact independent producer signature prevents novel-content forgery without its
key. Explicit compromise recovery needs a barrier plus the previously specified
credential remediation, not merely a routine update. No sender timestamp proves
that a record existed before a removal or compromise response.

The trial **48-hour window is measured from an owner-authenticated epoch closure**,
not from first receipt, local installation or restart. The receiver needs both
that fixed cutoff and the immutable record deadline. OpenMLS's inspected time
metadata is assigned when the receiver installs an epoch as current; its duration
API therefore does not implement our closure rule automatically. An epoch that
remained current for a week and an old epoch installed during catch-up illustrate
the difference. Track authenticated cutoffs durably and prune before admitting
late content; clock uncertainty must not silently extend grace.
[Local timestamps and pruning](https://github.com/openmls/openmls/blob/e0b21a70a55a9014b8de2772b36bf3d638adad51/openmls/src/group/mls_group/past_secrets.rs).

Public APIs support coarse past-state deletion and oldest-prefix/count pruning,
not arbitrary epoch-ID or byte-budget selection. Our ordered cutoffs permit
prefix eviction, but the host must map those epochs, enforce byte caps and prove
transactional persistence. A barrier merges first, then deletes all past
application state in the same durable transaction; otherwise merging can retain
the predecessor again. Timers need startup and active cleanup, and storage
journals/backups remain within the deletion audit. This integration is a prototype
gate, not a capability already established by an OpenMLS API call.
[Pruning API](https://github.com/openmls/openmls/blob/e0b21a70a55a9014b8de2772b36bf3d638adad51/openmls/src/group/mls_group/mod.rs#L1097-L1112).

The custody node still needs no content keys. Its confidentiality/metadata role
is unchanged; the increased exposure sits at readers retaining receive state.
After the receiver learns a barrier, retained old ciphertext cannot bypass it
as live work. Offline devices and concealed control changes retain the existing
cached-freshness limitation. The window is a maximum for local past-state use,
not a group-wide erasure deadline or a guarantee that every message remains
decryptable for that duration.

Therefore **14 days is currently a delivery policy, not a 14-day secrecy bound**.
Record expiry neither deletes offline devices' state nor rotates their leaf keys.
A stricter profile must choose how long inactive members remain cryptographically
eligible, separately from how long an obligation remains pending. Removing an
inactive device limits future access after the change, not access to old copies.
[MLS inactive-client considerations](https://www.rfc-editor.org/rfc/rfc9750.html#section-8.2.2).

## Recovery after compromise and the fixed owner

The owner-only Commit rule simplifies ordering but makes cryptographic recovery
depend on owner availability. Non-owner members must generate fresh leaf keys
and submit Update proposals; the owner commits them and peers process the result.
An owner updating only its own leaf does not necessarily evict an attacker's
access through another member's copied leaf secret. An epoch increment alone is
not evidence of healing.
[Member updates](https://book.openmls.tech/user_manual/updates.html),
[pathless-add tradeoff](https://book.openmls.tech/user_manual/add_members.html#adding-members-without-update).

Example: a laptop's state is copied, then cleaned. Ordinary group publication and
owner-only self-updates may remain readable through the copied laptop state.
Fresh laptop leaf material must be incorporated before claiming recovery from
that snapshot. If the owner is unreachable, publication may continue in cached
mode, but recovery can remain pending indefinitely. A stolen signing credential
also needs revocation/replacement with independently verified enrollment; an
encryption-key update does not automatically solve active impersonation.

If the owner itself is compromised, migrating to a new group must use fresh,
independently verified authority credentials. Reusing the compromised owner key
does not restore trust. The existing v0 recovery policy provides no automatic
safe authority replacement.

## Authentication and disclosure by readers

Our producer signatures and transferable receipts intentionally let recipients
show verifiable records to third parties. That provides provenance for telemetry,
replication and imports, but no deniability promise. LXMF's signed messages make
a similar tradeoff. Signatures establish evidence under a credential binding,
not a person's intent, and key compromise changes that interpretation. Group
members also learn device distinctions from the endpoint model.
[MLS authentication/deniability](https://www.rfc-editor.org/rfc/rfc9750.html#section-8.2.3).

Sesame-style sessions offer another way to isolate recipient devices, with
separate ratcheting sessions and per-device ciphertext costs. Their secrecy comes
from the selected session protocol; Sesame itself is session management. Its
optional resend mechanism also retains plaintext, so changing to pairwise sessions
does not eliminate the history/recovery tradeoff.
[Sesame recovery and compromise](https://signal.org/docs/specifications/sesame/#retry-requests-and-delivery-receipts).

## Decisions before the prototype

Every private profile needs an explicit visibility and key-lifecycle contract.
For the MLS candidate, use the following proposed minimums:

1. Require end-to-end object protection before private records enter blind custody; no fallback
   to public records or Link-only store-and-forward. Keep raw MLS group correlation
   as an explicit v0 limitation. Stronger metadata concealment needs another
   analyzed wrapper/key-distribution design.
2. Use encrypted-only member handshakes, private governance/bootstrap/tree transfer,
   minimal credentials and empty or explicitly public AAD. Put names, author
   evidence, targets, subscriptions and logical record digests inside encryption.
3. Track cryptographic update status separately from membership freshness. Use
   explicit update/commit and inactive-member policies; cached operation still
   cannot advertise bounded post-compromise recovery when the owner is unavailable.
4. Specify key retention separately from content, receipts and the 14-day obligation:
   skipped keys, past epochs, KeyPackage secrets, retries, journals and backups.
   Make application history retention visible as an intentional exposure.
5. Audit direct repair/control framing against Resource proof disclosure; transfer
   MLS ciphertext where possible and include hidden unpredictable framing for
   sensitive plaintext segments. Do not count padding or opaque handles as anonymity.
6. Preserve the distinction between original-reader live repair and newly granted
   history. Require independent credential recovery after compromise and retain
   the explicit no-deniability and no-traffic-anonymity limits.

## Recommended initial policies

These are proposed defaults for the optional MLS prototype, not adopted
requirements for all profiles, MLS mandates or measured safe limits.
Put the security choices in the authenticated group policy;
applications choose a supported profile, and endpoints report incompatible limits
instead of silently weakening it. Local history remains an explicit application
choice; a group cannot force a reader to erase its copies.

| Policy | Suggested starting point | Main tradeoff |
| --- | --- | --- |
| Member key refresh | Target a fresh leaf update every **24 hours while active**, incorporated at the next owner contact. Batch available updates briefly, up to **60 seconds** when contact permits; commit sooner if contact is ending. Removal and compromise response take priority. | More frequent recovery opportunities, but more control traffic and delayed-content repair. No per-message rekey or hard daily group-Commit cap. |
| Stale member | Warn after **7 days without an accepted fresh leaf update**. At **14 days**, the owner's next control decision incorporates a ready fresh update or removes the still-stale ordinary member. | Keeps abandoned devices from retaining future access indefinitely when control is available. Removal only takes effect through a committed transition adopted by peers. |
| Keys, retries and history | Delete consumed message keys through the durable crypto transaction. Trial **48 hours of bounded old-epoch receive state after routine closure**, only within the current authorization era; purge at barriers. Compare zero grace and a 14-day offline profile. Keep recoverable pending content until all required delivery obligations resolve or its fixed deadline expires, initially **at most 14 days**. No additional library archive, automatic new-device history or ordinary backup of live MLS state. | Enables delayed delivery from blind custody without author contact while keys survive. Extends unread/skipped-content exposure; expired keys still require an authorized repair source. Applications can explicitly retain history longer. |
| Metadata and traffic | Accept group/epoch correlation at the custodian in v0. Encrypt member control/bootstrap, use empty AAD and avoid private announce data. **No extra padding or cover traffic** in the bandwidth baseline; evaluate optional **256-byte padding buckets**. | Saves scarce airtime while exposing size, timing, contacts and ciphertext equality. Padding only reduces size precision. |

**Refresh scheduling and outages.** Each member needs its own fresh contribution;
the owner's routine self-update is insufficient for other copied member state.
Queue a local refresh intent while the owner is unavailable. Create an epoch-bound
MLS Update when an owner contact can incorporate it, rather than broadcasting it
early. A member with valid pending proposals cannot keep publishing application
data in that epoch: with owner-only commits it waits for the owner's transition.
Cached publication remains possible only when the MLS state permits it.
[MLS update scheduling and Commit rules](https://www.rfc-editor.org/rfc/rfc9420.html#section-12.4).

The age measurement is time since an accepted update, not proof of when its secret
was generated. Presence, receipts and publication do not reset it. A removed
device returns through authorized enrollment with a fresh incarnation; it does not
silently resume the old endpoint's obligations. The sole owner cannot be removed
by this timer: unavailable owner updates/removals remain overdue, and permanent
owner loss needs the already specified migration. These timers are independent of
each publication's delivery deadline.

**Deletion scope.** Past-state expiry or barrier purge does not mean deleting
current MLS state or the encrypted control chain needed for catch-up. Trial
same-epoch limits are **128 out-of-order generations per sender** and **1,024
maximum forward derivations**. They bound retained reordering state and work,
not elapsed time, total group memory or an attacker's derivations. Bound total
past epochs and bytes as well; high-rate telemetry needs measured bounds of its
own. An unsupported gap triggers bounded repair rather than unbounded allocation.
These are proposed experiment values, not the current adapter configuration.
Use the [MLS adapter limits](../mls-rs/README.md#what-is-bounded) for implemented
count-based quotas; trusted-clock grace and time-based retirement remain unwired. An offline current epoch may remain recoverable much longer.
[OpenMLS group configuration](https://book.openmls.tech/user_manual/group_config.html).

Even an unexpired retained epoch can lose decryptability through reordering: a
large accepted generation jump can discard earlier skipped keys. Receipts and
other group application traffic also advance generations. Protected inventory
ordering may help, but an opaque custodian cannot simply sort by the encrypted
sender/generation fields. Test selective subscriptions and reordered fetches;
longer time retention alone does not solve those gaps.

A custody acknowledgment does not resolve recipient obligations. Exact ciphertext
retries need no old plaintext key, but a new envelope needs recoverable content.
Early capacity eviction remains possible under best-effort storage and must report
loss or reduced repair capability. Application archives, encrypted-at-rest content
with available keys, and optional separately encrypted history backups all extend
content exposure. Ordinary backup restore must not resume live MLS/owner state;
apply the existing rejoin or owner-loss rules. Journals and physical storage need
their own deletion qualification; deleting a row is insufficient.

**The main experiment.** Hold daily member-refresh targets constant first and
compare zero, 48-hour and 14-day delayed-delivery grace with an offline recipient
and unavailable author. Snapshot receiver state before/after consuming messages,
routine rekeys, cutoff expiry and barriers. Measure ciphertext-only recovery,
retained secret bytes, control/repair traffic and cap-driven failures. Include
remove/re-add, role revoke/restore, compromise recovery with unchanged roster,
forged fresh timestamps, clock rollback, hidden barriers and crashes during purge.
Then vary refresh cadence if control costs dominate. A 14-day ciphertext outbox
alone still does not supply 14-day recoverability.
[OpenMLS delayed-message tradeoff](https://book.openmls.tech/forward_secrecy.html).

The default accepts availability while maintenance is overdue where protocol state
allows. A future stricter profile can stop new publication when an allowed age is
exceeded, checking **every current cryptographic reader**, not only the sender or
owner. This still cannot erase offline copies or substitute for ordered admission
when strict revocation freshness is required. Neither the 24-hour target nor the
14-day threshold establishes a global secrecy or compromise-recovery deadline.

The security experiment must inspect bytes at observers, custodians and members
during publication, enrollment, removal, repair and history import. Snapshot keys
before/after deletion and before/after each member's update; keep an offline member
and the owner unavailable in separate cases. Inspect persisted journals/backups,
test method downgrade refusal, and verify transferable receipts with a non-member
verifier. These tests precede any stronger confidentiality or erasure claims.

## Evidence scope

Source pins: Python Reticulum 1.5.5 `e40191b3d193b46b7f2d8a44424a594cd758839b`,
Python LXMF 1.2.0 `c3ff2d6dc2f256daab896dadc044dd5a913ecbb7`, PRNS
`4a2d22d4a815789031d62a1a7e3dfe17cc20cea8`; MLS RFC 9420, architecture RFC 9750,
and current OpenMLS documentation, with delayed-state implementation inspected at
OpenMLS `e0b21a70a55a9014b8de2772b36bf3d638adad51`. Those OpenMLS observations are historical comparisons, not the selected backend.
The [mls-rs source audit](mls-source-audit.md) and [adapter](../mls-rs/README.md)
document the implementation used here; host or library-build evidence does not
qualify a production backend.
These are source-backed protocol conclusions and design
requirements, not a cryptographic audit, proof of secure erasure or runtime
qualification of the proposed implementation.
