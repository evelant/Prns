# RemoteControl pairing maintainer checks

Record the exact source commit, firmware checksum, controller/app version,
physical board revision, host/phone OS and reset method for every result.
Build success does not establish physical qualification. Keep USB enrollment,
menu pairing, radio configuration and over-air group delivery as separate results.

## Branch and historical evidence

The shared change is based on upstream after the Heltec V3 SRAM-runtime merge.
The app integration is a separate companion to the existing app PR. Validate the
exact proposed branch commits before merge; earlier working-tree builds do not
qualify these extracted branches.

The earlier local campaign `0.3.8-pairing.20261021c` built all five menu boards.
Two E290 upgrades verified full private backups, unchanged retained data before
first boot, matching application readback, and authentication through existing
controller grants. Configured LoRa stayed connected; unconfigured LoRa changed
from Failed to Disabled with its settings unchanged. The owner confirmed both
physical Pairing menus and startup screens. Those checks used watchdog resets.

That campaign predates this clean extraction and the upstream V3 runtime adapter.
It is historical evidence for the original fixes. It does not establish new
invitation pairing, full power cycles, third-party-client interoperability or
physical operation of the other four models on the proposed branches. Published
or distributed binaries must identify their own exact source and checksums.

## Board matrix

| Board | Pairing path | Flash | Physical checks that matter |
| --- | --- | --- | --- |
| Heltec Vision Master E290-HF | Shared menu; user button GPIO21 | 16 MB | Normal menu font/alignment, e-ink refresh, configured and unconfigured startup |
| Heltec WiFi LoRa 32 V3/V3.1 | Shared menu on upstream S3FN8 runtime | 8 MB | Button actions, confirmation ordering, repeated sessions, SRAM stability |
| Heltec LoRa 32 V4/S3R2 | Shared menu | 16 MB | Display/button behavior, restart access, optional radio check |
| Heltec LoRa 32 V4 R8/S3R8 | Shared menu | 16 MB | Correct R8 firmware, display/button behavior, restart access |
| LilyGO T-Beam S3 Supreme/SX1262 | Shared menu | 8 MB | Display/button behavior, OLED recovery after reset/power cycle |

Confirm the physical model, radio band, flash size and existing partition layout.
ESP32-S3 detection alone cannot distinguish these boards. Do not interchange V4
and V4 R8 images. Tracker V2 and nRF menu support remain separate work.

## Preserve existing installations

Use the canonical board package and partition table. Back up all flash privately
before changing firmware. Backups can contain secret identities and grants;
never attach them to a public report. Close other serial users first. A backup
must finish successfully and have the full expected byte length.

With espflash 4.5.0, keep its RAM stub enabled. For slow/unreliable reads, use
`--block-size 0x1000 --max-in-flight 1`. If reading separate chunks, keep the
application stopped, use consecutive addresses and compare the assembled file's
MD5 with the whole-device checksum before treating it as a usable backup.

Flash only the supplied bootloader, partition table and application regions.
Do not erase the whole chip or write a filled image across retained partitions.
A compatible partition layout is required; unknown layouts need a separate
migration. Disable any flash-time enrollment option when updating an existing
board. Preserve its identity even when another controller will be added.

## Required E290 invitation check before merge

Keep the existing recovery administrator. Use a new independent controller for
this lane so prior authorization cannot hide a failed pairing attempt.

1. Complete app setup without a board, then choose to add one later.
2. Open **Menu → Pairing** on the board. Check the font, alignment and invitation.
3. Select the board and enter its invitation in the controller. Compare the
   confirmation code on both screens and review the requested authority.
4. Approve on the board and controller. Both must reach durable success. A
   controller-only approval must not imply that the board approved.
5. Keep existing radio settings or choose to finish configuration later. Perform
   an authenticated read. A failed configuration/read must retain saved access.
6. Restart the app and fully power-cycle the board. Reconnect and authenticate
   without pairing again. Record startup time and any retained-state warning.
7. Check a refused/mismatched confirmation, expired invitation, board/controller
   cancellation and interrupted confirmation using an unpaired controller.
   None may report successful authorization without durable completion. Rejecting
   a new attempt must not revoke an older grant. Recover uncertain outcomes by
   checking state, not by automatically replaying authorization.
8. Reopen pairing after each failed/cancelled attempt. On V3, repeat open/cancel
   ten times and make five sequential authenticated reads to check stability.

An unconfigured LoRa card should show **SubG / Setup**, with no Failed state.
Pairing alone must not choose a region/profile or start receiving/transmitting.
A configured board must keep its profile. A separate deliberate radio test needs
compatible settings and antennas; Bluetooth pairing success does not prove LoRa.

## USB setup and independent controllers

- Export a controller card or raw public key, enroll over the physical USB
  connection and verify the pinned target. Unsupported firmware must fail before
  an authorization request.
- Export/import target access only after the matching Saved receipt. Reject a
  card addressed to another controller and conflicting existing access.
- Check waiting, Continue checking, Stop checking, disconnect/reconnect and stale
  receipts. Reconciliation must not submit another grant.
- Authenticate with imported access, restart the controller and board, and repeat.
- Delegate a narrower grant between distinct controllers. Verify permitted reads,
  denied administrator operations and exact permission retention after restart.
- Record third-party controller builds precisely. Raw-key compatibility does not
  establish support for the new public-card carrier.

## Result template

| Check | Pass / fail / skipped | Evidence or reason |
| --- | --- | --- |
| Exact firmware and controller identities recorded | | |
| Backup and compatible retained layout verified | | |
| Normal startup, menu font and button actions | | |
| New invitation and matching code | | |
| Approval required on both devices | | |
| Authenticated access after durable pairing | | |
| Existing radio settings kept / configuration deferred | | |
| App restart and full board power cycle | | |
| Refusal, mismatch, expiry and cancellation | | |
| Interrupted confirmation and recovery | | |
| Existing grants remain usable | | |
| USB setup and receipt reconciliation, if tested | | |
| Repeated sessions / stability | | |

Attach redacted logs or observations with timings. Do not include private keys,
flash backups, controller-state directories, Wi-Fi credentials, active invitation
codes or complete device inventories. Group delivery is a separate qualification
and is not required to demonstrate the shared pairing flow.
