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

## Ownership

The app owns the setup screens, native operation admission and lifecycle
integration. Generated bindings expose the shared card codec and durable runtime
operations. Firmware enrollment and board menus belong to the shared
RemoteControl implementation; the app does not introduce another authorization
protocol or require group messaging for setup.

## Validation boundaries

Run generated-binding verification, TypeScript checks, platform facade/effect
tests, native card/import and pairing lifecycle tests, and the affected node UI
tests on the exact source revision. Before claiming physical acceptance, test a
new menu pairing on the matching firmware/app, an authenticated read, app restart
and a full board power cycle, then refusal, expiry and interrupted confirmation.
Native API changes require rebuilt Android/iOS binaries. Record the source
revision and package checksums with each physical result; checks on a different
binary do not qualify the candidate.
