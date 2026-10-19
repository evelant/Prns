# Secure flash, reclamation and MLS key retirement

**Implemented behind `prns-groups-store/secure`, with host tests and a separate
opt-in E290 fixture; not security-qualified.** The
[embedded guide](../../core/docs/embedded.md#e290-storage-fixture) distinguishes
local software-reset recovery from electrical power loss and physical secrecy.
The fixture's wrapping key is deliberately plaintext.
The backend changes no Reticulum wire behavior or MLS cryptography. The core
stays allocation-free; storage remains bounded `no_std + alloc` with platform
flash, entropy and key-provider interfaces.

## Decision and security boundary

Use immutable encrypted object versions, a small encrypted catalog of live
object keys, and independently erasable capsules holding each catalog's key.
Retire obsolete capsules promptly; reclaim their now-unreadable data later.
This separates key retirement from bulk space reclamation.

| Claim | Implemented mechanism and qualification limit |
| --- | --- |
| Crash recovery | Recover one complete published state and its exact outputs; an interrupted retirement resumes without re-running MLS. Corrupt/ambiguous publication may require explicit repair. |
| At-rest confidentiality/integrity | Standard AEAD protects blobs, catalogs and key capsules, rooted in a protected platform key. A software test key stored alongside data provides no physical protection. |
| Local key retirement | After verified capsule erasure and RAM cleanup, obsolete object keys are absent from the device's currently retained store. This assumes qualified NOR erase behavior and no other key copies. |
| Historical flash capture or full-device rollback | Not solved by an eFuse wrapping key, AEAD or a counter stored in the same flash. Requires a stronger external/hardware mechanism or an explicit exclusion. |
| Content deletion | Separate from MLS key deletion: retained inboxes, history and repair plaintext remain readable under their own retention policy. |

An attacker who saved an old wrapped capsule and later gains its wrapping key
or a usable unwrap oracle can still recover that old catalog. Even hardware
read-protection of the device root does not revoke an archived wrapper. Never
describe this design as protection against every physical attack, earlier
endpoint compromise, forensic remanence or a persistently compromised member.

## Storage structure

1. **Encrypted blobs.** Each immutable version gets a fresh random data key.
   Replacing a group snapshot or an evolved past-epoch record replaces its
   key, even though the logical record name is unchanged. The initial format
   encrypts each bounded value as one AEAD record and binds store incarnation,
   logical key/version, lengths and format. Logical namespaces supply the record
   type. Do not bind ciphertext to its physical
   address: compaction must be able to relocate identical ciphertext.
2. **Live catalog.** A bounded authenticated map holds current object locations,
   hashes and data keys, including the live application state and replay fences.
   Encrypt each catalog publication under a fresh random catalog key. Never
   include previous catalog keys or obsolete version keys. Unchanged live objects
   retain their data keys; old serialized copies of the catalog become harmless
   only when every capsule unlocking them has been erased.
3. **Key capsules.** Store the catalog key in an AEAD wrapper under a
   domain-separated, device-bound wrapping key. The publication descriptor binds
   catalog digest, physical generation, parent, store incarnation and configuration;
   the authenticated catalog supplies logical revision and receipt floor. Each capsule occupies its own
   erase sector; never put successive erasable keys together in an append log.
   The serialized ring derives the predecessor generation and slot from the
   authenticated current generation; it never runs detached cleanup jobs using
   a remembered address. At least three slots keep the predecessor distinct
   from the immediate unpublished successor after slot reuse.
4. **Capsule ring and data arenas.** Rotate capsule placement through a bounded
   sector pool to distribute wear. Normally only the active and one candidate
   capsule may exist. Use two data arenas for the first collector, with enough
   reserved space for the entire live set plus the largest admitted transaction.
   Ciphertext catalogs can reside in those arenas with other blobs.

Catalog and data keys must be independent random values, not
`HMAC(device_root, public_generation)` or a chain retaining ancestors that can
rederive retired keys. A root-derived wrapping key is acceptable under the
stated boundary because random wrapped keys are retired by destroying wrappers.
Capsule AEAD nonces must remain unique across failed attempts and reboots:
logical revision alone is unsafe. The initial format uses XChaCha20-Poly1305
with independent random 24-byte nonces, 32-byte keys and 16-byte tags. Fresh
entropy is mandatory on every attempt, including after reboot; random nonce
collision is probabilistic, not a counter-based uniqueness proof.
Never resume encryption with a previously used key/nonce pair.

Encrypted catalogs conceal record names and keys, but outer sizes, generation
changes and storage access patterns remain observable. This is not traffic or
storage-pattern obfuscation.

## Publication and retirement ordering

Serialize mutations and maintenance through the coordinator; resolve uncertain
work first. No external dispatch or application effects occur during staging.

1. Check capacity for the complete new live set, a candidate capsule and recovery
   reserve before writing changed secret state. The coordinator checks receipt
   capacity before MLS begins; some catalog/live-set capacity failures are only
   known at commit, when staged MLS work is discarded and durable state preserved.
2. Write new encrypted versions and the candidate catalog. Read back and verify
   their authenticated contents, hashes and references. Include the advanced
   ratchet, exact outbox/inbox result, key-package changes, barriers and operation
   receipt in this one logical transaction.
3. Write and verify the candidate capsule, then publish its separately programmed
   final marker. A complete authenticated publication establishes **Committed**.
   Lost success is resolved by returning the same stored result, not encrypting
   or applying the operation again.
4. Using the new root's authenticated retirement obligations, erase and verify
   every obsolete capsule sector, including abandoned candidates that could
   unlock older secret state. Zeroize superseded in-memory catalogs, keys and
   decode/staging buffers. Only then return a successful commit/recovery result.
   The initial backend enforces strict retirement implicitly; it does not expose
   a separately dispatchable pre-retirement `Committed` result.
5. Reclaim unreferenced ciphertext opportunistically. Copy live ciphertext to
   the spare arena, verify it, publish an updated catalog/location map, retire
   its predecessor capsule, then erase the old arena. GC advances the physical
   storage generation, not the application's logical revision or operation IDs.

The initial private profile should require retirement before returning a
dispatchable result, delivering plaintext to effectful application code, or
acknowledging successful consumption. Keep the existing admission callback pure.
Recovery/retry must enforce the same gate; `Committed` alone cannot bypass it.
Do not acknowledge a peer and then discover retirement could not finish.

| Interruption | Recovery behavior |
| --- | --- |
| Before publication, old root intact | Use the old state only when non-publication is definite; securely clean candidate capsules before further strict work. |
| After publication, before old capsule erasure | Recover the new state, complete its retirement obligations, then release the already committed output. |
| During obsolete capsule erasure | New state remains authoritative; resume only erases named by its authenticated obligations. An invalid old wrapper alone is not proof that all key bytes are gone. |
| During ciphertext GC | Old locations remain live until the new map is published; after publication, finish cleanup using the new map. |
| Ambiguous newer publication or corrupted selected root | Fail closed. Never restore an older ratchet snapshot as a convenience fallback. |

There is necessarily a transition during which old and new recovery material
coexist. Power loss can extend it until cleanup resumes; there is no offline
wall-clock erasure guarantee while a board is unpowered. A persisted
`retired=true` flag is not erasure evidence. Boot must verify the selected state
and the named cleanup, and treat unexplained non-erased capsule sectors as an
error rather than guessing which history to restore. The capsule format and
torn-publication classifier below are exercised by host fault tests; hardware
failure behavior still needs qualification.

After retirement, sacrifice recovery from loss of the sole authoritative state
rather than silently resurrecting old secrets. Repair means a new authorized
endpoint incarnation or explicit rejoin, with group and application reconciliation.

## MLS and application retention

MLS requires deletion of consumed secrets and permits bounded retention of
unconsumed material for delayed messages. Our local retirement completes that
deletion at the durable boundary; it does not erase material other members keep.
[RFC 9420 sections 9.2 and 16.8](https://www.rfc-editor.org/rfc/rfc9420.html#section-9.2).

| Material | Catalog rule |
| --- | --- |
| Current group state and evolved retained-epoch records | Keep only the latest serialized version. Every send/receive replacement retires its predecessor's storage key. |
| Skipped/unconsumed receive keys | Retain only under the adapter's bounds and authorization-era policy; retirement does not prematurely remove supported delayed delivery. |
| Membership/authorization barrier | Publish the merged state, old-epoch purge and admission fence together, then require retirement before reporting the barrier complete. |
| KeyPackage private state and pending update/commit secrets | Retire consumed/superseded versions with the transaction that resolves them; do not leave abandoned candidates in recovery copies. |
| Exact outbox and control bytes | Retain as delivery/catch-up requires. Retransmission needs those bytes, not the old MLS sending key. |
| Inbox plaintext, application history and repair copies | Explicit independent policy and keys. Acknowledgement may remove an inbox, but application copies can remain. Never call network-key retirement deletion of that content. |
| Operation receipts, removal tombstones and replay/admission floors | Preserve until replaced by an equivalent non-secret fence; space pressure cannot silently discard safety metadata. |

The implementation currently caps old epochs by count; the proposed 48-hour
grace still lacks a qualified clock. Compaction must not invent an expiry time or
relax a removal barrier. The adapter/application chooses semantic deletion;
storage preserves the resulting live set.

Offer delayed/batched retirement only as a separately declared weaker policy
after measuring wear and latency. It leaves consumed secrets recoverable during
the delay, so it cannot claim immediate physical deletion. Start with retirement
per crypto-changing transaction; do not batch merely because an app chose long
content retention. Emergency barriers must drain any retirement backlog.

## Receipt reclamation and API changes

After forgetting an opaque operation ID, returning
`NotCommitted` can re-enable the old operation. Bounded storage cannot remember
arbitrarily many unrelated IDs forever with exact deduplication.

The secure backend uses a local store-incarnation + monotonic operation sequence, a bounded
result/recovery window and a durable retired-prefix fence. Retired tickets return
a terminal **Retired/OutsideRecoveryWindow**, never `NotCommitted`. They cannot
execute again, although their exact result may no longer be retrievable. Keep
unacknowledged/uncertain operations in the window; no silent floor advance past
an unresolved obligation. The initial version accepts consecutive sequences only;
there are no burned tickets or gaps. Aborted, definitely uncommitted attempts can
retry the next ticket.
These are local transaction tickets, not replacements for application record IDs
or membership-scoped network deduplication. Reject closed or foreign store
incarnations instead of treating their tickets as fresh namespaces. The legacy
journal still preserves all receipts; there is no implicit format migration.

`FinalizeThrough(ticket)` is an explicit trusted attestation that results through
the ticket are durably handled and no longer need commit-result recovery. It
removes receipt details, not application outputs or safety tombstones. An
application acknowledgement is separate. `Compact` and `EraseRetired` run through
the coordinator's exclusive maintenance contract; errors fence ordinary access
until `recover_maintenance` succeeds without changing the logical revision.

`SecureLimits` bounds live objects, encoded catalog bytes, receipt count, key/value
sizes, changes and spare capacity. Geometry must leave at least one padded maximum
catalog publication in reserve, so receipt finalization can make progress at a
full window. Staging and old/new catalogs still coexist temporarily: these are
logical bounds, not a measured total heap guarantee. Do not drop
crypto state or replay fences under the application's oldest-content-first rule.

## Initial byte format and root selection

This experimental `PRNSSK01` format has no in-place migration. The partition has
at least three erase-isolated capsule sectors followed by two equal, erase-aligned
arenas. Reads/programming use 256-byte blocks; driver read/write sizes must divide
256 and erase size must be a multiple of 256 and at least 512 bytes. Numeric
fields in persistent records are little-endian. Ticket IDs encode their sequence
big-endian after the 24-byte incarnation.

| Capsule bytes | Contents |
| --- | --- |
| 0–63 | Magic, incarnation, physical generation, random wrapping nonce; authenticated as AEAD associated data. |
| 64–211 | Encrypted 132-byte descriptor and 16-byte tag: configuration hash, predecessor slot, catalog address/length/nonce/digest, catalog key. |
| 212–255 | Erased padding. |
| Final 256 bytes of sector | Separately programmed publication marker, all `0x3c`. |

The encrypted `PRNSCT01` catalog holds revision, retired receipt prefix, active
arena, live object descriptors and retained receipt sequences/revisions. Object
descriptors contain logical key, random version ID, random DEK/nonce, address,
plaintext length and ciphertext SHA-256. SHA-256 checks are authenticated by the
catalog/capsule; an unauthenticated checksum alone would not suffice. AEAD domains
separate object, catalog and capsule records. Geometry and every storage limit
are bound into the configuration hash. Applications supply a domain-separated
wrapping key and must not store it in the partition.

Mount selects the highest authenticated published generation, authenticates its
catalog and every live value, then verifies retirement before returning. The
only other occupied slots allowed are its immediate predecessor and a definitely
unpublished immediate successor. A partially erased predecessor is safe to erase
again because the selected root authenticates that obligation. An ambiguous or
corrupt successor is an error; it cannot authorize fallback to the old ratchet.
A fully erased marker indicates non-publication only under the exclusive-owner
fault model: erasing a previously published marker independently, hostile flash
restoration, and arbitrary corruption that recreates an older valid image are
outside that guarantee. Spare-ciphertext erasure is idempotent and uses the arena
opposite the freshly authenticated current catalog.

## E290 protection and operating costs

Implement generic AEAD storage over raw NOR with an injected key provider.
The E290 provider can use the S3 HMAC peripheral with a dedicated protected
eFuse key to derive a wrapping key; `esp-hal` already exposes this peripheral.
The eFuse root remains inaccessible, while upstream HMAC output is usable by
software. Espressif also uses HMAC-derived keys for NVS encryption, but its
append/invalidate lifecycle is not our secure-retirement mechanism.
[HMAC](https://docs.espressif.com/projects/esp-idf/en/v5.5.4/esp32s3/api-reference/peripherals/hmac.html),
[NVS encryption](https://docs.espressif.com/projects/esp-idf/en/v5.5.4/esp32s3/api-reference/storage/nvs_encryption.html),
[NVS storage](https://docs.espressif.com/projects/esp-idf/en/v5.5.4/esp32s3/api-reference/storage/nvs_flash.html).

Production protection should also qualify secure boot, release-mode flash/PSRAM
encryption and debug/download restrictions. Hardware XTS does not replace the
application's authentication or retirement rules. The current raw ROM flash
driver does not transparently handle encrypted partitions; simply setting a
partition flag is insufficient. If the MLS partition is additionally XTS-encrypted,
qualify encrypted writes, decrypted reads, encrypted partition-table validation
and raw erased-sector detection separately. Raw erased `0xff` bytes do not read
as `0xff` through XTS decryption. Alternatively, keep the software-AEAD MLS
partition raw while protecting code, other secrets and PSRAM in the board profile.
[Flash protection](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/security/flash-encryption.html).

The stock design has no qualified mutable freshness anchor outside rewritable
flash. Firmware anti-rollback's 16-bit eFuse secure-version field is unsuitable
for per-message state. Strong hostile-rollback protection needs a protected
monotonic head plus recovery protocol, or an online witness recording every
released security-relevant head. A later occasional check leaves an offline gap.
Protecting archived capsules against later unwrap access additionally needs key
retirement enforced outside compromised firmware; a counter check in ordinary
software is insufficient.
[Firmware anti-rollback](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/system/ota.html#anti-rollback).

Per-update writes are changed blobs + bounded catalog + capsule publication/erase,
not rewriting the whole journal. The current synchronous recovery path also
authenticates all live values and scans the active arena; commit and operation
recovery invoke it. Compaction traverses a whole arena; the E290 fixture yields
between driver calls and individual sector erases. These costs are bounded by
the configured partition, but the fixture measurements do not establish radio
deadline guarantees. Capsule sectors are the wear hotspot: estimate
lifetime from the fitted flash's specified erase endurance, pool size and actual
retirement rate, including retries. Do not assume an endurance rating from the
board model. Measure worst erase stalls and radio deadlines, not just averages;
pre-erase spare capacity and yield between bounded operations. The current driver
parks the other core during ROM calls. Reserve internal secret/bounce buffers,
zeroize them, and audit transient MLS/PSRAM copies and crash logging.

## Validation and remaining gates

Host coverage lives in the [secure backend tests](../src/secure/tests.rs),
[independent API checks](../tests/secure_boundary.rs) and
[real MLS recovery tests](../../security/mls-rs/tests/secure_recovery.rs).
These exercise real AEAD with test-only wrapping keys and simulated NOR, not
qualified hardware or a cryptographic audit. Surviving-key reachability is a
stronger check than searching flash for plaintext patterns; whole-image rollback
is deliberately demonstrated as an excluded threat.

Run the registered host suites from the repository root:

```sh
python3 validation/run.py run --suite group-communication-tests --suite group-communication-clippy --suite group-mls-adapter-tests --suite group-mls-adapter-clippy --suite group-mls-bounds-tests --platform current
```

The allocation-free core also checks for `thumbv6m-none-eabi`; the secure store
checks with `--no-default-features --features secure` for
`thumbv7em-none-eabihf` and `riscv32imac-unknown-none-elf`. These are library compile
checks, not linked firmware or measured target runtime results.

Remaining gates:

1. Audit current MLS serialization and all secret copies: flash retirement cannot
   fix consumed secrets still present in the latest state or application backups.
   Measure peak allocation for staging, old/new catalogs, AEAD and upstream MLS;
   zeroizing owned buffers does not prove every compiler/upstream transient is gone.
2. Extend the opt-in E290 fixture to erase/program worst cases, allocator/stack
   margins, wear accounting and loaded LoRa alongside the other radios; add
   controlled electrical interruption. Software resets around completed calls
   do not reproduce interrupted physical erase/program pulses.
3. Qualify irreversible provisioning and signed update/recovery on a dedicated
   device before enabling a physical-confidentiality profile. Clear all old
   plaintext qualification journals before real enrollment; encrypting new writes
   does not protect plaintext left by older firmware. No eFuses were changed for
   this implementation.

Source anchors: [current NOR journal](../src/flash.rs),
[coordinator/recovery contract](../src/lib.rs),
[MLS provider versions](../../security/mls-rs/src/provider.rs),
[MLS transitions](../../security/mls-rs/src/adapter/operations.rs),
[E290 raw flash driver](../../../personal-hopspot/embedded/esp32/src/flash.rs).
