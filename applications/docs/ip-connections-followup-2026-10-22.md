# Public TCP and live conversation follow-up — 2026-10-22

## Scope

This follow-up to the mobile IP connection slice adds an enabled
`PRNS public node` (`prns.fly.dev:4242`) only when no settings row exists.
Saved empty, disabled, edited and duplicate endpoint intent is preserved.
Authenticated unnamed LXMF announcements become discoverable messaging peers
without weakening identity/destination or malformed metadata checks.

Named messaging views subscribe to the immutable identity owner's existing
LXMF change watch. Idle waits release service admission, stop wakes waiters,
and JavaScript aborts on identity, host-generation or foreground retirement.
A foreground reattachment resets its cursor. Incoming and delivery revisions
refresh the mounted conversation and its already loaded pages; installation
views retain their existing global-snapshot fallback. No mailbox polling or
additional node runtime is introduced.

## Containing-tree physical evidence

Standalone development builds from the containing working tree were updated in
place on an iPhone 14 Pro and Pixel. That tree also contained parallel pairing
and branding work excluded from this extracted follow-up. These observations
qualify the exercised feature behavior, not a byte-identical extracted PR
package or distribution release.

Both phones retained their public identities, saved peer settings and earlier
synthetic history. Each connected to the hostname `prns.fly.dev:4242` (observed
endpoint `188.93.150.14:4242`) without a local TCP fixture. Unnamed announcements
became mutually discoverable messaging peers after the parser correction. The
trial used TCP only: LAN was disabled in both apps; the iPhone had no Bluetooth
permission, and the Pixel used cellular with Wi-Fi and app Bluetooth off.

On October 22, the already mounted conversations exchanged exactly one message
per direction without Refresh, route navigation or reopening after send:

- `Auto iPhone` / `Auto refresh iPhone to Pixel 2026-10-22`: sender became
  Delivered and Pixel received Verified source, ID
  `de219a481924f594c17e7e150691d2820911b51c37456494a244c47b59bf85f2`.
- `Auto Pixel` / `Auto refresh Pixel to iPhone 2026-10-22`: sender became
  Delivered and iPhone received Verified source, ID
  `cf10f279382ca7f271237a2a674c053adb3f7220255e4600eee4289a51d55896`.

Both messages and earlier records were preserved. Test-only LAN/Bluetooth
preferences were restored; the public peer remained enabled. The original iOS
production bundle was untouched. The temporary UI runner was removed. Public
screenshots, build/signature receipts and semantic checkpoints remain in the
workstation's ignored validation artifacts; no private key or app container was
exported. The signed iOS development binary had no multicast entitlement.

## Qualification boundary

The extracted follow-up needs its own generated-contract, native and frontend
source gates; its PR records those exact results. The new native change-watch
method requires rebuilt binaries and matching generated bindings. These checks
do not establish background or locked-idle delivery, public-server uptime,
store distribution, or group messaging over the public peer.
