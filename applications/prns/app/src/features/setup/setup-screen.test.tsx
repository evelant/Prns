import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";
import { Linking } from "react-native";
import { identityReferenceKey } from "@/native/named-identity-context";
import type { RawRouteParams } from "@/navigation/catalog";
import SetupRoute from "../../../app/(shell)/setup";
import { SetupScreen } from "./setup-screen";

const mockLinks: unknown[] = [];
const mockSetParams = jest.fn();
let mockRouteParams: RawRouteParams = {};
jest.mock("expo-router", () => ({
  useRouter: () => ({ setParams: mockSetParams, replace: jest.fn() }),
  useLocalSearchParams: () => mockRouteParams,
  Link: ({ children, href }: { readonly children: ReactNode; readonly href: unknown }) => {
    mockLinks.push(href);
    return children;
  },
}));
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => mockRuntime,
}));
jest.mock("@/native/named-identity-context", () => ({
  ...jest.requireActual("@/native/named-identity-context"),
  useNamedIdentity: () => mockIdentities,
}));
const reference = {
  incarnation: new Uint8Array(16).fill(1),
  sequence: 1n,
  publicHash: new Uint8Array(16).fill(2),
};
const identity = {
  reference,
  label: "Personal",
  publishedName: undefined,
  ready: true,
  transitioning: false,
  state: Bindings.NamedIdentityState.Enabled,
};
const profile = (displayName: string): Bindings.LocalMessagingProfileOutcome =>
  Bindings.LocalMessagingProfileOutcome.Ready.new({
    profile: { displayName, destination: new Uint8Array(16) },
  });
const mockReadName = jest.fn(async () => profile(""));
const mockSaveName = jest.fn(async (name: string) => profile(name));
const mockEnable = jest.fn(async (_enabled: boolean) => ({
  type: "outcome" as const,
  outcome: Bindings.LocalBluetoothSettingsOutcome.Ready.new({ enabled: true }),
}));
const mockRequestPermission = jest.fn(async () => {});
const mockChange = jest.fn(
  async (_reference: Bindings.NamedIdentityReference, _change: Bindings.NamedIdentityChange) =>
    true,
);
const mockDiscovery = jest.fn();
const mockRadio = jest.fn();
function fixture() {
  return {
    availability: { type: "available", platform: "android" },
    phase: "ready",
    snapshot: {
      runtime: Bindings.DevelopmentNodeRuntime.Running,
      primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
        identityHash: new Uint8Array(16),
      }),
      bluetooth: {
        desiredEnabled: false,
        state: Bindings.LocalBluetoothState.Disabled.new() as Bindings.LocalBluetoothState,
        peers: [],
      },
    },
    stoppingNode: false,
    canStartNode: true,
    startNode: jest.fn(),
    lifecycleFailure: null,
    bluetoothAuthorization: null,
    bluetoothAuthorizationFailure: null,
    androidRuntime: {
      status: { bluetoothPermission: "granted", bluetoothRadio: "on", locationServices: "on" },
      failure: null,
      refresh: jest.fn(async () => {}),
      requestBluetoothPermissions: mockRequestPermission,
    },
    refreshSnapshot: jest.fn(async () => ({ type: "outcome" })),
    setBluetoothEnabled: mockEnable,
    setBluetoothDiscoveryGroups: mockDiscovery,
    changeRemoteNode: mockRadio,
  };
}
let mockRuntime = fixture();
let mockIdentities = {
  catalog: { selected: reference, identities: [identity] },
  identity,
  ownerKey: identityReferenceKey(reference),
  runtime: { readMessagingProfile: mockReadName, setMessagingName: mockSaveName },
  canPublish: true,
  loading: false,
  failure: null,
  refresh: jest.fn(async () => {}),
  change: mockChange,
};
function ready() {
  mockRuntime.snapshot.bluetooth = {
    desiredEnabled: true,
    state: Bindings.LocalBluetoothState.WaitingForPeers.new(),
    peers: [],
  };
}
beforeEach(() => {
  jest.clearAllMocks();
  mockLinks.length = 0;
  mockRuntime = fixture();
  mockIdentities = {
    catalog: { selected: reference, identities: [identity] },
    identity,
    ownerKey: identityReferenceKey(reference),
    runtime: { readMessagingProfile: mockReadName, setMessagingName: mockSaveName },
    canPublish: true,
    loading: false,
    failure: null,
    refresh: jest.fn(async () => {}),
    change: mockChange,
  };
  mockReadName.mockResolvedValue(profile(""));
  mockSaveName.mockImplementation(async (name) => profile(name));
  mockChange.mockResolvedValue(true);
  mockSetParams.mockReset();
  mockRouteParams = {};
  mockEnable.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.LocalBluetoothSettingsOutcome.Ready.new({ enabled: true }),
  });
});

test("guides phone startup, explicit nearby enablement, name and owner-bound next actions", async () => {
  mockRuntime.snapshot.runtime = Bindings.DevelopmentNodeRuntime.Stopped;
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Start this phone" }));
  expect(mockRuntime.startNode).toHaveBeenCalledTimes(1);
  mockRuntime.snapshot.runtime = Bindings.DevelopmentNodeRuntime.Running;
  view.rerender(<SetupScreen />);
  expect(mockEnable).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Enable nearby connections" }));
  await waitFor(() => expect(mockEnable).toHaveBeenCalledWith(true));
  ready();
  view.rerender(<SetupScreen />);
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Continue to your identity" })).toBeEnabled(),
  );
  fireEvent.press(view.getByRole("button", { name: "Continue to your identity" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Save and continue" })).toBeEnabled(),
  );
  fireEvent.changeText(view.getByLabelText("Name others see"), "Ari");
  fireEvent.press(view.getByRole("button", { name: "Save and continue" }));
  await waitFor(() => expect(view.getByText("Ready to say hello")).toBeTruthy());
  expect(mockSaveName).toHaveBeenCalledWith("Ari");
  expect(mockLinks).toContainEqual({
    pathname: "/contacts/card",
    params: { owner: identityReferenceKey(reference) },
  });
  expect(mockLinks).toContainEqual({
    pathname: "/inbox/group/new",
    params: { owner: identityReferenceKey(reference) },
  });
  expect(mockDiscovery).not.toHaveBeenCalled();
  expect(mockRadio).not.toHaveBeenCalled();
});

test("denied permission remains recoverable and never reports nearby readiness", async () => {
  mockRuntime.androidRuntime.status.bluetoothPermission = "denied";
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Allow Bluetooth" }));
  await waitFor(() => expect(mockRequestPermission).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(view.getByRole("button", { name: "Allow Bluetooth" })).toBeEnabled());
  expect(view.getByRole("button", { name: "Continue to your identity" })).toBeDisabled();
  expect(mockEnable).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Allow Bluetooth" }));
  await waitFor(() => expect(mockRequestPermission).toHaveBeenCalledTimes(2));
  mockRuntime.androidRuntime.status.bluetoothPermission = "granted";
  view.rerender(<SetupScreen />);
  expect(view.getByRole("button", { name: "Enable nearby connections" })).toBeTruthy();
  expect(mockDiscovery).not.toHaveBeenCalled();
});

test("identity setup can continue offline without Bluetooth permission or a radio", async () => {
  mockRuntime.androidRuntime.status.bluetoothPermission = "denied";
  const view = render(<SetupScreen />);
  expect(view.getByRole("button", { name: "Continue to your identity" })).toBeDisabled();
  fireEvent.press(view.getByRole("button", { name: "Continue offline" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Save and continue" })).toBeEnabled(),
  );
  expect(mockEnable).not.toHaveBeenCalled();
  expect(mockRequestPermission).not.toHaveBeenCalled();
});

test("blocked permission opens settings and a powered-off radio is not ready", async () => {
  const settings = jest.spyOn(Linking, "openSettings").mockResolvedValue();
  mockRuntime.androidRuntime.status.bluetoothPermission = "blocked";
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Open app settings" }));
  await waitFor(() => expect(settings).toHaveBeenCalledTimes(1));
  mockRuntime.androidRuntime.status.bluetoothPermission = "granted";
  mockRuntime.androidRuntime.status.bluetoothRadio = "off";
  ready();
  view.rerender(<SetupScreen />);
  expect(view.getByRole("button", { name: "Continue to your identity" })).toBeDisabled();
  expect(view.getByRole("button", { name: "Open Bluetooth settings" })).toBeTruthy();
  settings.mockRestore();
});

test("reopening setup preserves the existing profile and connection configuration", async () => {
  ready();
  mockReadName.mockResolvedValue(profile("Existing name"));
  const view = render(<SetupScreen />);
  expect(view.getByText("Ready for nearby phones")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Continue to your identity" }));
  await waitFor(() => expect(view.getByDisplayValue("Existing name")).toBeTruthy());
  expect(mockEnable).not.toHaveBeenCalled();
  expect(mockSaveName).not.toHaveBeenCalled();
  expect(mockChange).not.toHaveBeenCalled();
  expect(mockDiscovery).not.toHaveBeenCalled();
  expect(mockRadio).not.toHaveBeenCalled();
});

test("an unconfirmed name save stays on the captured identity for retry", async () => {
  ready();
  mockSaveName.mockResolvedValue(
    Bindings.LocalMessagingProfileOutcome.SavedButNotApplied.new({
      profile: { displayName: "Ari", destination: undefined },
      detail: "host busy",
    }),
  );
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Continue to your identity" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Save and continue" })).toBeEnabled(),
  );
  fireEvent.changeText(view.getByLabelText("Name others see"), "Ari");
  fireEvent.press(view.getByRole("button", { name: "Save and continue" }));
  await waitFor(() =>
    expect(view.getByText("Your name is saved. Try again to finish applying it.")).toBeTruthy(),
  );
  expect(view.queryByText("Ready to say hello")).toBeNull();
});

test("iOS access denial offers Settings and cannot be mistaken for readiness", async () => {
  Object.assign(mockRuntime, {
    availability: { type: "available", platform: "ios" },
    androidRuntime: null,
    bluetoothAuthorization: {
      authorization: "denied",
      nativeStart: "permissionDenied",
      restorationAttemptRequested: false,
      revision: 1,
    },
  });
  ready();
  const settings = jest.spyOn(Linking, "openSettings").mockResolvedValue();
  const view = render(<SetupScreen />);
  expect(view.getByRole("button", { name: "Continue to your identity" })).toBeDisabled();
  fireEvent.press(view.getByRole("button", { name: "Open app settings" }));
  await waitFor(() => expect(settings).toHaveBeenCalledTimes(1));
  expect(mockRequestPermission).not.toHaveBeenCalled();
  expect(mockEnable).not.toHaveBeenCalled();
  settings.mockRestore();
});

test("iOS enablement awaits observed permission and actual Bluetooth readiness", async () => {
  Object.assign(mockRuntime, {
    availability: { type: "available", platform: "ios" },
    androidRuntime: null,
    bluetoothAuthorization: {
      authorization: "notDetermined",
      nativeStart: "notStarted",
      restorationAttemptRequested: false,
      revision: 1,
    },
  });
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Enable nearby connections" }));
  await waitFor(() => expect(mockEnable).toHaveBeenCalledWith(true));
  expect(view.getByRole("button", { name: "Continue to your identity" })).toBeDisabled();
  Object.assign(mockRuntime, {
    bluetoothAuthorization: {
      authorization: "allowedAlways",
      nativeStart: "running",
      restorationAttemptRequested: false,
      revision: 2,
    },
  });
  ready();
  view.rerender(<SetupScreen />);
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Continue to your identity" })).toBeEnabled(),
  );
});

test("switching identity does not let the previous identity's late save complete setup", async () => {
  ready();
  let resolveSave: ((outcome: Bindings.LocalMessagingProfileOutcome) => void) | undefined;
  mockSaveName.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        resolveSave = resolve;
      }),
  );
  const work = { ...identity, label: "Work", reference: { ...reference, sequence: 2n } };
  mockIdentities.catalog.identities = [identity, work];
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Continue to your identity" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Save and continue" })).toBeEnabled(),
  );
  fireEvent.changeText(view.getByLabelText("Name others see"), "Personal name");
  fireEvent.press(view.getByRole("button", { name: "Save and continue" }));
  await waitFor(() => expect(mockSaveName).toHaveBeenCalledWith("Personal name"));
  mockIdentities = {
    ...mockIdentities,
    identity: work,
    ownerKey: identityReferenceKey(work.reference),
    catalog: { selected: work.reference, identities: [identity, work] },
  };
  view.rerender(<SetupScreen />);
  await act(async () => {
    resolveSave?.(profile("Personal name"));
  });
  expect(view.queryByText("Ready to say hello")).toBeNull();
  expect(view.getByText("Use Work for messages and groups.")).toBeTruthy();
  expect(mockSaveName).toHaveBeenCalledTimes(1);
});

test("choosing Work stays on the real owned setup route and saves only its profile", async () => {
  ready();
  mockRouteParams = { owner: identityReferenceKey(reference) };
  mockReadName.mockResolvedValue(profile("Personal name"));
  const work = { ...identity, label: "Work", reference: { ...reference, sequence: 2n } };
  const workRead = jest.fn(async () => profile("Work name"));
  const workSave = jest.fn(async (name: string) => profile(name));
  mockIdentities.catalog.identities = [identity, work];
  mockChange.mockImplementationOnce(async () => {
    // Updating the default alone must not retarget an explicit Personal route.
    mockIdentities.catalog.selected = work.reference;
    return true;
  });
  mockSetParams.mockImplementation(({ owner }: { owner: string }) => {
    mockRouteParams = { ...mockRouteParams, owner };
    mockIdentities = {
      ...mockIdentities,
      ownerKey: owner,
      identity: work,
      runtime: { readMessagingProfile: workRead, setMessagingName: workSave },
    };
  });
  const view = render(<SetupRoute />);
  fireEvent.press(view.getByRole("button", { name: "Continue to your identity" }));
  await waitFor(() => expect(view.getByDisplayValue("Personal name")).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Work" }));
  await waitFor(() =>
    expect(mockSetParams).toHaveBeenCalledWith({ owner: identityReferenceKey(work.reference) }),
  );
  view.rerender(<SetupRoute />);
  await waitFor(() => expect(view.getByDisplayValue("Work name")).toBeTruthy());
  expect(view.queryByText("Connect nearby")).toBeNull();
  fireEvent.changeText(view.getByLabelText("Name others see"), "Team Ari");
  fireEvent.press(view.getByRole("button", { name: "Save and continue" }));
  await waitFor(() => expect(view.getByText("Ready to say hello")).toBeTruthy());
  expect(workSave).toHaveBeenCalledWith("Team Ari");
  expect(mockSaveName).not.toHaveBeenCalled();
  expect(mockLinks).toContainEqual({
    pathname: "/contacts/card",
    params: { owner: identityReferenceKey(work.reference) },
  });
});

test("failed identity selection preserves the current route and unsaved name", async () => {
  ready();
  const work = { ...identity, label: "Work", reference: { ...reference, sequence: 2n } };
  mockIdentities.catalog.identities = [identity, work];
  mockChange.mockResolvedValueOnce(false);
  const view = render(<SetupScreen />);
  fireEvent.press(view.getByRole("button", { name: "Continue to your identity" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Save and continue" })).toBeEnabled(),
  );
  fireEvent.changeText(view.getByLabelText("Name others see"), "Personal draft");
  fireEvent.press(view.getByRole("button", { name: "Work" }));
  await waitFor(() =>
    expect(view.getByText("That identity could not be selected. Try again.")).toBeTruthy(),
  );
  expect(mockSetParams).not.toHaveBeenCalled();
  expect(view.getByDisplayValue("Personal draft")).toBeTruthy();
  expect(mockSaveName).not.toHaveBeenCalled();
});
