# Provision a development group on phones

This helper installs one privately generated `member-N.json` as
`group-fixture.json` in the existing **development** app's private storage.
It does not build, install, launch, stop, or clear an app; it never modifies the
identity vault or `application.redb`. Production app containers are unsupported.

Use a freshly generated group for a new trial. The fixture generator and native
enrollment behavior are documented in
[`prns-groups/runner/native`](../../../prns-groups/runner/native/README.md).
Each artifact belongs to its selected endpoint's existing public Reticulum
identity. Do not copy a member's artifact to two devices, or reuse it after
resetting that device's database. Keep the generated directory private (0700)
and member files private (0600). The files contain initial private MLS state.

## Prepare

1. Select the intended physical device explicitly. Android uses `adb devices -l`;
   iOS uses `xcrun devicectl list devices`. Do not select a phone merely because
   it is the only available device.
2. Install the current development build separately. Initialize its node identity,
   then copy the public primary identity hash from the app. Generate the group
   artifacts against those identities before provisioning. Do not export private
   identity files to determine the public hash.
3. Stop the app process and keep it stopped until the helper finishes. Merely
   stopping its node leaves the native owner alive. The helper checks the process
   inventory and refuses an app that is still running; it does not terminate it.
4. Run `check` with the selected serial/UDID, that phone's public identity hash,
   and its member artifact. Then run the same command with `install`.

From the repository root:

```sh
./tools/prns device groups provision -- check \
  --platform android --device ANDROID_SERIAL \
  --identity PHONE_PRIMARY_IDENTITY_HEX --fixture /private/trial/member-1.json

./tools/prns device groups provision -- install \
  --platform ios --device IPHONE_UDID \
  --identity PHONE_PRIMARY_IDENTITY_HEX --fixture /private/trial/member-2.json
```

The identity argument checks the artifact's local member against the public hash
you copied from the selected phone. It does **not** independently read the phone's
vault. On the next native app start, enrollment checks the actual stored primary
identity before importing state. A mismatch fails closed and is displayed as
group unavailability; preserve the existing state and investigate the selection.

Android needs a debuggable development app accessible with `run-as`. Its private
destination is `no_backup/prns/development/group-fixture.json`. Release builds
that deny `run-as` cannot be provisioned this way; there is no root or public
shared-storage fallback. The helper streams the fixture privately, verifies its
digest, and creates the final name exclusively using shell noclobber. It refuses
an existing name, including a symlink. Android 10's `run-as` SELinux policy can
prohibit hard links, so publication uses a second copy into that exclusive file.
The name creation is atomic, but the final contents are not: an interrupted copy
can leave a partial fixture that must be preserved for investigation. Keep the app
stopped throughout. File flushing uses Android 10's no-argument `sync` command.

iOS needs a paired device with Developer Mode, Xcode's `devicectl`, and an app
installed with container access. Its container-relative destination is
`Library/Application Support/prns/development/group-fixture.json`.
`devicectl` has no atomic create-if-absent file operation. The helper checks for an
existing fixture immediately before copying and excludes concurrent invocations
using the same explicit device ID on this host. Keep the app stopped and do not
use another host or container editor during installation. This is a coordinated
development operation, not a race-safe production enrollment protocol.

## Outcomes and recovery

The helper prints only public IDs, a SHA-256 digest, and one of these outcomes:

| Outcome | Meaning |
| --- | --- |
| `ready-to-install` | Read-only preflight succeeded; no fixture exists. |
| `installed` | Copy completed and the device's bytes matched. |
| `already-present` | The exact artifact is already present; nothing was replaced. |

An existing different fixture always refuses replacement, including partial files
left by an interrupted Android or iOS copy. A timeout or failed readback is not success.
Retry `check` to determine whether the original copy completed; do not delete or
overwrite a different artifact, reset the database, or recreate a used MLS
incarnation to make a retry pass. The tool deliberately offers no `--force` or
cleanup mode. Native enrollment imports only into a vacant group namespace;
subsequent starts verify its configuration and open the persisted ratchets.

After successful provisioning, restart the app explicitly, start its node, and
check the group conversation and synchronization status. Keep the fixture on the
phone: native startup still uses its configuration. The retained plaintext
initial enrollment snapshot is a development limitation, not production key
retirement or forward-secrecy evidence. It must not be used as a recovery backup.

Private iOS staging/readback copies use temporary directories accessible only to
the current user and are removed when the command exits. This does not promise
secure erasure, and Python does not guarantee in-memory zeroization. Tool failures
omit raw device output and fixture contents. A successful copy does not qualify
background execution, radio delivery, or recovery; measure those separately.

## Validation

```sh
python3 validation/run.py run --suite applications-group-provisioning
```

The 24 focused tests exercise identity binding, private files, no replacement,
idempotency, stopped-process checks, host locking, verification failures, and
command privacy. Android publication is also tested using the local shell's real
exclusive-create behavior. Unit tests mock device commands and iOS JSON schemas.
The October 13 trial (historical local record) also
installed and verified enrollment on MetalbeardMobile and the Android 10 Galaxy
S9+, including actual Android exclusive-create and symlink refusal checks.
Other device/tool versions must pass `check` before claiming support.
Unknown `devicectl` schemas and inaccessible containers fail closed.
Existing iOS fixture readback accepts the observed `metadata.size` inventory
field and legacy top-level size fields, requires a bounded integer size, and
refuses conflicting sizes, directories, symlinks or malformed resource flags.
The downloaded byte count must still match the inventory before its digest is used.
