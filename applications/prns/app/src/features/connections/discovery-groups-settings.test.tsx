import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render } from "@testing-library/react-native";
import { StrictMode } from "react";

import type { DevelopmentRuntimeView } from "@/native/development-runtime-context";
import { DiscoveryGroupsSettings } from "./discovery-groups-settings";

jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => mockRuntime,
}));

type Settings = Bindings.LocalBluetoothDiscoveryGroupsSettings;
type Result = Awaited<ReturnType<DevelopmentRuntimeView["readBluetoothDiscoveryGroups"]>>;
const ready = (overrides: Partial<Settings> = {}): Result => ({
  type: "outcome",
  outcome: Bindings.LocalBluetoothDiscoveryGroupsOutcome.Ready.new({
    settings: {
      ownerId: 7n,
      generationId: 2n,
      groups: ["reticulum"],
      appliedGroups: ["reticulum"],
      ...overrides,
    },
  }),
});
const read = jest.fn<Promise<Result>, []>();
const write = jest.fn<Promise<Result>, [Bindings.SetBluetoothDiscoveryGroupsInput]>();
const snapshot = (
  generationId = 2n,
  runtime = Bindings.DevelopmentNodeRuntime.Running,
): NonNullable<DevelopmentRuntimeView["snapshot"]> =>
  ({
    generationId,
    runtime,
    bluetooth: { desiredEnabled: true, state: Bindings.LocalBluetoothState.Connected.new() },
  }) as NonNullable<DevelopmentRuntimeView["snapshot"]>;
let mockRuntime: Pick<
  DevelopmentRuntimeView,
  | "availability"
  | "phase"
  | "stoppingNode"
  | "snapshot"
  | "readBluetoothDiscoveryGroups"
  | "setBluetoothDiscoveryGroups"
>;

beforeEach(() => {
  jest.resetAllMocks();
  read.mockResolvedValue(ready());
  write.mockResolvedValue(ready());
  mockRuntime = {
    availability: { type: "available", platform: "android" },
    phase: "ready",
    stoppingNode: false,
    snapshot: snapshot(),
    readBluetoothDiscoveryGroups: read,
    setBluetoothDiscoveryGroups: write,
  };
});

async function open() {
  const view = render(<DiscoveryGroupsSettings />);
  await act(async () =>
    fireEvent.press(view.getByRole("button", { name: "Change discovery groups" })),
  );
  return view;
}

test("advanced discovery settings are collapsed and do not perform background reads", async () => {
  const view = render(<DiscoveryGroupsSettings />);
  expect(read).not.toHaveBeenCalled();
  expect(view.queryByLabelText("Discovery groups (one per line)")).toBeNull();
  await act(async () =>
    fireEvent.press(view.getByRole("button", { name: "Change discovery groups" })),
  );
  expect(read).toHaveBeenCalledTimes(1);
  expect(view.getByDisplayValue("reticulum")).toBeTruthy();
  expect(
    view.getByText("Saved and applied to Bluetooth. This does not confirm a connection."),
  ).toBeTruthy();
});

test("confirmation freezes names and owner fences, then saves only once while pending", async () => {
  let complete: ((result: Result) => void) | undefined;
  write.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const view = await open();
  fireEvent.changeText(view.getByLabelText("Discovery groups (one per line)"), "camp-a\n trail ");
  fireEvent.press(view.getByRole("button", { name: "Save discovery groups" }));
  expect(write).not.toHaveBeenCalled();
  expect(view.getByText(/disconnects nearby devices/)).toBeTruthy();
  expect(view.getByLabelText("Discovery groups (one per line)").props.editable).toBe(false);
  await act(async () => {
    const confirm = view.getByRole("button", { name: "Save and reconnect" });
    fireEvent.press(confirm);
    fireEvent.press(confirm);
  });
  expect(write).toHaveBeenCalledTimes(1);
  expect(write).toHaveBeenCalledWith({
    ownerId: 7n,
    generationId: 2n,
    groups: ["camp-a", " trail "],
  });
  await act(async () =>
    complete?.(ready({ groups: ["camp-a", " trail "], appliedGroups: ["reticulum"] })),
  );
  expect(
    view.getByText("Saved. Application to Bluetooth is not yet confirmed; refresh to check."),
  ).toBeTruthy();
  expect(
    view.queryByText("Saved and applied to Bluetooth. This does not confirm a connection."),
  ).toBeNull();
});

test("restore default is explicit and remains usable with Bluetooth stopped", async () => {
  mockRuntime = {
    ...mockRuntime,
    snapshot: snapshot(2n, Bindings.DevelopmentNodeRuntime.Stopped),
  };
  read.mockResolvedValue(ready({ groups: ["camp-a"], appliedGroups: undefined }));
  write.mockResolvedValue(ready({ appliedGroups: undefined }));
  const view = await open();
  expect(view.getByText("Saved for the next Bluetooth start.")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Restore default discovery group" }));
  expect(write).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Cancel" }));
  expect(write).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Restore default discovery group" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(write).toHaveBeenCalledWith({ ownerId: 7n, generationId: 2n, groups: ["reticulum"] });
  expect(view.getByDisplayValue("reticulum")).toBeTruthy();
});

test("uncertain save rereads durable state and never claims that unchanged live groups were applied", async () => {
  read
    .mockResolvedValueOnce(ready())
    .mockResolvedValueOnce(ready({ groups: ["camp-a"], appliedGroups: ["reticulum"] }));
  write.mockResolvedValue({ type: "operationFailure", detail: "timed out" });
  const view = await open();
  fireEvent.changeText(view.getByLabelText("Discovery groups (one per line)"), "camp-a");
  fireEvent.press(view.getByRole("button", { name: "Save discovery groups" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(read).toHaveBeenCalledTimes(2);
  expect(view.getByText(/saved groups below have been checked/)).toBeTruthy();
  expect(
    view.getByText("Saved. Application to Bluetooth is not yet confirmed; refresh to check."),
  ).toBeTruthy();
  expect(write).toHaveBeenCalledTimes(1);
});

test("failed reconciliation disables another save until a fresh read and preserves the draft", async () => {
  read
    .mockResolvedValueOnce(ready())
    .mockResolvedValueOnce({ type: "operationFailure", detail: "offline" });
  write.mockResolvedValue({ type: "operationFailure", detail: "timed out" });
  const view = await open();
  fireEvent.changeText(view.getByLabelText("Discovery groups (one per line)"), "camp-a");
  fireEvent.press(view.getByRole("button", { name: "Save discovery groups" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(view.getByRole("button", { name: "Save discovery groups" })).toBeDisabled();
  expect(view.getByDisplayValue("camp-a")).toBeTruthy();
  await act(async () =>
    fireEvent.press(view.getByRole("button", { name: "Refresh discovery groups" })),
  );
  expect(view.getByRole("button", { name: "Save discovery groups" })).toBeEnabled();
  expect(view.getByDisplayValue("camp-a")).toBeTruthy();
});

test("a late settings read cannot overwrite a replacement node generation", async () => {
  let complete: ((result: Result) => void) | undefined;
  read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const view = await open();
  read.mockResolvedValue(
    ready({ generationId: 3n, groups: ["new-owner"], appliedGroups: undefined }),
  );
  mockRuntime = { ...mockRuntime, snapshot: snapshot(3n) };
  await act(async () => view.rerender(<DiscoveryGroupsSettings />));
  await act(async () => complete?.(ready({ groups: ["stale-owner"] })));
  expect(view.getByDisplayValue("new-owner")).toBeTruthy();
  expect(view.queryByDisplayValue("stale-owner")).toBeNull();
});

test("Strict Mode effect replay does not leave the settings read busy or publish its retired result", async () => {
  let complete: ((result: Result) => void) | undefined;
  read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const view = render(
    <StrictMode>
      <DiscoveryGroupsSettings />
    </StrictMode>,
  );
  await act(async () =>
    fireEvent.press(view.getByRole("button", { name: "Change discovery groups" })),
  );
  expect(read).toHaveBeenCalledTimes(2);
  expect(view.getByRole("button", { name: "Save discovery groups" })).toBeEnabled();
  await act(async () => complete?.(ready({ groups: ["retired-read"] })));
  expect(view.getByDisplayValue("reticulum")).toBeTruthy();
  expect(view.queryByDisplayValue("retired-read")).toBeNull();
});

test("invalid native validation feedback keeps the edited names and sends no follow-up mutation", async () => {
  write.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.LocalBluetoothDiscoveryGroupsOutcome.Invalid.new({
      detail: "Discovery groups must be distinct.",
    }),
  });
  const view = await open();
  fireEvent.changeText(view.getByLabelText("Discovery groups (one per line)"), "camp\ncamp");
  fireEvent.press(view.getByRole("button", { name: "Save discovery groups" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(view.getByText("Discovery groups must be distinct.")).toBeTruthy();
  expect(view.getByDisplayValue("camp\ncamp")).toBeTruthy();
  expect(read).toHaveBeenCalledTimes(1);
  expect(write).toHaveBeenCalledTimes(1);
});

test("reopening settings reloads the native preference without replaying a write", async () => {
  const view = await open();
  fireEvent.press(view.getByRole("button", { name: "Hide discovery groups" }));
  read.mockResolvedValue(ready({ groups: ["camp-a"], appliedGroups: undefined }));
  await act(async () =>
    fireEvent.press(view.getByRole("button", { name: "Change discovery groups" })),
  );
  expect(view.getByDisplayValue("camp-a")).toBeTruthy();
  expect(read).toHaveBeenCalledTimes(2);
  expect(write).not.toHaveBeenCalled();
});

test("a changed storage owner is reread and the next confirmed change uses its new token", async () => {
  read.mockResolvedValueOnce(ready()).mockResolvedValueOnce(ready({ ownerId: 9n }));
  write.mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.LocalBluetoothDiscoveryGroupsOutcome.GenerationChanged.new(),
  });
  const view = await open();
  fireEvent.changeText(view.getByLabelText("Discovery groups (one per line)"), "camp-a");
  fireEvent.press(view.getByRole("button", { name: "Save discovery groups" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(write).toHaveBeenCalledTimes(1);
  fireEvent.press(view.getByRole("button", { name: "Save discovery groups" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(write).toHaveBeenLastCalledWith({ ownerId: 9n, generationId: 2n, groups: ["camp-a"] });
});

test.each([
  ["disabled", Bindings.DevelopmentNodeRuntime.Running, false],
  ["stopped", Bindings.DevelopmentNodeRuntime.Stopped, true],
  ["starting", Bindings.DevelopmentNodeRuntime.Starting, true],
  ["failed", Bindings.DevelopmentNodeRuntime.Failed, true],
] as const)(
  "%s Bluetooth invalidates a cached applied claim and reloads settings",
  async (_, state, enabled) => {
    const view = await open();
    expect(
      view.getByText("Saved and applied to Bluetooth. This does not confirm a connection."),
    ).toBeTruthy();
    const next = snapshot(2n, state);
    mockRuntime = {
      ...mockRuntime,
      snapshot: { ...next, bluetooth: { ...next.bluetooth, desiredEnabled: enabled } },
    };
    await act(async () => view.rerender(<DiscoveryGroupsSettings />));
    expect(read).toHaveBeenCalledTimes(2);
    expect(
      view.queryByText("Saved and applied to Bluetooth. This does not confirm a connection."),
    ).toBeNull();
    expect(view.getByText("Not confirmed")).toBeTruthy();
  },
);

test("invalid saved settings offer only an explicit fenced default repair, never fabricated defaults", async () => {
  read.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.LocalBluetoothDiscoveryGroupsOutcome.InvalidSaved.new({
      ownerId: 13n,
      generationId: 2n,
      detail: "Saved discovery groups are damaged.",
    }),
  });
  const view = await open();
  expect(view.getByText("Saved discovery groups are damaged.")).toBeTruthy();
  expect(view.queryByDisplayValue("reticulum")).toBeNull();
  expect(view.getByRole("button", { name: "Save discovery groups" })).toBeDisabled();
  expect(view.getByLabelText("Discovery groups (one per line)").props.editable).toBe(false);
  fireEvent.press(view.getByRole("button", { name: "Restore default discovery group" }));
  expect(write).not.toHaveBeenCalled();
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(write).toHaveBeenCalledWith({ ownerId: 13n, generationId: 2n, groups: ["reticulum"] });
  expect(view.getByDisplayValue("reticulum")).toBeTruthy();
  expect(view.queryByText("Saved discovery groups are damaged.")).toBeNull();
});

test("an expired repair fence is reread and requires a fresh explicit confirmation", async () => {
  const invalid = (ownerId: bigint, generationId: bigint): Result => ({
    type: "outcome",
    outcome: Bindings.LocalBluetoothDiscoveryGroupsOutcome.InvalidSaved.new({
      ownerId,
      generationId,
      detail: "Saved discovery groups are damaged.",
    }),
  });
  read.mockResolvedValueOnce(invalid(13n, 2n)).mockResolvedValueOnce(invalid(14n, 3n));
  write.mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.LocalBluetoothDiscoveryGroupsOutcome.GenerationChanged.new(),
  });
  const view = await open();
  fireEvent.press(view.getByRole("button", { name: "Restore default discovery group" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(write).toHaveBeenCalledTimes(1);
  expect(read).toHaveBeenCalledTimes(2);
  expect(view.queryByRole("button", { name: "Save and reconnect" })).toBeNull();
  fireEvent.press(view.getByRole("button", { name: "Restore default discovery group" }));
  await act(async () => fireEvent.press(view.getByRole("button", { name: "Save and reconnect" })));
  expect(write).toHaveBeenLastCalledWith({ ownerId: 14n, generationId: 3n, groups: ["reticulum"] });
});
