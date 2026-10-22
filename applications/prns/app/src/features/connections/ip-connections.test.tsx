import * as Bindings from "@prns-internal/expo";
import { fireEvent, render, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";
import { Linking } from "react-native";

import type { DevelopmentRuntimeView } from "@/native/development-runtime-context";
import { IpConnections, SetupLanChoice } from "./ip-connections";
import { connectedIpCount } from "./ip-physical-connections";
import { parseTcpPeerDraft } from "./tcp-peer-form";

jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
}));
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => mockRuntime,
}));

type ConnectionRuntime = Pick<
  DevelopmentRuntimeView,
  | "connectionSnapshot"
  | "connectionFailure"
  | "readConnectionSettings"
  | "updateConnectionSettings"
  | "refreshConnections"
  | "retryConnection"
  | "requestLocalNetworkPermission"
>;
let mockRuntime: { -readonly [Key in keyof ConnectionRuntime]: ConnectionRuntime[Key] };
const saved = (
  revision = 1n,
  tcpPeers: Bindings.TcpPeerProfile[] = [],
): Bindings.ConnectionSettingsSnapshot => ({
  ownerId: 1n,
  revision,
  settings: { lanEnabled: false, tcpPeers },
});
const ready = (snapshot: Bindings.ConnectionSettingsSnapshot) => ({
  type: "outcome" as const,
  outcome: Bindings.ConnectionSettingsOutcome.Ready.new({ snapshot }),
});
function connectionSnapshot(settings = saved()): Bindings.ConnectionSnapshot {
  return {
    generationId: 1n,
    settings,
    appliedRevision: settings.revision,
    lan: {
      desiredEnabled: false,
      state: Bindings.LocalConnectionState.Disabled,
      discoveryMethod: Bindings.LanDiscoveryMethod.Bonjour,
      detail: undefined,
      stockMulticastAvailable: false,
      connectedPeerCount: 0,
    },
    tcpPeers: [],
    physicalConnections: [],
  };
}
beforeEach(() => {
  mockRuntime = {
    connectionSnapshot: connectionSnapshot(),
    connectionFailure: null,
    requestLocalNetworkPermission: jest.fn(async () => ({
      type: "outcome" as const,
      outcome: "notRequired" as const,
    })),
    readConnectionSettings: jest.fn(async () => ready(saved())),
    updateConnectionSettings: jest.fn(async (input) =>
      ready({
        ownerId: input.ownerId,
        revision: input.expectedRevision + 1n,
        settings: {
          ...input.settings,
          tcpPeers: input.settings.tcpPeers.map((peer) => ({
            ...peer,
            id: peer.id || "a".repeat(32),
          })),
        },
      }),
    ),
    refreshConnections: jest.fn(async () => ({
      type: "outcome" as const,
      outcome: Bindings.ConnectionSnapshotOutcome.Ready.new({ snapshot: connectionSnapshot() }),
    })),
    retryConnection: jest.fn(async () => ({
      type: "outcome" as const,
      outcome: Bindings.ConnectionSnapshotOutcome.Ready.new({ snapshot: connectionSnapshot() }),
    })),
  };
});

async function addPeer(view: ReturnType<typeof render>) {
  await waitFor(() => expect(view.getByRole("button", { name: "Add TCP peer" })).toBeEnabled());
  fireEvent.press(view.getByRole("button", { name: "Add TCP peer" }));
  fireEvent.changeText(view.getByLabelText("Peer name"), "Home relay");
  fireEvent.changeText(view.getByLabelText("Hostname or IP address"), "relay.example.org");
  fireEvent.changeText(view.getByLabelText("Port"), "4242");
}

test("saves an offline TCP peer with native-assigned identity and revision, independent of multicast", async () => {
  const view = render(<IpConnections />);
  await addPeer(view);
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() => expect(view.getByText("Home relay")).toBeTruthy());
  expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledWith({
    ownerId: 1n,
    expectedRevision: 1n,
    settings: {
      lanEnabled: false,
      tcpPeers: [
        { id: "", label: "Home relay", host: "relay.example.org", port: 4242, enabled: true },
      ],
    },
  });
  expect(mockRuntime.retryConnection).not.toHaveBeenCalled();
  expect(view.getByText("Waiting for connection status")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Local network details" }));
  expect(view.getByText("Bonjour · compatible PRNS peers")).toBeTruthy();
  expect(view.getByText(/Stock Reticulum multicast discovery is unavailable/)).toBeTruthy();
});

test("a revision conflict preserves the draft and requires a new explicit save", async () => {
  const update = jest.spyOn(mockRuntime, "updateConnectionSettings").mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.ConnectionSettingsOutcome.Conflict.new({ snapshot: saved(2n) }),
  });
  const view = render(<IpConnections />);
  await addPeer(view);
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() => expect(view.getByText(/Connection settings changed/)).toBeTruthy());
  expect(update).toHaveBeenCalledTimes(1);
  expect(view.getByLabelText("Peer name").props.value).toBe("Home relay");
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() => expect(update).toHaveBeenCalledTimes(2));
  expect(update.mock.calls[1]?.[0].expectedRevision).toBe(2n);
});

test("LAN permission denial preserves desired intent and leaves saved TCP controls usable", async () => {
  const request = jest
    .spyOn(mockRuntime, "requestLocalNetworkPermission")
    .mockResolvedValue({ type: "outcome", outcome: "denied" });
  const view = render(<IpConnections />);
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Enable local network discovery" })).toBeEnabled(),
  );
  expect(request).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Enable local network discovery" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Open local network settings" })).toBeTruthy(),
  );
  expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledWith({
    ownerId: 1n,
    expectedRevision: 1n,
    settings: { lanEnabled: true, tcpPeers: [] },
  });
  await addPeer(view);
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() => expect(view.getByText("Home relay")).toBeTruthy());
  expect(request).toHaveBeenCalledTimes(1);
});

test("setup offers LAN discovery without enabling it until the user chooses it", async () => {
  const view = render(<SetupLanChoice />);
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Enable local network discovery" })).toBeEnabled(),
  );
  expect(mockRuntime.updateConnectionSettings).not.toHaveBeenCalled();
  expect(mockRuntime.requestLocalNetworkPermission).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Enable local network discovery" }));
  await waitFor(() =>
    expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledWith({
      ownerId: 1n,
      expectedRevision: 1n,
      settings: { lanEnabled: true, tcpPeers: [] },
    }),
  );
  expect(mockRuntime.requestLocalNetworkPermission).toHaveBeenCalledTimes(1);
});

test.each([Bindings.LocalConnectionState.Unavailable, Bindings.LocalConnectionState.Failed])(
  "enabled LAN in state %s offers app settings without inferring permission denial",
  async (state) => {
    const settings = saved();
    settings.settings.lanEnabled = true;
    const snapshot = connectionSnapshot(settings);
    snapshot.lan.desiredEnabled = true;
    snapshot.lan.state = state;
    snapshot.lan.detail = "Bonjour could not start";
    mockRuntime.connectionSnapshot = snapshot;
    jest.spyOn(mockRuntime, "readConnectionSettings").mockResolvedValue(ready(settings));
    const openSettings = jest.spyOn(Linking, "openSettings").mockResolvedValue();
    const view = render(<IpConnections />);
    await waitFor(() =>
      expect(view.getByRole("button", { name: "Open local network settings" })).toBeEnabled(),
    );
    expect(mockRuntime.requestLocalNetworkPermission).not.toHaveBeenCalled();
    expect(view.queryByText(/Allow local network access in app settings/)).toBeNull();
    fireEvent.press(view.getByRole("button", { name: "Open local network settings" }));
    expect(openSettings).toHaveBeenCalledTimes(1);
    openSettings.mockRestore();
  },
);

test("a lost save acknowledgement rereads durable settings without replay or optimistic connection", async () => {
  const update = jest
    .spyOn(mockRuntime, "updateConnectionSettings")
    .mockResolvedValue({ type: "operationFailure", detail: "The native response was lost" });
  const read = jest.spyOn(mockRuntime, "readConnectionSettings");
  const view = render(<IpConnections />);
  await addPeer(view);
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() => expect(view.getByText(/The save result was not confirmed/)).toBeTruthy());
  expect(read).toHaveBeenCalledTimes(2);
  expect(update).toHaveBeenCalledTimes(1);
  expect(view.queryByText("Waiting for connection status")).toBeNull();
  expect(view.getByLabelText("Hostname or IP address").props.value).toBe("relay.example.org");
});

test("an unconfirmed save and failed reread block further edits until settings are checked", async () => {
  jest
    .spyOn(mockRuntime, "updateConnectionSettings")
    .mockResolvedValue({ type: "operationFailure", detail: "Response lost" });
  jest
    .spyOn(mockRuntime, "readConnectionSettings")
    .mockResolvedValueOnce(ready(saved()))
    .mockResolvedValue({ type: "operationFailure", detail: "Storage temporarily unavailable" });
  const view = render(<IpConnections />);
  await addPeer(view);
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() => expect(view.getByText(/The save result is still unconfirmed/)).toBeTruthy());
  expect(view.getByRole("button", { name: "Save peer" })).toBeDisabled();
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledTimes(1);
  expect(view.getByRole("button", { name: "Refresh saved connections" })).toBeEnabled();
});

test("retry submits native fences without rewriting saved intent", async () => {
  const profile = {
    id: "b".repeat(32),
    label: "Remote relay",
    host: "192.0.2.1",
    port: 4242,
    enabled: true,
  };
  const snapshot = connectionSnapshot(saved(3n, [profile]));
  snapshot.tcpPeers = [
    {
      profile,
      interfaceId: undefined,
      state: Bindings.LocalConnectionState.Reconnecting,
      detail: "Connection refused",
      rxBytes: 0n,
      txBytes: 0n,
    },
  ];
  mockRuntime.connectionSnapshot = snapshot;
  jest.spyOn(mockRuntime, "readConnectionSettings").mockResolvedValue(ready(snapshot.settings));
  const view = render(<IpConnections />);
  await waitFor(() => expect(view.getByRole("button", { name: "Retry peer" })).toBeEnabled());
  fireEvent.press(view.getByRole("button", { name: "Retry peer" }));
  await waitFor(() =>
    expect(mockRuntime.retryConnection).toHaveBeenCalledWith({
      ownerId: 1n,
      generationId: 1n,
      expectedRevision: 3n,
      profileId: profile.id,
    }),
  );
  expect(mockRuntime.updateConnectionSettings).not.toHaveBeenCalled();
});

test("removal needs an explicit confirmation and persists the remaining peer list", async () => {
  const profile = {
    id: "b".repeat(32),
    label: "Remote relay",
    host: "192.0.2.1",
    port: 4242,
    enabled: false,
  };
  jest.spyOn(mockRuntime, "readConnectionSettings").mockResolvedValue(ready(saved(3n, [profile])));
  const view = render(<IpConnections />);
  await waitFor(() => expect(view.getByRole("button", { name: "Remove peer" })).toBeEnabled());
  fireEvent.press(view.getByRole("button", { name: "Remove peer" }));
  expect(mockRuntime.updateConnectionSettings).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Remove saved peer" }));
  await waitFor(() =>
    expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledWith({
      ownerId: 1n,
      expectedRevision: 3n,
      settings: { lanEnabled: false, tcpPeers: [] },
    }),
  );
});

test("editing a disabled peer preserves its stable ID and desired disabled state", async () => {
  const profile = {
    id: "b".repeat(32),
    label: "Remote relay",
    host: "192.0.2.1",
    port: 4242,
    enabled: false,
  };
  jest.spyOn(mockRuntime, "readConnectionSettings").mockResolvedValue(ready(saved(3n, [profile])));
  const view = render(<IpConnections />);
  await waitFor(() => expect(view.getByRole("button", { name: "Edit peer" })).toBeEnabled());
  fireEvent.press(view.getByRole("button", { name: "Edit peer" }));
  fireEvent.changeText(view.getByLabelText("Hostname or IP address"), "replacement.example.org");
  fireEvent.press(view.getByRole("button", { name: "Save peer" }));
  await waitFor(() =>
    expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledWith({
      ownerId: 1n,
      expectedRevision: 3n,
      settings: { lanEnabled: false, tcpPeers: [{ ...profile, host: "replacement.example.org" }] },
    }),
  );
  expect(mockRuntime.retryConnection).not.toHaveBeenCalled();
});

test("enable and disable save explicit desired state without optimistic connection health", async () => {
  const profile = {
    id: "b".repeat(32),
    label: "Remote relay",
    host: "192.0.2.1",
    port: 4242,
    enabled: false,
  };
  jest.spyOn(mockRuntime, "readConnectionSettings").mockResolvedValue(ready(saved(3n, [profile])));
  const view = render(<IpConnections />);
  await waitFor(() => expect(view.getByRole("button", { name: "Enable peer" })).toBeEnabled());
  fireEvent.press(view.getByRole("button", { name: "Enable peer" }));
  await waitFor(() => expect(view.getByRole("button", { name: "Disable peer" })).toBeEnabled());
  expect(mockRuntime.updateConnectionSettings).toHaveBeenCalledWith({
    ownerId: 1n,
    expectedRevision: 3n,
    settings: { lanEnabled: false, tcpPeers: [{ ...profile, enabled: true }] },
  });
  expect(view.getByText("Waiting for connection status")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Disable peer" }));
  await waitFor(() => expect(view.getByRole("button", { name: "Enable peer" })).toBeEnabled());
  expect(mockRuntime.updateConnectionSettings).toHaveBeenLastCalledWith({
    ownerId: 1n,
    expectedRevision: 4n,
    settings: { lanEnabled: false, tcpPeers: [profile] },
  });
});

test("physical counts exclude local rendezvous and disconnected peers", () => {
  const snapshot = connectionSnapshot();
  const peer = {
    interfaceId: new Uint8Array(8),
    logicalInterfaceId: new Uint8Array(8),
    parentInterfaceId: undefined,
    profileId: undefined,
    kind: "WifiPeer",
    name: undefined,
    state: Bindings.LocalConnectionState.Connected,
    rxBytes: 3n,
    txBytes: 5n,
    endpoint: undefined,
    localOnly: false,
  };
  snapshot.physicalConnections = [
    peer,
    { ...peer, localOnly: true },
    { ...peer, kind: "TcpClient", state: Bindings.LocalConnectionState.Reconnecting },
  ];
  expect(connectedIpCount(snapshot)).toBe(1);
});

test("peer form validates nonzero ports, names and UTF-8 limits without resolving DNS", () => {
  expect(parseTcpPeerDraft(" Home ", " relay.example.org ", "4242")).toEqual({
    label: "Home",
    host: "relay.example.org",
    port: 4242,
  });
  expect(typeof parseTcpPeerDraft("Home", "relay.example.org", "0")).toBe("string");
  expect(typeof parseTcpPeerDraft("", "relay.example.org", "4242")).toBe("string");
  expect(typeof parseTcpPeerDraft("é".repeat(33), "relay.example.org", "4242")).toBe("string");
  expect(typeof parseTcpPeerDraft("Home", "https://relay.example.org", "4242")).toBe("string");
});
