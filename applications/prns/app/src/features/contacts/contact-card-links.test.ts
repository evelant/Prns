import { redirectSystemPath } from "../../../app/+native-intent";
import { isContactCardUriShape, MAX_CONTACT_CARD_URI_LENGTH } from "./contact-card-links";

describe("portable contact link admission", () => {
  it.each([true, false])("rewrites a %s initial link to preview only", (initial) => {
    const card = "rns-contact://v1/YWJj_-09";
    expect(redirectSystemPath({ path: card, initial })).toBe(
      `/contacts/import?card=${encodeURIComponent(card)}`,
    );
  });
  it.each([
    "rns-contact://v2/YWJj",
    "rns-contact://v1/",
    "rns-contact://v1/YWJj=",
    "rns-contact://v1/YWJj?groupId=abc",
    "rns-contact://v1/YWJj#invite",
    "rns-contact://v1/YW%2Fj",
    "rns-contact://other/v1/YWJj",
    "RNS-CONTACT://v1/YWJj",
    `rns-contact://v1/${"A".repeat(369)}`,
  ])("rejects altered or oversized card intent %s", (path) => {
    expect(redirectSystemPath({ path, initial: false })).toBe("/contacts/import?invalid=1");
  });
  it("leaves existing application navigation alone", () => {
    expect(redirectSystemPath({ path: "/inbox", initial: true })).toBe("/inbox");
    expect(redirectSystemPath({ path: "rs.reticulum.prns://contacts", initial: false })).toBe(
      "rs.reticulum.prns://contacts",
    );
    expect(isContactCardUriShape(`rns-contact://v1/${"A".repeat(368)}`)).toBe(true);
    expect(MAX_CONTACT_CARD_URI_LENGTH).toBe(385);
  });
});
