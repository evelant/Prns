import { MAX_CONTACT_CARD_URI_LENGTH } from "../features/contacts/contact-card-links";
import { type RawRouteParams, routeParamsAreValid, type ScreenId, screenById } from "./catalog";

describe("route parameter validation", () => {
  it.each<[ScreenId, RawRouteParams]>([
    ["installation.setup", {}],
    ["contacts.index", {}],
    ["contacts.add", {}],
    ["contacts.card", {}],
    ["contacts.import", { card: "", invalid: "0" }],
    ["contacts.entry", { destination: "ab".repeat(16) }],
    ["inbox.index", {}],
    ["inbox.compose", { destination: "ab".repeat(16) }],
    ["inbox.conversation", { destination: "ab".repeat(16) }],
    ["inbox.group-new", {}],
    ["inbox.group", { groupId: "ab".repeat(32) }],
    ["inbox.group-invite", { groupId: "ab".repeat(32) }],
    ["inbox.invitation", { invitationId: "ab".repeat(32) }],
  ])("accepts an immutable owner on the implemented %s journey", (screen, params) => {
    const entry = screenById(screen);
    const owner = `${"12".repeat(16)}-2-${"34".repeat(16)}`;
    expect(routeParamsAreValid(entry, { ...params, owner })).toBe(true);
    expect(routeParamsAreValid(entry, { ...params, owner: [owner] })).toBe(false);
    expect(routeParamsAreValid(entry, { ...params, owner: "work" })).toBe(false);
    expect(routeParamsAreValid(entry, { ...params, owner, unrelated: "value" })).toBe(false);
  });
  it("bounds editable contact drafts and never accepts group context from a link", () => {
    const entry = screenById("contacts.import");
    const card = "rns-contact://v1/YWJj";
    expect(routeParamsAreValid(entry, {})).toBe(true);
    expect(routeParamsAreValid(entry, { card })).toBe(true);
    expect(routeParamsAreValid(entry, { invalid: "1" })).toBe(true);
    expect(routeParamsAreValid(entry, { card: "", invalid: "0" })).toBe(true);
    expect(routeParamsAreValid(entry, { card: "rns-contact://v1/" })).toBe(true);
    expect(routeParamsAreValid(entry, { invalid: "2" })).toBe(false);
    expect(routeParamsAreValid(entry, { card: [card] })).toBe(false);
    expect(routeParamsAreValid(entry, { card, groupId: "ab".repeat(32) })).toBe(false);
    // These are untrusted text for review, not accepted cards or group parameters.
    expect(routeParamsAreValid(entry, { card: `${card}?groupId=other` })).toBe(true);
    expect(routeParamsAreValid(entry, { card: "https://example.org/card" })).toBe(true);
    expect(routeParamsAreValid(entry, { card: "A".repeat(MAX_CONTACT_CARD_URI_LENGTH + 1) })).toBe(
      true,
    );
    expect(routeParamsAreValid(entry, { card: "A".repeat(MAX_CONTACT_CARD_URI_LENGTH + 2) })).toBe(
      false,
    );
  });
  it("keeps creation parameter-free and invitations separate from group and contact IDs", () => {
    expect(routeParamsAreValid(screenById("inbox.group-new"), {})).toBe(true);
    expect(routeParamsAreValid(screenById("inbox.group-new"), { groupId: "ab".repeat(32) })).toBe(
      false,
    );
    expect(
      routeParamsAreValid(screenById("inbox.group-invite"), { groupId: "ab".repeat(32) }),
    ).toBe(true);
    const invitation = screenById("inbox.invitation");
    expect(routeParamsAreValid(invitation, { invitationId: "ab".repeat(32) })).toBe(true);
    expect(routeParamsAreValid(invitation, { invitationId: "ab".repeat(16) })).toBe(false);
    expect(routeParamsAreValid(invitation, { invitationId: ["ab".repeat(32)] })).toBe(false);
    expect(routeParamsAreValid(invitation, { groupId: "ab".repeat(32) })).toBe(false);
  });
  it("keeps group identities distinct from direct destinations", () => {
    const entry = screenById("inbox.group");
    expect(routeParamsAreValid(entry, { groupId: "ab".repeat(32) })).toBe(true);
    expect(routeParamsAreValid(entry, { groupId: "ab".repeat(16) })).toBe(false);
    expect(routeParamsAreValid(entry, { groupId: "xy".repeat(32) })).toBe(false);
    expect(routeParamsAreValid(entry, { groupId: ["ab".repeat(32)] })).toBe(false);
    expect(routeParamsAreValid(entry, {})).toBe(false);
  });
  it("accepts only the implemented optional Network filters", () => {
    const entry = screenById("activity.index");
    expect(entry.label).toBe("Network");
    expect(entry.availability).toBe("implementedScaffold");
    expect(routeParamsAreValid(entry, {})).toBe(true);
    for (const filter of ["connections", "routes", "announcements"]) {
      expect(routeParamsAreValid(entry, { filter })).toBe(true);
    }
    for (const filter of ["all", "network", "operation", "unknown"]) {
      expect(routeParamsAreValid(entry, { filter })).toBe(false);
    }
    expect(routeParamsAreValid(entry, { filter: ["routes"] })).toBe(false);
  });

  it("accepts only supported interface sections", () => {
    const entry = screenById("interfaces.index");
    expect(routeParamsAreValid(entry, {})).toBe(true);
    expect(routeParamsAreValid(entry, { section: "bluetooth" })).toBe(true);
    expect(routeParamsAreValid(entry, { section: "ip" })).toBe(true);
    expect(routeParamsAreValid(entry, { section: "unknown" })).toBe(false);
    expect(routeParamsAreValid(entry, { section: ["ip"] })).toBe(false);
  });

  it("rejects unknown parameters on routes without parameters", () => {
    const entry = screenById("nodes.index");

    expect(routeParamsAreValid(entry, {})).toBe(true);
    expect(routeParamsAreValid(entry, { unexpected: "value" })).toBe(false);
  });

  it("requires one non-empty scalar and rejects repeated values", () => {
    const entry = screenById("inbox.conversation");

    expect(routeParamsAreValid(entry, { destination: "00".repeat(16) })).toBe(true);
    expect(routeParamsAreValid(entry, {})).toBe(false);
    expect(routeParamsAreValid(entry, { destination: "not-a-destination" })).toBe(false);
    expect(routeParamsAreValid(entry, { destination: "" })).toBe(false);
    expect(routeParamsAreValid(entry, { destination: "   " })).toBe(false);
    expect(routeParamsAreValid(entry, { destination: ["00".repeat(16)] })).toBe(false);
  });

  it("accepts only declared onboarding steps as a single optional value", () => {
    const entry = screenById("installation.onboarding");

    expect(routeParamsAreValid(entry, {})).toBe(true);
    for (const step of [
      "welcome",
      "create",
      "import",
      "retention",
      "provision",
      "interfaces",
      "pairing",
      "complete",
    ]) {
      expect(routeParamsAreValid(entry, { step })).toBe(true);
    }
    expect(routeParamsAreValid(entry, { step: "future-step" })).toBe(false);
    expect(routeParamsAreValid(entry, { step: ["welcome", "complete"] })).toBe(false);
  });

  it("accepts only supported contact sections while preserving the identity owner", () => {
    const entry = screenById("contacts.index");
    const owner = `${"ab".repeat(16)}-1-${"cd".repeat(16)}`;
    expect(routeParamsAreValid(entry, {})).toBe(true);
    for (const section of ["saved", "discovered", "profile"]) {
      expect(routeParamsAreValid(entry, { section, owner })).toBe(true);
    }
    expect(routeParamsAreValid(entry, { section: "unknown" })).toBe(false);
    expect(routeParamsAreValid(entry, { section: ["profile"] })).toBe(false);
    expect(routeParamsAreValid(entry, { section: "profile", owner: "unknown" })).toBe(false);
  });

  it("keeps manual contact creation parameter-free", () => {
    const entry = screenById("contacts.add");

    expect(routeParamsAreValid(entry, {})).toBe(true);
    expect(routeParamsAreValid(entry, { source: "manual" })).toBe(false);
  });

  it("addresses contact details by one destination parameter", () => {
    const entry = screenById("contacts.entry");

    expect(routeParamsAreValid(entry, { destination: "00".repeat(16) })).toBe(true);
    expect(routeParamsAreValid(entry, {})).toBe(false);
    expect(routeParamsAreValid(entry, { destination: ["00".repeat(16)] })).toBe(false);
  });

  it("accepts only exact optional compose destinations", () => {
    const compose = screenById("inbox.compose");

    expect(routeParamsAreValid(compose, {})).toBe(true);
    expect(routeParamsAreValid(compose, { destination: "00".repeat(16) })).toBe(true);
    expect(routeParamsAreValid(compose, { destination: "destination:abc" })).toBe(false);
    expect(routeParamsAreValid(compose, { destination: "00".repeat(15) })).toBe(false);
  });
});
