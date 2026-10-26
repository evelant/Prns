# Maintainer group quickstart

Use this guide for a small foreground demo with test messages. Keep prns open on
each phone and use matching standalone builds. See [validation](validation.md) for
recorded checks, known limits and the remaining APK/TestFlight handoff gates.

## Before the demo

- Use nearby Bluetooth for the simplest setup; no external board or node pairing
  is required. Both phones need at least one matching discovery-group name
  (`reticulum` is the default).
- Keep only one prns app variant participating in Bluetooth on each phone.
  If an older development app is installed, disable Bluetooth **inside that app**;
  do not erase its data.
- Enroll the demo participants before sending. The owner delays a join while its
  own current messages await receipts, including messages sent after inviting.
  Other members' unfinished sends can still be interrupted; coordinate additions.
  New members receive future messages, not earlier history.
- Use the supplied build's install/update instructions. Preserve app data during
  updates and recovery; backup/restore is not a supported group-key recovery path.

## Set up and choose an identity

On a fresh installation, choose **Set up this phone → Connect this phone**. Start
this phone, allow Bluetooth, enable nearby connections and choose an identity.
Set **Name others see** if desired. Existing installations can open connection
setup from Inbox or Connections.

The identity control at the top chooses which identity Inbox and Contacts use.
**More → Identities** creates and manages independent identities such as Personal
and Work. Their private names are local labels; their public messaging names,
contacts, groups and histories are separate. Switching identity does not move a
conversation or copy its contacts.

Other active identities keep receiving. Their unread messages and invitations
appear in the identity picker. Selecting an identity does not mark its messages
viewed; opening the conversation does. Invitations remain pending until accepted,
declined or expired. A **+** or unavailable activity indicator means the summary
is incomplete, not that no other activity exists.

**Pause identity** stops its services after admitted work settles. Saved history
remains available while the shared local node runs. **Resume identity** restores
its services. Pausing or changing the selected identity does not change board
pairings or pause other identities.

## Share a contact, invite and send

1. On one phone, open **Contacts → My contact QR** or **Share contact link**.
   On the other phone, choose **Scan or paste contact**, select **Save contact to**,
   review the card and save it. Repeat in the other direction. Choosing the save
   identity does not change the default identity. A signed card authenticates its
   identity binding; it does not prove the peer is reachable or the name is trusted.
2. On the creator, choose **Inbox → New group**. Give it a name, then choose
   **Invite a device** and select the other phone's saved contact. **Scan or paste
   contact** can also prepare that selection. Confirm the recipient and press
   **Send invitation** once.
3. On the recipient, open the invitation in **Inbox** and accept. Keep both apps
   open until joining completes. The owner's **Device joined** state confirms
   the joining phone acknowledged its stored membership.
4. Open the group and send a short message in each direction. Check the receipt
   on the sending phone and the matching message on the receiving phone.

If an invitation already exists, open **View existing invitation**. An already
enrolled device cannot be invited again. Advanced manual entry requires the
recipient's device identity, not its messaging destination address.

**Waiting for current deliveries** means the latest response said the owner was
finishing pending deliveries. Your acceptance is saved; the same join retries
automatically, including after restart. The status can lag while confirmation is
in transit. A definite message expiry can also release
the wait; it does not count as delivery. An invitation can expire while waiting.
Do not resend the message or create another invitation to bypass the wait.
After delivery finishes, joining resumes on its next automatic attempt. On slower
or unknown paths, retries can be minutes apart; connection failures can extend
the wait.

| Status | Meaning |
| --- | --- |
| Saved locally / Waiting for devices | The message is stored for retry; remote delivery is not yet confirmed |
| Received by 1 of 1 devices | The other enrolled device acknowledged receipt; this is not a human read receipt |
| Device joined | Enrollment completed and was acknowledged |
| Waiting for current deliveries | Joining is waiting for the owner's pending receipts or definite message expiry |
| Retry scheduled | A future attempt is scheduled; the countdown is not an arrival estimate |
| Delivery interrupted by membership change | The group changed before this message completed delivery |

**Group details** shows members and invitation history. **Delivery details**
shows connection/retry observations. Technical explanations are behind info or
Details controls so the conversation can stay focused on messages.

## Manage a group

The owner can remove another device from **Group details**, cancel an unused
outgoing invitation, or close the group. If joining committed before cancellation,
the device appears as a member and can be removed there. Finished invitations can
be dismissed from the local list.

Members can choose **Leave group**. Participation stops locally immediately;
the saved departure waits for the owner to commit removal. Closing is the owner's
departure action. Removal and closure retain readable local history and disable
new sends. They cannot erase copies already held by other devices. Offline peers
learn the change when they reconnect.

**Delete from this device** removes local group history and private group state.
Owners must close first. A member's local deletion alone does not change remote
membership; leave first to request removal. Already saved departure and closure
notifications survive local deletion. A removed member can accept a fresh
invitation with new credentials; a closed group cannot be reopened.

### Recreate the earlier test group

The management protocol and stored group state use version two. This alpha does
not migrate version-one groups or interoperate with older group clients. Upgrade
all participants and recreate the disposable test group. Unsupported versions
are reported explicitly; opening the app does not erase them.

For an existing version-one database, stop the app and keep a copy of the database.
The host-only reset tool removes its legacy group namespaces, invitation catalog
and saved group actions, preserving identity, contact and node-setting tables:

```sh
cargo run --locked --manifest-path applications/Cargo.toml -p prns-app-native \
  --features host-test --bin reset-test-groups -- \
  --database /absolute/path/to/application.redb --confirm-legacy-reset
```

Run it on the relevant identity's closed database on each participating device
(or an offline copy restored while the app is stopped). It refuses current
version-two group catalogs. Reset can resume after interruption. Create a new
group and invitations after reopening; the old group's address is not reused.

Future storage conversions and network upgrades are separate decisions. Preserve
signed historical bytes, validate converted state, and make any required
conversion atomic and restart-safe when real retained data requires one.

## Check recovery

Use unique test text so an old message cannot be mistaken for a new arrival.
For identity recovery, pause the receiving identity, send one message from the
other phone and verify it remains pending. Pause the sending identity, restart
that app, and confirm its original message and paused state remain. Resume the
receiver, then the sender, and wait for the original message and its receipt.

To test a connection outage instead, disable the receiving app's Bluetooth
connection under **Nodes → This phone → Connections**. That makes it unreachable
only if no other usable path remains. Re-enable it and observe the same saved
message. A Bluetooth test is not evidence of a LoRa hop; isolated radio testing
has a [separate procedure](../../prns-groups/transport/reticulum/docs/lora-qualification.md).

If the result of Send, Accept or Invite is uncertain, open the saved action or use
its **Retry same…** path. Do not create a replacement or clear data. A saved action
may already have succeeded. If retry evidence expired, review the existing
messages/invitations before starting new work.

## When something stalls

Check the chosen identity is Active, the local node is running, and Connections
shows actual connected devices. **Ready** alone is not a connection to the peer.
Confirm matching discovery groups and keep both apps open. Inspect **Delivery
details** for retry, storage or membership issues. Announcements help discovery;
**Announce groups now** does not itself deliver queued content.

For a report, open **More → About → Diagnostics**, review the summary and choose
**Share diagnostics**. **Refresh diagnostics** reads status again; the text can
also be selected and copied. The summary includes build and connection status,
not messages, names, addresses, identities or keys.

Add the phone models/connection method, visible receipt/error state and what
happened before the stall. Preserve the original message and note any manual
intervention. A screenshot of Delivery details can help; avoid sensitive content.

## Alpha limits

- Up to eight named identities, four groups per identity and eight devices per
  group. Local deletion releases its active-group slot. Group names and text are
  bounded to 128 and 2048 UTF-8 bytes. Retained membership history is bounded by
  storage bytes; there is no seven-change lifetime limit or history compaction.
- One owner orders membership changes. Ownership transfer, history transfer, attachments and
  a shared identity across multiple phones are not implemented.
- Keep the apps open. Background delivery and notifications are not guaranteed.
- Saved group data is not application-encrypted. Protected storage and supported
  backup/recovery remain deferred; use test data.

For implementation and reusable APIs, start with the [workspace guide](../README.md)
and [group runner](../../prns-groups/runner/native/README.md). The
[roadmap](roadmap.md) separates the next alpha work from later product features.
