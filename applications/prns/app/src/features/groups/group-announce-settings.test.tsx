import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";

import type { RuntimeCommandResult } from "@/native/development-runtime-context";
import { GroupAnnounceSettings } from "./group-announce-settings";

let mockSnapshot = { generationId: 1n, runtime: Bindings.DevelopmentNodeRuntime.Running };
const mockGet = jest.fn<Promise<RuntimeCommandResult<Bindings.GroupAnnounceSettingsOutcome>>, []>();
const mockSet = jest.fn<
  Promise<RuntimeCommandResult<Bindings.GroupAnnounceSettingsOutcome>>,
  [Bindings.GroupAnnounceSettings]
>();
const mockAnnounce = jest.fn<Promise<RuntimeCommandResult<Bindings.GroupAnnounceOutcome>>, []>();
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({
    availability: { type: "available", platform: "ios" },
    phase: "ready",
    snapshot: mockSnapshot,
    getGroupAnnounceSettings: mockGet,
    setGroupAnnounceSettings: mockSet,
    announceGroups: mockAnnounce,
  }),
}));

const preference = (
  intervalMinutes: number | undefined,
): RuntimeCommandResult<Bindings.GroupAnnounceSettingsOutcome> => ({
  type: "outcome",
  outcome: Bindings.GroupAnnounceSettingsOutcome.Ready.new({ settings: { intervalMinutes } }),
});

beforeEach(() => {
  jest.clearAllMocks();
  mockSnapshot = { generationId: 1n, runtime: Bindings.DevelopmentNodeRuntime.Running };
  mockGet.mockResolvedValue(preference(60));
  mockSet.mockImplementation(async (settings) => preference(settings.intervalMinutes));
  mockAnnounce.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupAnnounceOutcome.Requested.new(),
  });
});

test("loads persisted interval, saves manual only, and keeps manual announcement independent", async () => {
  const view = render(<GroupAnnounceSettings />);
  await waitFor(() => expect(view.getByText("Automatic announcements: Every hour.")).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Change announce interval" }));
  expect(view.getByText(/Short intervals use more shared radio bandwidth/)).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Manual only" }));
  await waitFor(() => expect(mockSet).toHaveBeenCalledWith({ intervalMinutes: undefined }));
  await waitFor(() => expect(view.getByText("Automatic announcements: Manual only.")).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Announce groups now" }));
  await waitFor(() =>
    expect(view.getByText(/does not confirm that another device received them/)).toBeTruthy(),
  );
  expect(mockAnnounce).toHaveBeenCalledTimes(1);
});

test("reports native cooldown without scheduling automatic retries", async () => {
  mockAnnounce.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupAnnounceOutcome.Throttled.new({ retryAfterMillis: 59_001n }),
  });
  const view = render(<GroupAnnounceSettings />);
  await waitFor(() => expect(view.getByText("Automatic announcements: Every hour.")).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Announce groups now" }));
  await waitFor(() =>
    expect(view.getByText("Recently announced. Try again in 60 seconds.")).toBeTruthy(),
  );
  expect(mockAnnounce).toHaveBeenCalledTimes(1);
});

test("does not project an old generation's late setting result", async () => {
  let finish!: (value: RuntimeCommandResult<Bindings.GroupAnnounceSettingsOutcome>) => void;
  mockSet.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(<GroupAnnounceSettings />);
  await waitFor(() => expect(view.getByText("Automatic announcements: Every hour.")).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Change announce interval" }));
  fireEvent.press(view.getByRole("button", { name: "Every 15 minutes" }));
  mockSnapshot = { generationId: 2n, runtime: Bindings.DevelopmentNodeRuntime.Stopped };
  view.rerender(<GroupAnnounceSettings />);
  await waitFor(() => expect(mockGet).toHaveBeenCalledTimes(2));
  await act(async () => finish(preference(15)));
  expect(view.getByText("Automatic announcements: Every hour.")).toBeTruthy();
  expect(view.getByRole("button", { name: "Announce groups now" })).toBeDisabled();
});
