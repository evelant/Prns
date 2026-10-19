# Embedded group support

This source boundary includes the allocation-free `no_std` group core and the
bounded `no_std + alloc` transaction coordinator and MLS adapter. The record
service, text codec, native enrollment/transport and app are separate integrations.
Reticulum routing and the group application protocol remain separate: a board can
forward group traffic without joining or holding MLS keys.

ESP32-S3 with PSRAM is the first full-member integration target. A complete
networked embedded member is still planned; the E290 storage fixture exercises
local MLS and recovery building blocks. Neither library compilation nor a normal
hotspot resource report qualifies the combined group workload.

## Support and ownership

| Target or role | Current boundary |
| --- | --- |
| Native host | Separate native integration supplies record service, enrollment, transport and runner. The native storage backend here is plaintext; protected phone storage is separate work. |
| ESP32-S3 with PSRAM | First embedded MLS target. Local E290 storage fixtures exist; portable enrollment, network synchronization and a full-member resource contract remain. |
| ESP32-S3 without PSRAM | Conditional expansion after measuring the complete workload; no group-member support promise. |
| ESP32-C6 | Routing is supported independently of groups. Group membership needs a separate memory, stack and single-core scheduling qualification. |
| nRF52840 | No initial MLS-member promise. Current hotspot profiles already have tight RAM/flash budgets; removing SoftDevice does not automatically reclaim its reserved layout. |
| Blind custodian | Planned opaque storage role. It needs authenticated admission and a bounded custody contract, but no MLS decryption keys. A transit router is not a custodian. |

Applications supply storage ownership, entropy, time, scheduling and record
meaning. They choose hard limits before admitting work; no board may silently
weaken the security profile to fit. The [participant design](embedded-participants.md)
defines the portable enrollment/carrier seams and independently enrolled board
role. [MLS security](../../security/docs/security.md) explains the protocol choice
and its delayed-delivery tradeoffs; the [adapter](../../security/mls-rs/README.md)
is authoritative for implemented quotas and missing integration.

## Resource contract

Qualify **board + firmware features + member/owner role + numeric profile**.
Owner Add/Commit, join and recovery peaks can exceed ordinary send costs.
Physical RAM totals, linked static headroom, reserved stack, sampled allocator
usage and largest free allocation are different quantities.

| Budget | Required accounting |
| --- | --- |
| Internal RAM and stacks | Radio/interrupt reserves, executor futures, crypto work buffers and call chains; PSRAM cannot replace every internal allocation. |
| Heap | One serialized MLS job initially; decoded state, snapshot copies, staging, indexes and fragmentation. Upstream infallible allocations prevent a general recoverable-OOM claim. |
| Persistent state | Crypto/current and retained epochs, membership history, immutable records/targets, exact ciphertext, receipts, replay floors and pending enrollment. |
| Maintenance | Spare capacity for the largest admitted transaction, catalog/capsule replacement and safe retirement. An alternate compaction arena must fit the live set; two arenas do not double live capacity. |
| Network/work | Largest encoded control/envelope, bounded queues and transfers, retry work, radio duty limits, scheduler yields and watchdog deadlines. |
| Time | Qualified clock uncertainty and durable expiry floors. Uptime or unauthenticated peer time cannot establish key expiry across reboot. |

Use the maintained [board memory contracts](../../../personal-hopspot/memory/src/profiles/mod.rs)
and [embedded resource validation](../../../docs/embedded-assurance.md), rather than
copying historical firmware headroom into a library capacity promise. C6 uses the
same 88 KiB radio heap split between reclaimed and ordinary RAM; that change does
not allocate an MLS budget. PSRAM is volatile, already shared with the existing
runtime, and does not supply durable storage.

For `N` members, `E` retained epochs, `K` skipped keys and `Q` records, budget
measured crypto state `C(N,E,K)` plus record bodies, target/evidence metadata,
control history, exact outgoing work and recovery tombstones. Multiple persisted
groups still consume quota when unloaded. Receipt traffic advances MLS generations
and consumes storage too. Bound count, bytes and time separately.

At one 1 KiB record per hour, 14 days of bodies alone is 336 KiB; at that rate per
member it scales with membership. A retention timer cannot promise offline custody
without sufficient storage and surviving content/keys. Oldest-content-first
best-effort eviction must never delete live crypto, control obligations or replay
fences. Reserved custody requires explicit admission and release/handoff rules.

The existing transport journal already owns its flash region and batching policy.
Reuse its recovery principles, not its record format or deferred write schedule.
Group crypto, application facts and exact output must commit together before
publication. See the [transaction coordinator](../../store/README.md) and
[secure flash contract](../../store/docs/secure-flash.md). Flash geometry, erase
latency, lifetime wear and uncertain commits belong to the board adapter. Logical
deletion is not physical erasure or rollback protection.

## E290 storage fixture

The separately integrated firmware fixture uses two local MLS peers, hardware
entropy and explicitly owned NOR partitions. Historical campaigns exercised
software resets around completed flash calls, exact-output recovery, encrypted
capsule/catalog compaction and receipt retirement alongside observed Wi-Fi work.
They did not establish remote group delivery, loaded BLE/LoRa behavior, maximum
admitted workloads, electrical power-loss recovery or current resource margins.
Dated captures are historical evidence, not a supported member profile.

The secure fixture stores its test wrapping key in plaintext. It therefore does
not qualify confidentiality against flash extraction, protected provisioning,
physical key retirement or rollback resistance. The firmware integration carries
the [firmware reproduction guide](../../../personal-hopspot/embedded/esp32/docs/mls-qualification.md)
and compiler configuration; no fixture partition is
implicitly available to the shipping record service.

## Release gates

1. Extract bounded enrollment/control and carrier state machines from native
   adapters. Preserve exact replay, authorization barriers and admission rules;
   keep Tokio, redb and app projections outside firmware.
2. Host-test one board-profile member with independent native peers. Exercise
   offline catch-up without the author, receipt reconciliation, interrupted joins,
   capacity exhaustion and uncertain clocks. Inject cuts at crypto, record,
   receipt, cursor, compaction and retirement boundaries.
3. Measure the complete firmware at the selected limits: member and owner roles,
   multiple persisted groups, retained epochs, many-sender gaps, maximum controls
   and malformed input. Record internal/PSRAM peaks, stack high-water, fragmentation,
   flash latency/wear and sustained radio/watchdog behavior.
4. Qualify protected provisioning and electrical interruption separately. Compare
   state before/after key retirement and test rollback/backup assumptions; a CRC,
   static wrapping key or software reset cannot establish those properties.
5. Verify independent endpoints through Python Reticulum routers and isolated
   LoRa paths, with authenticated receipts and bounded recovery. Preserve core
   protocol compatibility; application interoperability also needs versioned
   canonical vectors and a supported profile.

Only then publish board/member/owner limits or expand to smaller targets. The
[security contract](../../security/docs/security.md) and
[behavioral contract](v0.md) retain the unimplemented policy choices; neither
turns a proposed grace period, custody level or recovery rule into a guarantee.
