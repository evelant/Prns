# Application validation and limits

This is a development application. Host tests, source exports, packaged builds
and device trials establish different things. A later checkout does not inherit
physical acceptance from an older binary. Consult the actual CI run and retain
source/package hashes with each result.

Use the [workspace guide](../README.md) for setup, the
[SDK guide](react-native-sdk-implementation.md) for ownership, and the
[roadmap](roadmap.md) for remaining work. The app at this boundary supports direct
LXMF messaging and node/network controls; group and named-identity app journeys
are not integrated. Separate group libraries and firmware experiments do not
establish those app capabilities.

## Repeatable checks

| Gate | What it establishes |
| --- | --- |
| `api:check` | Generated contracts match source |
| `verify` | Portable native, SDK/platform, UI, route, configuration and web-export checks |
| `mobility:verify -- --working-tree` | Isolated source export, packed dependencies and supported native/Python LXMF exchange |
| `mobility:verify` | Recorded-release contract; promotion requires review |
| `native:ios:test` | Swift lifecycle and release-symbol checks |
| `native:android:standalone` / `native:ios:build` | Embedded-JavaScript packages; installation and physical acceptance are separate |

The standalone SDK's JavaScript-owned session reopens after reload; the app's
native owner can outlive JavaScript replacement. Test each separately using the
[SDK qualification guide](../../prns-react-native/docs/qualification.md).
Current-source export is not recorded-release or public-distribution qualification.
Preserve the [release contract](../release/README.md) until a containing revision
and matching artifacts are reviewed.

Firmware changes require the configured resource gates on their exact source.
Linked fit is not peak stack/heap measurement or physical recovery. Retain
reports, source hashes and maps with the qualification run rather than copying
temporary file paths or historical headroom into current guides.

## Foreground phone check

1. Install the intended package on each phone without clearing retained data.
   Record build/source identity, signing method and device/OS versions.
2. Keep only one app variant participating in Bluetooth per phone. Enable nearby
   connections through normal app permission; no board pairing or OS bond is
   needed for phone-to-phone messaging.
3. Announce distinct messaging names, discover and save contacts, then send short
   direct messages in both directions. Check the original message and its
   proof-backed result; sender verification is a separate observation.
4. Restart one app while the other remains running. Send to the saved contact,
   then test a temporary connection outage and recovery without replacing pending
   work or clearing data. Record actual delivery and duplicates, not just routes.
5. Inspect physical connections, routes and accepted announcements. Clearing
   discovery/history must preserve saved contacts and messages.

Keep test text unique, preserve failed attempts and record any manual intervention.
Follow [iOS](ios.md) and [Android](android.md) procedures for platform checks.

## Separate acceptance still required

- Long locked/unplugged idle, natural suspension, repeated restoration,
  silent-peer expiry and permission/radio transitions on supported OS versions.
- Fresh pairing and capability-driven node reads/writes, successful Wi-Fi Keep,
  denied/Busy results, cancellation and uncertain-write recovery on matching
  Android/iOS and firmware builds. Retained grants do not prove fresh pairing.
- Complete detached aggregate iOS consumption, public package distribution,
  signed release installation/update, protected keys and supported backup/recovery.

Neither an Android foreground service nor iOS event-driven Bluetooth windows
guarantees continuous execution or background delivery. Reproduce old faults on
relevant builds before prescribing fixes. Keep generated logs, screenshots and
per-run narratives outside the source documentation.
