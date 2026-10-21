# Optional board setup

Initial app setup can finish without a board. Add one later from Nodes. Pairing
and importing saved access both offer a separate connection check and an option
to configure the radio when needed. Keeping the current settings or finishing
later must retain the saved target.

## Invitation pairing

Open Pairing on the board, select the invitation in the app, compare the codes,
and approve on both devices. The app waits for durable completion. After success,
check access, configure the radio, pair another node, or finish. Cancelling or
retrying an attempt must keep previous saved targets and grants intact.

## Public setup cards

The app can export its controller card or raw public key for another setup tool.
It can preview and import a recipient-bound target-access card from a trusted
source. These operations use the shared RemoteControl codec and live runtime
identity; the app does not maintain a separate card protocol.

A preview shows the target, intended controller and authority. Import requires
explicit source trust, rejects a card for another controller and rejects a
conflicting existing access record. Native import verifies the complete recipient
key and returns success after persistence and activation. Accepted work continues
if the caller stops waiting; an uncertain outcome must not be presented as a
failed save or retried automatically.

Imported access is labeled saved. A card does not prove that the board currently
accepts that controller or that it is online. The separate authenticated check
establishes reachability/access at that time. The card contains public keys and
permissions, not private identity material.

## Review scope

This companion change contains the setup/card UI, native admission and lifecycle
integration, generated bindings and relevant tests. Baseline onboarding already
allows setup without a board. Messaging/group changes, named identities, general
navigation redesign, app release versions and installer artifacts are excluded.

The companion currently carries the shared setup API commit as a dependency on
the existing app branch. Once that API lands upstream, refresh the app base and
remove the duplicate dependency commit before publication or merge as appropriate.
No firmware code is added by the app-specific commit.

## Validation boundaries

Run generated-binding verification, TypeScript checks, platform facade/effect
tests, native card/import and pairing lifecycle tests, and the affected node UI
tests on the exact companion commit. Before claiming physical acceptance, test a
new menu pairing on the matching firmware/app, an authenticated read, app restart
and a full board power cycle, then refusal, expiry and interrupted confirmation.
The earlier Android/iOS binaries were built from a broader working tree; they do
not qualify the extracted companion branch or its newly generated bindings.
