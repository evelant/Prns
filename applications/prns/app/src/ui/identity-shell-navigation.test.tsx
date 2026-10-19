import type * as Bindings from "@prns-internal/expo";
import { router } from "expo-router";
import { act, fireEvent, renderRouter, screen } from "expo-router/testing-library";
import { Text } from "react-native";
import { identityReferenceKey } from "@/native/named-identity-context";
import { NavigationLink } from "./navigation-link";
import { ShellLayout } from "./shell";

const mockPersonal: Bindings.NamedIdentityReference = {
  incarnation: new Uint8Array(16).fill(7),
  sequence: 1n,
  publicHash: new Uint8Array(16).fill(1),
};
const mockWork = { ...mockPersonal, sequence: 2n, publicHash: new Uint8Array(16).fill(2) };
jest.mock("@/native/named-identity-context", () => ({
  ...jest.requireActual("@/native/named-identity-context"),
  useNamedIdentity: () => {
    const params = jest.requireActual("expo-router").useGlobalSearchParams();
    return {
      catalog: { selected: mockPersonal },
      ownerKey: params.owner,
      identity: null,
      canPublish: false,
    };
  },
}));
jest.mock("@/state/scaffold-state-context", () => ({
  useScaffoldState: () => ({
    state: { scaffoldSchema: 2, showUnavailableFeatures: true },
  }),
}));

function Journey() {
  return <NavigationLink href="/contacts/card">Read saved card</NavigationLink>;
}

test.each([
  ["/contacts", "Contacts"],
  ["/inbox", "Inbox"],
] as const)(
  "the %s main tab returns to the default while journey links retain ownership",
  (path, label) => {
    const personal = identityReferenceKey(mockPersonal);
    const work = identityReferenceKey(mockWork);
    const view = renderRouter(
      {
        _layout: ShellLayout,
        contacts: Journey,
        inbox: Journey,
        "contacts/card": () => <Text>Saved contact card</Text>,
      },
      { initialUrl: `${path}?owner=${work}` },
    );
    fireEvent.press(screen.getByRole("link", { name: "Read saved card" }));
    expect(view.getPathname()).toBe("/contacts/card");
    expect(view.getSearchParams().owner).toBe(work);
    act(() => router.navigate({ pathname: path, params: { owner: work } }));
    expect(view.getSearchParams().owner).toBe(work);
    fireEvent.press(screen.getByRole("link", { name: `${label}, current page` }));
    expect(view.getPathname()).toBe(path);
    expect(view.getSearchParams().owner).toBe(personal);
  },
);
