# Group policy and recovery design

This design expands the [architecture](design.md) with policy choices and worked
failure cases. The
[initial behavioral contract](v0.md) selects a small subset;
the matrices here describe the wider design space, not features all supported
by v0. This document does not freeze a wire format or establish implementation
guarantees; consult each component's README for its supported contract.
See the [encryption and privacy analysis](../../security/docs/security.md) for
who can read each layer, forward-secrecy conditions and the LXMF comparison.

## Configuration model

Configure **membership freshness, approval authority, content protection, storage
assurance and retention separately**. Publish a small set of supported bundles
rather than promise every combination. Group policy establishes minimums;
individual operations may request stronger supported guarantees. Authenticate
the policy identifier/version and changes under the preceding authority rules.
Unsupported combinations fail explicitly; there is no silent security downgrade.

All profiles retain authenticated control, replay/rollback checks, bounded work
and truthful outcomes. Low durability does not mean weak authentication, and long
retention does not imply strong confidentiality or replication.

## Membership freshness

Every participant need not be online or caught up before publication. The sender
must satisfy the selected freshness rule; receivers acquire missing control state
before accepting what depends on it. A signed snapshot proves an authorized
state, not that no newer state exists elsewhere during a partition.

| Mode | Promise | Availability and cost |
| --- | --- | --- |
| Cached membership | Use the latest authenticated state known locally; recipients repair or reject stale attempts according to policy. | Disconnected sends work. An unknown removal may still decrypt the content; repair adds traffic later. |
| Recently checked | Obtain an authoritative observation within an allowed age before sending. | Adds contacts and clock/expiry assumptions. Bounds the age of an observation, not changes since that observation. |
| Ordered admission | Membership changes and content admission share one authoritative order; a superseded membership/epoch cannot admit new work. | Requires an available gate with consistent state/failover for each admission. Recipients may remain offline. |

Checking membership and then sending separately has a race. Ordered admission
must atomically validate the relevant head, publication and encryption context
against the same history that commits removals. Multiple independent gate caches
do not provide this guarantee. A lease is stronger than an age check only if its
issuer enforces the promised authority semantics throughout the lease.

The strict guarantee is **no new stale-state admission after an ordered removal**.
It does not prohibit arrival of previously admitted ciphertext after removal,
retract old plaintext/keys, or prevent a malicious authorized member sharing data.
The encryption/key-distribution profile must exclude removed recipients too;
merely routing old-key ciphertext through a gate is insufficient. Compliant
senders withhold distributable content until admission; a signed gate response
alone cannot undo an earlier disclosure.

A gate can check an authenticated current head without knowing payload plaintext.
Validating hidden membership rights additionally requires suitable verification
evidence or an authorized validator. A plaintext-blind relay is not automatically
such a validator.

Distinguish a **new admission attempt** from evidenced delivery/recovery of
previously admitted work. Ordered admission evidence can establish that
distinction; a sender-supplied creation timestamp cannot. The initial cached
MLS candidate permits bounded delayed delivery across routine rekeys within the
same authorization era, as described below. Crossing an authorization/security barrier
still requires fresh original-author reauthorization or another explicitly
supported operation. A removed author's honestly delayed record can therefore
be lost. Retained local acceptance permits that receiver's exact historical
restoration; explicit authorized history import is a separate operation.

## Delayed delivery across routine rekeys

The epoch-specific rules below describe the preferred MLS private profile; authorization
eras and barriers apply to every profile. No production private implementation is
qualified; see the [MLS embedded plan](embedded.md).

Separate an **MLS epoch**, which selects cryptographic state, from an
**authorization era**, which identifies an uninterrupted period of the same
permissions and security policy. The era refers to genesis or the most recent
authenticated barrier in the control chain. It is not a hash of the current
roster: removing and later re-adding the same participants cannot restore an old
era. Both references remain bound to the same owner-ordered history.

A routine encryption-key update can preserve the era. An add/remove, endpoint
replacement, role/grant change, credential/signing-key change, policy change or
declared compromise recovery creates a barrier. The last case matters even when
the roster is unchanged. Receivers verify the transition's actual effects against
a narrow routine-update allowlist; an owner's label alone is insufficient.
Unknown transition types receive no routine exception.
Every MLS-profile barrier also starts a new MLS epoch, even when only roles
or application policy changed; one epoch cannot straddle eras. Use an approved
fresh-key transition, with the affected member's replacement/removal and credential
remediation where recovery requires it. A new era label alone does not heal keys.

This preserves two existing removal properties: updated senders exclude removed
readers from newly protected publications, and a receiver that has adopted a
barrier refuses previously unseen prior-era envelopes as live work. It cannot retract old
keys/plaintext or reveal an undiscovered removal during a partition. A removed
member can still decrypt delayed old ciphertext for which it retained keys.
No grace period makes such ciphertext secret from its original readers.
Other profiles implement their own required key changes. A generic barrier does
not delete a recipient key also used by unrelated groups; it invalidates
prior-era live admission even if the old ciphertext remains decryptable.

### Options and tradeoffs

| Option | Delivery without the original author online | Secrecy and removal cost |
| --- | --- | --- |
| Zero past application epochs plus existing repair | Only if the original envelope is still decryptable; otherwise needs an authorized recoverable-content source. | Least retained past-epoch key state. Source plaintext/retry history still extends exposure. Routine rotation causes repair traffic. |
| Bounded past-epoch receive state within one era | Yes, when the recipient retained the required state and custody/control/expiry limits permit. | Recommended first experiment. More unread/skipped content remains decryptable after receiver compromise; barriers terminate this exception. |
| Fetch old data before advancing keys | Sometimes, when a useful backlog is reachable. | Useful optimization, not a correctness rule. A relay cannot prove every publication arrived; waiting indefinitely retains old state and delays recovery. Never defer a known barrier to drain data. |
| Check only whether the old author is currently authorized | Often, with retained old keys. | Too weak as the baseline: misses revoke/restore, remove/re-add and compromise-recovery boundaries. Same current roster does not establish uninterrupted authority. |
| Owner admission certificates or authenticated checkpoints | Evidence can survive author removal; decryption still needs retained keys or another authorized source. | Orders specific record digests before barriers and defeats backdating. Adds an owner-dependent admission/confirmation path. A bare timestamp or custody receipt is insufficient. Later profile. |
| Established per-device ratcheting sessions | Yes, for independently queued recipient ciphertext within each session's limits. | Isolates receiver retention and avoids group rekeys invalidating every session. Costs per-recipient envelopes/state and still needs group authorization barriers and stale-directory handling. Sesame is session management, not the cipher itself. |
| Independent object keys with recipient/key wrappers | Yes if the recipient can recover its object key. | Helps immutable large objects and selective access. Retained wrappers, object keys and their unlocking keys extend exposure; distributing new keys must exclude removed readers. A long-lived shared data key defeats recovery from its compromise. Additional protocol work. |
| Authorized archive or content repair service | Yes if that service is available and authorized. | Trades source availability for retained recoverable content and, for a reading service, additional trust. A blind ciphertext store alone supplies neither decryption nor admission evidence. |

The bounded-epoch option uses MLS's existing delayed-message machinery, with our
authorization checks above it. It requires no Reticulum core changes. Other
options can also sit above ordinary Links/Resources; RFed's inventory/fetch
patterns help retrieve bytes but do not resolve keys or permissions.
[MLS delayed messages](https://www.rfc-editor.org/rfc/rfc9420.html#section-15.3),
[Sesame](https://signal.org/docs/specifications/sesame/).

### Recommended experiment

For the MLS candidate, use bounded receive-only past-epoch state within the
current era, with a trial
**48-hour grace after each routine epoch closes**. Compare zero grace and a
**14-day offline profile**. Forty-eight hours is a starting tradeoff for brief
interruptions; a 14-day delivery deadline does not imply 14-day decryption support.
Count, byte and sender-generation caps can shorten either window. Set those caps
from the 2/8/16-endpoint workloads before promising an interoperable profile.
Retained state grows with epochs, member ratchets and skipped generations, plus
historical credentials/context; it is not just one small key per epoch.

The owner-authenticated transition closing an epoch supplies its fixed closure
time and late-acceptance cutoff under the preceding policy. Routine cutoffs must
be ordered and no later than closure plus the configured grace. Validate closure
time itself against the trusted-clock/future-skew rules; a future-dated closure
must not stretch grace. A receiver checks the cutoff and the record's original
deadline against its trusted clock interval.
Neither the writer's timestamp nor first fetch, retries or restart can extend the
cutoff. Clock uncertainty yields `TimeUncertain`. A still-current epoch without
an observed closing transition has no such closure bound; cached mode provides
no global maximum key age.

Process available authenticated control before releasing new application records.
An old envelope must belong to a verified ancestor epoch in the current era,
within all retention/cutoff bounds, and pass its original and current author,
issuer, target, signature, deadline and replay checks. Recheck eligibility in the
same durable transaction as acceptance. This exception applies only to application
data, never old proposals/Commits or automatic execution of delayed commands.
Commands still need application-specific expiry and execution authorization.

At a barrier, atomically merge approved control/MLS state, disable prior-era live
admission and purge prior-era application secrets. Purge after the merge because
the merge can insert the previous epoch into past state. Preserve current state
and necessary authenticated control evidence; never restore a whole old MLS
snapshot to decrypt a delayed object. Missing keys or expired grace use the
existing repair/history paths, without changing original obligations.

Retaining old receive state does not necessarily retain every consumed key. But
it preserves whatever unread/skipped generations remain derivable. It can also
leave an old-epoch injection path for an attacker possessing the old encryption
state and the necessary signing keys. A routine update can protect new-epoch
traffic while that acceptance path remains open; declared recovery must close it
with a barrier. [MLS authentication](https://www.rfc-editor.org/rfc/rfc9420.html#section-16.5).

### Cases the model must distinguish

| Case | Required outcome |
| --- | --- |
| Epoch 10 record, routine rekeys to 11/12, unchanged era and valid cutoff | Accept from a blind custodian with retained receive keys, even if the author is offline. |
| Member removed and re-added, or publishing permission revoked and restored | Reject the old envelope after learning the barrier, even if the current roster/roles look identical. |
| Recovery rekey after suspected compromise, unchanged roster | Advance the era; reject old envelopes and purge their receive state. |
| New reader added | Barrier. No past-key transfer or implicit history. Repair must preserve the original audience or use authorized recipient-specific transfer. |
| Attacker with old keys invents a fresh timestamp/deadline | Cannot cross a barrier or extend the epoch cutoff. Within grace, necessary stolen signing keys remain an explicit risk. |
| Owner/custodian hides newer control | Cached freshness remains limited. No claim of global removal enforcement. |
| Key cap/expiry reached, but custody bytes still exist | Report repair required, not delivered or recoverable merely because bytes remain. |
| Fetches arrive with a large sender-generation gap | Apply bounded ratchet processing; report lost decryption coverage where skipped keys were discarded, even inside the time window. |
| Crash during barrier merge, key deletion or late-record receipt | Restore a coherent state; never release an old-era record against a committed barrier. |

Owner admission evidence is the route to accepting a removed author's proven
pre-removal record, if an application needs that later. Bind the exact digest,
author/incarnation, reader audience, deadline and admission order to the same
history as barriers. A checkpoint covers only its committed records; it does not
prove an asynchronous backlog complete. This is a distinct policy from v0's
conservative rejection of unseen prior-era work, and it does not recover erased
decryption keys.

## Membership authority and records

Approval answers who may authorize a change; sequencing decides which authorized
successor becomes effective. Configure these independently.

| Approval rule | Benefit | Cost or failure mode |
| --- | --- | --- |
| Any current member | Collaborative groups can manage themselves. | One compromised member can authorize changes; conflicts still need ordering. |
| Any designated administrator | Simple delegation and restricted authority. | One compromised administrator suffices. |
| k of n administrators | Distributes authority and tolerates some absent approvers. | Collect matching approvals; define administrator-set transitions and recovery. |
| All current administrators | Everyone explicitly approves. | One unavailable or hostile administrator can block changes, including their own removal. |

The initial contract uses one fixed owner for approval and ordering; the broader
administrator rules remain extensions. The authority need not be a data
custodian. Threshold signatures/approvals alone do
not supply consensus, prevent conflicting successors, or authorize automatic
failover. Even unanimous approvers need rules against approving conflicting
changes from the same predecessor.

An authenticated transition binds the group, predecessor hash, policy/admin-set
version, exact membership/role changes, approvals and associated security
transition. Counts/revision numbers aid presentation; hashes and verified
ancestry establish relationships. Approvals over different base states cannot
be combined. Policy changes are authorized by the old policy, not themselves.

Conceptual lifecycle: **proposed → approval pending → commit pending → effective**,
with explicit conflict/recovery states. Effective means one accepted transition
aligns authorization and cryptographic access; distribution to every member can
follow later. Editing a roster alone does not revoke an old decryption key.
For MLS, bind application governance to its accepted Commit/epoch; do not create
an independently editable competing membership history. Membership heads and
cryptographic epochs are separate references because rekeying need not change
the roster. [MLS changes and message processing](https://www.rfc-editor.org/rfc/rfc9420.html#section-12.4).

Define a recovery authority at group creation if stable-address recovery is
required: delegated recovery credentials or an explicit recovery approval rule.
Without one, permanent authority loss can mean a new group/address. A restored
backup cannot claim authority merely by incrementing a counter. Competing recovery
branches require authenticated resolution; arbitrary failover must not silently
weaken an all-admin policy. Detailed recovery/fencing rules remain open and are
excluded from v0: permanent owner loss or unsafe rollback requires a new group.

## Efficient and private control distribution

Include authenticated membership/policy and encryption-context references with
each envelope. Keep the logical record immutable; refreshed routing, encryption
and admission evidence belong to a new authenticated envelope. A retry must not
launder an otherwise revoked author's operation into an authorized one.

Normally send a compact state hash/reference, not the full membership list.
Piggyback a small delta when useful; fetch missing predecessors or authorized
checkpoints with bounded pages and work. Full bundles can serve first contact or
self-contained transfers where that extra size is worthwhile. Send change hints
to known peers and reconcile on later contact; receiving every hint is not a
correctness requirement. Batch receipts/control traffic without treating an
unverified summary as evidence.

For private groups, encrypt membership records end to end. Non-member relays
store opaque objects indexed by scoped, opaque handles and apply independent
storage quotas/capabilities. They can forward verifiable updates without becoming
membership authorities. They may still observe object sizes, timing and access
relationships; opaque identifiers do not provide traffic-analysis anonymity.
Raw MLS also exposes group ID, epoch, content type and AAD to a custodian handling
its ciphertext. The initial MLS candidate accepts that correlation, keeps sensitive
fields out of AAD and requires encrypted member handshakes/private tree bootstrap.

Use the chosen protocol's key transition to exclude removed members. New members
need an authorized bootstrap they can decrypt, not access to erased historical
keys. Control repair must be possible through endpoint credentials or a recovery
channel without already possessing the missing epoch. Check entitlement under
the selected freshness rule before returning private rosters, history or keys;
refuse disclosure if its required freshness cannot be established. Removed senders
may get only a minimal rejection, not the new roster. Previously disclosed rosters
cannot be made secret again. [MLS handshake privacy](https://www.rfc-editor.org/rfc/rfc9420.html#section-6).
The removal transition itself can be visible to a member holding the preceding
epoch's keys; private correction responses do not conceal knowledge already
obtainable from that transition.

## Storage assurance and retention

Storage assurance describes a commitment; retention describes what remains useful.

| Assurance | Acceptance evidence | Capacity rule and limit |
| --- | --- | --- |
| Transient | Optional transport/processing response; no durable commitment. | Drop on deadline, congestion or restart. No offline recovery promise. |
| Best-effort durable | A named custodian commits the required content and obligation state. | Oldest-content-first eviction is permitted. Report known eviction/loss; later retrieval remains conditional. |
| Reserved custody, k | k distinct custodians commit a specified retention obligation. | Reserve capacity; transfer/release obligations or reject admission instead of silently evicting promised content. Failures can still destroy copies. |

Prefer independent failure domains for replicas. k storage acknowledgments mean
k commitments obtained, not consensus, permanent availability or delivery to all
recipients. An opaque custodian's acknowledgment does not establish decryption.

| Retention shape | What is retained | Release or recovery rule |
| --- | --- | --- |
| Deadline | Content useful until a stated deadline. | Expire; late live frames need not be replayed. |
| Bounded history | A duration/byte-limited history. | Best-effort eviction can create gaps; reserved commitments take precedence. |
| Latest state/checkpoint | Current state plus necessary dependencies/deltas. | Supersede only under application merge rules and outstanding obligations. |
| Archive | Specified records/content for an explicit term. | An archive may be best effort or reserved; long-term retrieval also requires usable keys and authority evidence. |

Keep delivery deadlines, retained history and cryptographic lifetimes distinct.
Specify how deadlines survive restarts and clock uncertainty; retries never
reset them. Enrolling a new device or requesting history creates explicit new
obligations rather than changing the original publication's target set.

Bind targets and subscription revisions at admission. Later changes create
authorized obligation revisions. Dashboard receipts cannot cancel archive
obligations; device removal yields `removed`, not `received`. Completion after
q of N receipts must say q of N, not all. Expired obligations and known unavailable
content remain visible outcomes.

Before acknowledging durable storage or durable recipient receipt, commit the
required content, obligations, cryptographic state and outgoing evidence
coherently. An ordered gate must preserve its authoritative decisions/order but
need not take payload custody. Transient payloads may still require persistent
cryptographic state or a defined fresh-session restart to prevent key reuse.
Declare whether the storage commitment covers process restart, power loss, and
which platform/failure assumptions apply; reject unsupported requested guarantees.

After a lost reply, retry/query using the same logical identity; deduplication
resumes unfinished work. Retain bounded control, receipt and deletion/checkpoint
evidence separately from payloads. Beyond the recovery boundary, require
checkpoint/history import or cryptographic rejoin; an application snapshot is
not automatically an MLS-state recovery mechanism.

## Content protection and application bundles

Require authenticated producers and control integrity in every profile.
Publication rights are role-specific; authorized outside producers need not be
members, and public reading need not require membership.

| Protection candidate | Suitable use | Cost and limitation |
| --- | --- | --- |
| Public authenticated content | Public measurements, announcements, release catalogs. | No end-to-end content secrecy; still pays authentication and control costs. |
| Recipient-encrypted objects | Small groups needing blind offline custody and explicitly accepting weaker compromise guarantees. | Per-recipient ciphertext or key wrappers; later static recipient-key compromise exposes recorded ciphertext for that key. No automatic post-compromise recovery. Producer, audience and recipient bindings remain authenticated. |
| Private device sessions | Independent readers and outside producers; small groups. | Per-device envelopes/session state and repair. Forward secrecy depends on the selected established protocol and key deletion. |
| Private MLS group | Repeated publication among group members. | Shared encrypted publication, but network fan-out remains; epoch/key-package state and recovery are required. |

Ordered admission can accompany a private profile when revocation freshness is
required; it is not a stronger cipher. Likewise, encryption does not imply
storage durability. Long-lived histories or plaintext retry caches extend what
a later compromise can reveal; old ciphertext without keys may be unrecoverable.
Optional history import should use a separately authorized archive transfer,
not indefinite retention of every live-session secret.
Delivery expiry is not a forward-secrecy deadline. Key age, skipped/past-epoch
retention, inactive-member policy and archive/backups require separate rules.
For owner-committed MLS, healing another member's compromised state requires that
member's fresh key material to be committed; owner self-updates alone are not
a bounded compromise-recovery guarantee.

Re-encryption or a new recipient key wrapper must not silently grant historical
content to new readers. An MLS retry under a newer epoch is one example.
Compare cryptographic audiences, not subscriptions/receipt targets;
use recipient-specific transfer when the current audience would be larger. The
initial contract uses authenticated direct Links for that repair/import and
requires a reachable authorized plaintext source when old keys are gone.

Sesame supplies a useful stale-device-list rejection/retry pattern and session
management; it assumes directory/mailbox services, which may be distributed.
MLS provides group encryption and an epoch protocol; neither selects our storage
policy. Their recovery constraints remain visible to callers.
[Sesame §§3.3, 5.1, 6.4](https://signal.org/docs/specifications/sesame/),
[MLS delivery and access control](https://www.rfc-editor.org/rfc/rfc9750.html#section-5.2).

| Candidate application bundle | Starting composition | Main tradeoff |
| --- | --- | --- |
| Everyday messaging | Cached membership; designated admins; private protection; one best-effort durable custodian; 14-day delivery window; optional history. | Works disconnected; stale-membership exposure and last-copy eviction remain possible. |
| Public sensor dashboard | Public authenticated content; transient/latest state or short durable history. | Small retained state; incomplete sample history. |
| Private telemetry archive | Outside-writer ingress; private protection; reserved k and explicit archive term. | More key/bootstrap work, storage and replicas; privacy/strict freshness can add an online dependency. |
| Sensitive operational group | Ordered admission; chosen admin approval rule; private protection; reserved custody if required. | Freshness at admission; authority unavailability blocks new publication. |
| Commands | Appropriate freshness/protection; execution deadline; designated executor and durable idempotency/result state. | Custody or receipt never implies execution or exactly-once physical effects. |
| Files and media | Durable manifests; selective chunk custody for files, transient deadlines for live frames. | Avoid forcing all subscribers to download every object or replay stale frames. |

Outside producers must not be made readers merely to publish. Compare independent
recipient envelopes with a member-mediated ingress that sees plaintext and
republishes it, preserving producer evidence. A plaintext-blind external ingress
for MLS application data is not supplied by MLS external proposals; its design
would need separate evaluation. [MLS external proposals](https://www.rfc-editor.org/rfc/rfc9420.html#section-12.1.8).

## Discovery and protocol outcomes

Use normal `SINGLE` service announces for protocol/version and compact capability
information. Reticulum announce data is signed but public, so private rosters,
group lists and per-group changes belong in encrypted control exchanges. A group
change should not require a network-wide announce. Public group discovery can
be an explicit alternative profile.
[Python announce construction](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Destination.py#L298-L304),
[unencrypted announce packing](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Packet.py#L193-L195).

Logical exchange: discover/cache endpoint → authenticate and negotiate profile →
compare authorized scope heads → fetch bounded dependencies → admit/store content
→ reconcile receipts. Hints accelerate this; repair must work when hints are lost.

Names below are conceptual outcomes, not assigned wire codes. Bind responses to
issuer/role, request, record, envelope attempt and applicable policy/control state.

| Outcome | Meaning and next action |
| --- | --- |
| `NeedIdentity`, `UnsupportedProfile` | Repair authentication or stop an incompatible exchange; never silently downgrade. |
| `NeedControl`, `MissingDependency` | Receiver lacks prerequisites. Fetch bounded authorized dependencies; this is not a group-wide rejection. |
| `StaleMembership`, `StaleEpoch` | Verify supplied/referenced successor evidence, update, then retry if permitted. A greater numeric revision alone is insufficient. |
| `MembershipConflict` | Incompatible authenticated histories need resolution; do not choose by arrival time. |
| `NotAuthorized` | Reject this operation under the cited authority; disclose no private correction to an unauthorized requester. |
| `Busy`, `CapacityRejected`, `RetentionUnavailable` | Back off or select another custodian/profile allowed by policy. No acceptance commitment was made by this result. |
| `AdmissionAccepted`, `Stored`, `Received` | Distinct gate, custodian and recipient evidence. Include the actual commitment/meaning; none automatically implies the next. |
| `AlreadyStored` | Bytes exist; return/query obligations and evidence instead of assuming delivery completed. |
| `Expired`, `ObjectUnavailable`, `CheckpointRequired` | Report this peer's known state and recover elsewhere or through an authorized bootstrap. Do not claim universal absence. |
| Timeout/disconnection | Outcome unknown. Query or idempotently retry; silence is not rejection. |

A blind relay can report its own storage outcomes or forward verifiable authority
evidence, not invent membership decisions. Even an authenticated peer's state may
lag; it does not prove global freshness. Rejecting one envelope cannot retract
another peer's valid earlier acceptance.

Python's request allow-list can reject before invoking the application handler,
yielding no typed response. Use an explicit bounded bootstrap/control handshake
if `NeedIdentity` is promised; apply group authorization inside that contract.
Before authorization return only small generic errors. Send corrections over the
established channel, cap retries/repair bytes, coalesce hints, and do not respond
to errors with errors.
[Python request handling](https://github.com/markqvist/Reticulum/blob/e40191b3d193b46b7f2d8a44424a594cd758839b/RNS/Link.py#L818-L855),
[RFed identity-error lesson](https://github.com/jrl290/RFed/blob/14716b9277924e2a44e6e56a3f094fdf02865358/rfed/src/destinations.rs#L475-L488).

## Worked cases

**Outside writer and removal.** A sensor knows membership 7; an opaque relay holds
encrypted control 8, which removes a reader. In cached mode the sensor can send
against 7; a current recipient returns an authorized correction, but exposure to
the removed reader cannot be undone. In ordered mode the gate rejects the new
attempt against 7 before release. The sensor fetches only its permitted ingress/
recipient information and submits a fresh envelope for the same logical record.
If required authority is unreachable it waits; other recipients need not be online.

**Lost reply and storage pressure.** A custodian commits a chat record and phone/
laptop obligations, then crashes before replying. Retrying resumes those
obligations. The phone receives; the laptop remains offline. Best-effort eviction
can leave laptop delivery unavailable unless another usable copy is found.
Reserved custody instead needs completed handoff/release or refusal of competing
admission. A transfer started but unacknowledged is not completed handoff.

**Different subscriptions.** Sample 42 supersedes sample 41 for a dashboard, but an
offline archive still has a reserved obligation for 41. Keep the archive copy.
Changing the dashboard subscription does not cancel it. A later archive requesting
history creates a new request for whatever is available; expiry does not turn
missing history into delivered history. A late producer's unsent sample is a new
admission evaluated against current profile rules, not proof of old acceptance.

**Administrator loss.** The sequencer disappears. Members can continue cached-mode
data publication only while their profile/crypto state permits it; membership
changes and ordered admissions wait. A preauthorized recovery rule can nominate
and fence a replacement. If a restored old sequencer also signs a successor,
receivers preserve evidence and enter conflict/recovery handling. Without an
agreed recovery rule there is no safe automatic winner.

## Costs and next decisions

Use measured encoded sizes, not only payload size. For P publications, repeating
a membership snapshot of size M costs P×M per receiver; a reference of size h
costs P×h plus the control objects actually fetched. Rekeying, onboarding and
repair still add work. Approval traffic grows with required approvers; all-admin
latency includes the slowest approver. Eager all-to-all receipt rebroadcast can
grow quadratically with member count, so reconcile/batch evidence selectively.

For fixed rate λ, retained duration T, content B and metadata h, one historical
copy is approximately λT(B+h), before indexes, journals and keys. k additional
full copies multiply that storage. Complete per-target receipt history adds a
term proportional to record count × target count. Latest-state savings require
legal compaction. Unicast fan-out still transmits content toward each target;
group encryption alone does not provide network multicast. Airtime additionally
depends on topology, headers, retries and duty limits.

The [behavioral contract](v0.md) specifies proposed authority/admission behavior,
recovery limits and clock assumptions. The current MLS implementation and
[embedded support guide](embedded.md) distinguish implemented mechanisms from
remaining transport, resource and security qualification. Alternative private
profiles are comparisons, not automatic fallbacks or parallel requirements.
Reserved custody, strict admission, threshold governance, attachment protection
and automatic authority recovery remain separate extensions.
