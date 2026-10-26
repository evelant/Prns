# E290 MLS qualification

The Heltec Vision Master E290-HF can run an opt-in MLS storage fixture alongside
the hotspot radio workload. It requires an ESP32-S3 with 8 MiB PSRAM and 16 MiB
flash. Two local peers exercise durable MLS state, recovery and key retirement;
this does not make the board a network group participant. Shipping builds leave
the fixture disabled.

The generic group core is allocation-free `no_std`. This adapter uses
`no_std + alloc`. See the [shared storage contract](../../../../prns-groups/store/docs/secure-flash.md)
for publication, reclamation and receipt-retirement invariants.

## Run

Prepare the [embedded tooling](../../../../docs/embedded-assurance.md), then use
the registered device task from the repository root. Replace the serial-port
placeholder with the exact board port and use a fresh output directory:

```sh
./tools/prns device hopspot mls-e290 --variant secure \
  --target-dir target/qualification-build/e290 \
  --output target/qualification/e290-secure \
  --port /dev/cu.usbmodemPORT --erase-mls --monitor-seconds 240
```

`--erase-mls` clears only `0x600000..0xE00000`, the two experimental peer stores.
Omit it and select a new output directory to test persistent reopen. Switching
from the original journal to secure storage requires an explicit erase; there
is no in-place migration. `--build-only` avoids device access, and
`--variant baseline` builds the radio comparison image.

The tool records compiler identity, effective flags, image hashes, serial logs
and recovery markers. A completed collector is not itself a qualification pass.
Preserve failed and incomplete runs with successful runs in an external evidence
archive; do not commit generated captures. A saved `--elf` needs independently
verified build provenance, since the tool cannot infer its flags from the file.

## Profiles and storage

| Variant | Storage | Recovery exercise |
| --- | --- | --- |
| `mls` | Original plaintext NOR journal; 64 append-only slots per peer | Torn body, unpublished transaction, and publication before caller success |
| `secure` | Encrypted capsule/catalog store with compaction and retirement fences | Catalog publication, old-arena erase, send publication, and predecessor-key retirement |

The [qualification partition table](../partitions-hopspot-16mb-mls.csv) reserves
two 4 MiB peer partitions and limits the application to `0x10000..0x600000`.
Existing identity, settings and routing offsets are preserved. Firmware checks
partition ownership before mount. Use the matching table; restoring shipping
firmware can make the experimental storage area available for other uses.

Each secure peer reserves 8 KiB for fixture provisioning, four 4 KiB key-capsule
sectors and two 2,084,864-byte data arenas. The fixture wrapping key is stored in
plaintext, and secure boot, flash encryption and eFuses are unchanged. This
exercises authenticated storage and recovery, not confidentiality against raw
flash extraction or rollback of a complete flash image. Do not use real private
group data with these fixtures.

The secure scenario checks immutable pending output after reset, receipt
retirement through ticket 19 and rejection of older tickets after reopen. Cuts
occur between completed NOR calls, so they do not qualify electrical power loss
during a program or erase pulse. The original journal refuses new writes when
full and provides no reclamation.

## Resource and compiler limits

The [memory profile](../../../memory/src/profiles/espressif/mls_qualification.rs)
reserves a 40 KiB worker stack plus a guard before radio allocations fragment
internal RAM. MLS runs only with the additional internal reserve available and
yields between bounded flash operations. PSRAM remains split between the 4 MiB
engine arena and 4 MiB general heap; the shared allocator is not an MLS quota.

Admitted limits are four members, one group per peer, 16 KiB wire messages,
1 KiB bodies and 32 KiB snapshots. The fixture workload uses two members and
does not establish worst-case resource use at every admitted limit. It uses
runtime entropy but a synthetic validation timestamp.

The qualification task applies `-C force-frame-pointers=no` to baseline and MLS
builds, and the build guard rejects the known unsafe configuration. Keep the
[call-frame reproducer](../../../../prns-groups/security/mls-rs/fixtures/xtensa-call-frame/README.md)
and final-ELF inspection in compiler changes. The workaround does not establish
correctness for every generated function.

## Assess a run

Compare matching compiler settings and image hashes. Heap peaks include the
worker reservation and other firmware allocations; sampled free space does not
measure stack high-water, fragmentation or every transient peak. Phase latency
includes serialization, cryptography, full-partition scans and scheduling.
Flash-call timings include driver overhead and core parking.

Check each reset boundary, recovered outputs, plaintext checks, final receipt
fences and no-erase reopen. Report actual radio traffic during the measurement;
Wi-Fi activity alone does not qualify sustained LoRa/BLE contention or deadlines.
Electrical interruption, protected key provisioning, full-load memory/stack
measurements and independent network peers remain separate qualification work.
