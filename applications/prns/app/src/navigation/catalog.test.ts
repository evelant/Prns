import { navigationEntries, routeParamsAreValid, screenById, screenCatalog } from "./catalog";

describe("screen catalog", () => {
  it("keeps one closed, uniquely addressed route inventory", () => {
    expect(screenCatalog).toHaveLength(36);
    expect(new Set(screenCatalog.map(({ id }) => id)).size).toBe(36);
    expect(new Set(screenCatalog.map(({ path }) => path)).size).toBe(36);

    const routeFiles = screenCatalog.flatMap(({ routeFiles: files }) => files);
    expect(routeFiles).toHaveLength(37);
    expect(new Set(routeFiles).size).toBe(37);
  });

  it("marks the implemented presentation slices", () => {
    expect(
      screenCatalog
        .filter(({ availability }) => availability === "implementedScaffold")
        .map(({ id }) => id),
    ).toEqual([
      "installation.onboarding",
      "installation.setup",
      "installation.recovery",
      "inbox.index",
      "inbox.conversation",
      "inbox.group",
      "inbox.group-new",
      "inbox.group-invite",
      "inbox.invitation",
      "inbox.compose",
      "contacts.index",
      "contacts.entry",
      "contacts.add",
      "contacts.card",
      "contacts.import",
      "nodes.index",
      "nodes.local",
      "nodes.managed",
      "nodes.pair",
      "more.index",
      "identities.index",
      "identities.detail",
      "interfaces.index",
      "interfaces.detail",
      "interfaces.edit",
      "activity.index",
      "settings.index",
      "settings.about",
      "help.index",
    ]);
  });

  it("orders navigation roots and can hide unavailable entries", () => {
    expect(navigationEntries("phone", true).map(({ id }) => id)).toEqual([
      "inbox.index",
      "contacts.index",
      "nodes.index",
      "more.index",
    ]);
    expect(navigationEntries("phone", false).map(({ id }) => id)).toEqual([
      "inbox.index",
      "contacts.index",
      "nodes.index",
      "more.index",
    ]);
  });

  it("keeps retired browsing routes out of every navigation mode", () => {
    for (const layout of ["phone", "wide"] as const) {
      for (const showUnavailable of [true, false]) {
        expect(
          navigationEntries(layout, showUnavailable).some((entry) =>
            /explore|nomadnet|location/iu.test(`${entry.id} ${entry.path}`),
          ),
        ).toBe(false);
      }
    }
    expect(screenCatalog.some(({ path }) => path.startsWith("/explore"))).toBe(false);
    expect(navigationEntries("wide", false).map(({ id }) => id)).toContain("activity.index");
  });

  it("distinguishes implemented native state from remaining placeholders", () => {
    expect(screenById("more.index")).toMatchObject({
      summary: "Open identities, interfaces, network inspection, settings, and help.",
      limitation: "Messaging and node connections require the iOS or Android app.",
    });
    expect(screenById("notifications.settings").limitation).toBe(
      "Notifications and background message delivery are not available yet.",
    );
    expect(screenById("settings.storage").limitation).toBe(
      "Storage details are not available yet.",
    );
    expect(screenById("help.index").summary).toBe("Find setup, messaging, identity and node help.");
  });

  it.each([
    [
      "inbox.index",
      "Messaging is available on iOS and Android. Background delivery is not guaranteed.",
    ],
    ["contacts.index", "Contacts are available on iOS and Android."],
    ["nodes.index", "Node management is available on iOS and Android."],
    ["nodes.local", "Detailed node status is available on iOS and Android."],
    ["nodes.pair", "Pairing is available on iOS and Android with Bluetooth and a nearby node."],
  ] as const)("describes the supported mobile platforms for %s", (id, limitation) => {
    expect(screenById(id).limitation).toBe(limitation);
  });

  it("describes both supported paired-node controls without promising access", () => {
    expect(screenById("nodes.managed")).toMatchObject({
      summary: "View a paired node, check its connection, and share its address.",
      limitation:
        "Connection checks and node address sharing are available when the node allows them.",
    });
  });
});

test("only messaging routes accept a well-formed local identity owner", () => {
  const owner = `${"01".repeat(16)}-1-${"02".repeat(16)}`;
  expect(routeParamsAreValid(screenById("contacts.import"), { owner })).toBe(true);
  expect(routeParamsAreValid(screenById("inbox.group"), { owner, groupId: "03".repeat(32) })).toBe(
    true,
  );
  expect(routeParamsAreValid(screenById("contacts.import"), { owner: [owner] })).toBe(false);
  expect(routeParamsAreValid(screenById("contacts.import"), { owner: "bad" })).toBe(false);
  expect(routeParamsAreValid(screenById("nodes.index"), { owner })).toBe(false);
});
