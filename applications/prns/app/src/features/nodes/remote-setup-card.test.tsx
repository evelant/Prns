import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";
import type { RuntimeCommandResult } from "@/native/development-runtime-context";
import { RemoteSetupCard } from "./remote-setup-card";

const controller = new Uint8Array(16).fill(0x11);
const target = new Uint8Array(16).fill(0x22);
const cardText = "prns-rc:target-card-for-this-controller";
const exported: Bindings.ControllerSetupCard = {
  text: "prns-rc:controller-card",
  publicKeyHex: "ab".repeat(64),
  controllerIdentityFingerprint: controller,
};
const preview: Bindings.TargetAccessCardPreview = {
  targetIdentityFingerprint: target,
  recipientControllerFingerprint: controller,
  authority: Bindings.RemoteControlControllerAuthority.Operator,
  permissions: [
    Bindings.RemoteControlRequestKind.Describe,
    Bindings.RemoteControlRequestKind.ConfigureRadio,
  ],
};

const mockLinks: unknown[] = [];
const mockExport = jest.fn<
  Promise<RuntimeCommandResult<Bindings.ControllerSetupCardOutcome>>,
  []
>();
const mockPreview = jest.fn<
  Promise<RuntimeCommandResult<Bindings.TargetAccessCardPreviewOutcome>>,
  [string]
>();
const mockImport = jest.fn<
  Promise<RuntimeCommandResult<Bindings.TargetAccessCardImportOutcome>>,
  [Bindings.ImportTargetAccessCardInput]
>();
const mockDescribe = jest.fn<
  Promise<RuntimeCommandResult<Bindings.RemoteControlDescribeOutcome>>,
  [Bindings.DescribeRemoteControlTargetInput]
>();
const mockChange = jest.fn();

function snapshot(): Bindings.DevelopmentNodeSnapshot {
  return {
    contractFingerprint: "test",
    revision: 1n,
    generationId: 1n,
    runtime: Bindings.DevelopmentNodeRuntime.Running,
    controllerIdentityFingerprint: controller,
    primaryIdentity: Bindings.PrimaryIdentityState.Missing.new(),
    localHost: Bindings.LocalHostState.Stopped.new({ lastStartFailure: undefined }),
    network: {
      state: Bindings.LocalNetworkState.Stopped.new(),
      routes: [],
      announces: [],
      activityRevision: 0n,
      droppedAnnounceCount: 0n,
    },
    bluetooth: {
      desiredEnabled: false,
      state: Bindings.LocalBluetoothState.Stopped.new(),
      peers: [],
    },
    lxmf: {
      state: Bindings.LxmfHealthState.Stopped,
      inboundOverflowCount: 0n,
      mailboxRevision: 0n,
      projectionRevision: 0n,
    },
    pairing: Bindings.RemoteControlPairingState.Searching.new(),
    pairingCandidates: [],
    pairedTargets: [],
    activeOperation: undefined,
    lastAnnouncement: undefined,
    lastRemoteChange: undefined,
    failure: undefined,
  };
}
const mockRuntime = {
  phase: "ready",
  stoppingNode: false,
  snapshot: snapshot(),
  exportControllerSetupCard: mockExport,
  previewTargetAccessCard: mockPreview,
  importTargetAccessCard: mockImport,
  describeTarget: mockDescribe,
  changeRemoteNode: mockChange,
};

jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => mockRuntime,
}));
jest.mock("expo-router", () => ({
  Link: ({ children, href }: { readonly children: ReactNode; readonly href: unknown }) => {
    mockLinks.push(href);
    return children;
  },
}));

function outcome<T>(value: T): RuntimeCommandResult<T> {
  return { type: "outcome", outcome: value };
}
function deferred<T>() {
  let complete: (value: T) => void = () => {};
  const promise = new Promise<T>((resolve) => {
    complete = resolve;
  });
  return { promise, complete: (value: T) => complete(value) };
}
async function review(view: ReturnType<typeof render>) {
  fireEvent.changeText(view.getByLabelText("Target access card"), cardText);
  fireEvent.press(view.getByRole("button", { name: "Review access card" }));
  await waitFor(() => expect(view.getByText("Source not verified")).toBeTruthy());
}
async function save(view: ReturnType<typeof render>) {
  await review(view);
  fireEvent.press(view.getByRole("button", { name: "Trust this source and save access" }));
  await waitFor(() => expect(view.getByText("Access saved locally")).toBeTruthy());
}

beforeEach(() => {
  jest.clearAllMocks();
  mockLinks.length = 0;
  mockRuntime.phase = "ready";
  mockRuntime.stoppingNode = false;
  mockRuntime.snapshot = snapshot();
  mockExport.mockResolvedValue(
    outcome(Bindings.ControllerSetupCardOutcome.Ready.new({ card: exported })),
  );
  mockPreview.mockResolvedValue(
    outcome(Bindings.TargetAccessCardPreviewOutcome.Valid.new({ preview })),
  );
  mockImport.mockResolvedValue(
    outcome(
      Bindings.TargetAccessCardImportOutcome.Saved.new({ targetIdentityFingerprint: target }),
    ),
  );
  mockDescribe.mockResolvedValue(outcome(Bindings.RemoteControlDescribeOutcome.Busy.new()));
});

test("exports public details on demand and saves only after explicit trust without checking or configuring", async () => {
  const view = render(<RemoteSetupCard />);
  expect(mockExport).not.toHaveBeenCalled();
  expect(mockPreview).not.toHaveBeenCalled();
  expect(mockImport).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Show my controller details" }));
  await waitFor(() => expect(view.getByText(exported.text)).toBeTruthy());
  expect(view.getByText(exported.publicKeyHex)).toBeTruthy();
  await review(view);
  expect(mockPreview).toHaveBeenCalledWith(cardText);
  expect(view.getByText("22222222222222222222222222222222")).toBeTruthy();
  expect(view.getByText("Operator")).toBeTruthy();
  expect(view.getByText("View node information")).toBeTruthy();
  expect(view.getByText("Change radio settings")).toBeTruthy();
  expect(mockImport).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Trust this source and save access" }));
  await waitFor(() => expect(view.getByText("Access saved locally")).toBeTruthy());
  expect(mockImport).toHaveBeenCalledWith({
    card: cardText,
    expectedControllerIdentity: controller,
    trustedSourceConfirmed: true,
  });
  expect(view.getByText(/Control has not been checked/u)).toBeTruthy();
  expect(view.getByRole("link", { name: "Keep current settings" })).toBeTruthy();
  expect(view.getByRole("link", { name: "Do this later" })).toBeTruthy();
  expect(mockLinks).toContainEqual({
    pathname: "/nodes/managed/[nodeId]",
    params: { nodeId: "22".repeat(16) },
  });
  expect(mockDescribe).not.toHaveBeenCalled();
  expect(mockChange).not.toHaveBeenCalled();
});

test("a card for another controller never offers trust or save", async () => {
  mockPreview.mockResolvedValue(
    outcome(
      Bindings.TargetAccessCardPreviewOutcome.Valid.new({
        preview: { ...preview, recipientControllerFingerprint: new Uint8Array(16).fill(0x33) },
      }),
    ),
  );
  const view = render(<RemoteSetupCard />);
  await review(view);
  expect(view.getByText(/This card is for a different controller/u)).toBeTruthy();
  expect(view.queryByRole("button", { name: "Trust this source and save access" })).toBeNull();
  expect(mockImport).not.toHaveBeenCalled();
});

test("editing a card discards both its preview and any late preview response", async () => {
  const pending = deferred<RuntimeCommandResult<Bindings.TargetAccessCardPreviewOutcome>>();
  mockPreview.mockReturnValueOnce(pending.promise);
  const view = render(<RemoteSetupCard />);
  fireEvent.changeText(view.getByLabelText("Target access card"), cardText);
  fireEvent.press(view.getByRole("button", { name: "Review access card" }));
  fireEvent.changeText(view.getByLabelText("Target access card"), "another card");
  await act(async () =>
    pending.complete(outcome(Bindings.TargetAccessCardPreviewOutcome.Valid.new({ preview }))),
  );
  expect(view.queryByText("Source not verified")).toBeNull();
  expect(view.queryByRole("button", { name: "Trust this source and save access" })).toBeNull();
  await review(view);
  fireEvent.changeText(view.getByLabelText("Target access card"), "changed again");
  expect(view.queryByRole("button", { name: "Trust this source and save access" })).toBeNull();
  expect(mockImport).not.toHaveBeenCalled();
});

test("a changed controller discards a pending export and the old import preview", async () => {
  const pending = deferred<RuntimeCommandResult<Bindings.ControllerSetupCardOutcome>>();
  mockExport.mockReturnValueOnce(pending.promise);
  const view = render(<RemoteSetupCard />);
  await review(view);
  fireEvent.press(view.getByRole("button", { name: "Show my controller details" }));
  mockRuntime.snapshot = {
    ...mockRuntime.snapshot,
    controllerIdentityFingerprint: new Uint8Array(16).fill(0x44),
  };
  view.rerender(<RemoteSetupCard />);
  await act(async () =>
    pending.complete(outcome(Bindings.ControllerSetupCardOutcome.Ready.new({ card: exported }))),
  );
  expect(view.queryByText(exported.text)).toBeNull();
  expect(view.queryByText("Source not verified")).toBeNull();
  expect(view.getByLabelText("Target access card").props.value).toBe("");
  expect(mockImport).not.toHaveBeenCalled();
});

test("a late import result cannot mark access saved in a new runtime generation", async () => {
  const pending = deferred<RuntimeCommandResult<Bindings.TargetAccessCardImportOutcome>>();
  mockImport.mockReturnValueOnce(pending.promise);
  const view = render(<RemoteSetupCard />);
  await review(view);
  const trust = view.getByRole("button", { name: "Trust this source and save access" });
  fireEvent.press(trust);
  fireEvent.press(trust);
  expect(mockImport).toHaveBeenCalledTimes(1);
  mockRuntime.snapshot = { ...mockRuntime.snapshot, generationId: 2n };
  view.rerender(<RemoteSetupCard />);
  await act(async () =>
    pending.complete(
      outcome(
        Bindings.TargetAccessCardImportOutcome.Saved.new({ targetIdentityFingerprint: target }),
      ),
    ),
  );
  expect(view.queryByText("Access saved locally")).toBeNull();
  expect(view.queryByRole("button", { name: "Check connection" })).toBeNull();
  expect(mockDescribe).not.toHaveBeenCalled();
});

test("uncertain admission remains unknown and does not resubmit", async () => {
  mockImport.mockResolvedValue({ type: "operationFailure", detail: "connection interrupted" });
  const view = render(<RemoteSetupCard />);
  await review(view);
  fireEvent.press(view.getByRole("button", { name: "Trust this source and save access" }));
  await waitFor(() => expect(view.getByText(/The save outcome is unknown/u)).toBeTruthy());
  expect(view.getByRole("button", { name: "Trust this source and save access" })).toBeDisabled();
  expect(view.getByRole("link", { name: "Check saved nodes" })).toBeTruthy();
  expect(view.queryByText("Access saved locally")).toBeNull();
  expect(mockImport).toHaveBeenCalledTimes(1);
});

test("a connection check is optional, authenticates the saved target, and never changes settings", async () => {
  const view = render(<RemoteSetupCard />);
  await save(view);
  mockDescribe.mockResolvedValue(
    outcome(
      Bindings.RemoteControlDescribeOutcome.Described.new({
        target: {
          targetIdentityFingerprint: target,
          controllerIdentityFingerprint: controller,
          destination: new Uint8Array(16),
          permittedRequests: preview.permissions,
        },
        availableRequests: preview.permissions,
        rttMillis: 3n,
        snapshot: mockRuntime.snapshot,
      }),
    ),
  );
  expect(mockDescribe).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Check connection" }));
  await waitFor(() => expect(view.getByText(/Connection checked\./u)).toBeTruthy());
  expect(mockDescribe).toHaveBeenCalledWith({ targetIdentityFingerprint: target });
  expect(view.queryByText(/Control has not been checked/u)).toBeNull();
  expect(mockChange).not.toHaveBeenCalled();
});

test("a conflicting saved target is not silently replaced", async () => {
  mockImport.mockResolvedValue(outcome(Bindings.TargetAccessCardImportOutcome.Conflict.new()));
  const view = render(<RemoteSetupCard />);
  await review(view);
  fireEvent.press(view.getByRole("button", { name: "Trust this source and save access" }));
  await waitFor(() => expect(view.getByText(/it has not been replaced/u)).toBeTruthy());
  expect(view.queryByText("Access saved locally")).toBeNull();
  expect(mockImport).toHaveBeenCalledTimes(1);
});
