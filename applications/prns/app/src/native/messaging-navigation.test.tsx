import * as Bindings from "@prns-internal/expo";
import { fireEvent, render } from "@testing-library/react-native";
import { type PropsWithChildren, useEffect, useState } from "react";
import { Text, TextInput } from "react-native";
import { CatalogRouteGuard } from "@/features/placeholder-screen";
import ProductShellLayout from "../../app/(shell)/_layout";
import { MessagingJourney } from "./messaging-journey";

let mockPath = "/setup";
let mockMounts = 0;
let mockUnmounts = 0;
let mockIdentity = {
  ownerKey: "personal",
  runtime: null,
  identity: {
    reference: { publicHash: new Uint8Array(16) },
    state: Bindings.NamedIdentityState.Enabled,
  },
  canPublish: true,
};
const PassThrough = ({ children }: PropsWithChildren) => children;
jest.mock("expo-router", () => ({
  usePathname: () => mockPath,
  useGlobalSearchParams: () => ({}),
  useLocalSearchParams: () => ({}),
  useRouter: () => ({ replace: jest.fn() }),
}));
jest.mock("@/native/named-identity-context", () => ({
  NamedIdentityProvider: ({ children }: PropsWithChildren) => children,
  useNamedIdentity: () => mockIdentity,
}));
jest.mock("@/native/development-runtime-context", () => ({
  DevelopmentRuntimeProvider: ({ children }: PropsWithChildren) => children,
  DevelopmentMessagingScope: ({ children }: PropsWithChildren) => children,
  routeConsumesDevelopmentSnapshot: () => true,
}));
jest.mock("@/native/contact-runtime-context", () => ({
  ContactRuntimeProvider: ({ children }: PropsWithChildren) => children,
}));
jest.mock("@/ui/shell", () => ({
  ShellLayout: () => <MockShellProbe />,
}));

function MockShellProbe() {
  useEffect(() => {
    mockMounts += 1;
    return () => {
      mockUnmounts += 1;
    };
  }, []);
  return <Text>{mockPath}</Text>;
}
function Draft() {
  const [text, setText] = useState("");
  return <TextInput accessibilityLabel="Contact draft" value={text} onChangeText={setText} />;
}
beforeEach(() => {
  mockPath = "/setup";
  mockMounts = 0;
  mockUnmounts = 0;
  mockIdentity = {
    ...mockIdentity,
    ownerKey: "personal",
    identity: { ...mockIdentity.identity, state: Bindings.NamedIdentityState.Enabled },
  };
});

test("entering a nested contact route from setup preserves the shell navigator", () => {
  const view = render(<ProductShellLayout />);
  expect(mockMounts).toBe(1);
  mockPath = "/contacts/card";
  view.rerender(<ProductShellLayout />);
  expect(view.getByText("/contacts/card")).toBeTruthy();
  expect(mockMounts).toBe(1);
  expect(mockUnmounts).toBe(0);
  mockIdentity = { ...mockIdentity, ownerKey: "work" };
  view.rerender(<ProductShellLayout />);
  expect(mockMounts).toBe(1);
});

test("contact route leaves discard the previous owner's local draft", () => {
  const screen = () => (
    <CatalogRouteGuard screenId="contacts.add">
      <Draft />
    </CatalogRouteGuard>
  );
  const view = render(screen());
  fireEvent.changeText(view.getByLabelText("Contact draft"), "Personal contact");
  mockIdentity = { ...mockIdentity, ownerKey: "work" };
  view.rerender(screen());
  expect(view.getByLabelText("Contact draft")).toHaveProp("value", "");
  fireEvent.changeText(view.getByLabelText("Contact draft"), "Work contact");
  mockIdentity = {
    ...mockIdentity,
    identity: { ...mockIdentity.identity, state: Bindings.NamedIdentityState.Paused },
  };
  view.rerender(screen());
  expect(view.getByLabelText("Contact draft")).toHaveProp("value", "");
});

test("wide master journeys reset ownership while their parent remains mounted", () => {
  const view = render(
    <PassThrough>
      <MessagingJourney>
        <Draft />
      </MessagingJourney>
    </PassThrough>,
  );
  fireEvent.changeText(view.getByLabelText("Contact draft"), "Personal draft");
  mockIdentity = { ...mockIdentity, ownerKey: "work" };
  view.rerender(
    <PassThrough>
      <MessagingJourney>
        <Draft />
      </MessagingJourney>
    </PassThrough>,
  );
  expect(view.getByLabelText("Contact draft")).toHaveProp("value", "");
});
