import { type DevelopmentNodeSnapshot, DevelopmentNodeRuntime } from "@prns-internal/expo";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import { Share } from "react-native";
import { AboutScreen } from "./about-screen";

const mockRefresh = jest.fn();
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({ refreshSnapshot: mockRefresh }),
}));
jest.mock("@/native/runtime-provider", () => ({
  runtimeProvider: { availability: { type: "available", platform: "ios" } },
}));
jest.mock("expo-constants", () => ({
  __esModule: true,
  default: {
    expoConfig: {
      name: "prns",
      slug: "prns",
      version: "0.1.0",
      ios: { buildNumber: "9" },
      extra: { prnsBuild: { variant: "production", label: "alpha-9" } },
    },
  },
}));

const snapshot = {
  runtime: DevelopmentNodeRuntime.Running,
  bluetooth: { state: { tag: "WaitingForPeers" }, desiredEnabled: true, peers: [] },
  network: { state: { tag: "Ready" }, routes: [] },
} as unknown as DevelopmentNodeSnapshot;

beforeEach(() => {
  jest.restoreAllMocks();
  mockRefresh.mockReset().mockResolvedValue({ type: "outcome", outcome: snapshot });
});

test("technical details are optional and reading them never shares automatically", async () => {
  const share = jest.spyOn(Share, "share").mockResolvedValue({ action: Share.sharedAction });
  const view = render(<AboutScreen />);
  expect(view.queryByText("Configured build")).toBeNull();
  expect(mockRefresh).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Build details" }));
  expect(view.getByText("alpha-9")).toBeTruthy();
  expect(view.getByText("9")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: "Diagnostics" }));
  await waitFor(() => expect(view.getByText(/Node: Running/u)).toBeTruthy());
  expect(share).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Share diagnostics" }));
  await waitFor(() => expect(share).toHaveBeenCalledTimes(1));
  expect(share.mock.calls[0]?.[0]).toMatchObject({
    title: "prns diagnostics",
    message: expect.stringContaining("Build label: alpha-9"),
  });
});

test("share cancellation is quiet and failures remain actionable without exposing exception text", async () => {
  const share = jest
    .spyOn(Share, "share")
    .mockResolvedValueOnce({ action: Share.dismissedAction })
    .mockRejectedValueOnce(new Error("private file address"));
  const view = render(<AboutScreen />);
  fireEvent.press(view.getByRole("button", { name: "Diagnostics" }));
  await waitFor(() => expect(view.getByText(/Node: Running/u)).toBeTruthy());
  fireEvent.press(view.getByRole("button", { name: "Share diagnostics" }));
  await waitFor(() =>
    expect(view.getByRole("button", { name: "Share diagnostics" })).not.toBeDisabled(),
  );
  expect(view.queryByText(/could not be shared/u)).toBeNull();
  fireEvent.press(view.getByRole("button", { name: "Share diagnostics" }));
  await waitFor(() => expect(view.getByText(/Diagnostics could not be shared/u)).toBeTruthy());
  expect(view.queryByText(/private file address/u)).toBeNull();
  expect(share).toHaveBeenCalledTimes(2);
});

test("an unavailable runtime still permits an explicitly unavailable build report", async () => {
  mockRefresh.mockResolvedValue({ type: "operationFailure", detail: "private identity path" });
  const view = render(<AboutScreen />);
  fireEvent.press(view.getByRole("button", { name: "Diagnostics" }));
  await waitFor(() => expect(view.getByText(/Runtime snapshot: Unavailable/u)).toBeTruthy());
  expect(view.queryByText(/private identity path/u)).toBeNull();
  expect(view.getByRole("button", { name: "Share diagnostics" })).not.toBeDisabled();
});

test("closing diagnostics while a snapshot is loading neither shares nor updates a later report", async () => {
  let resolve: (value: unknown) => void = () => {};
  mockRefresh.mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    }),
  );
  const share = jest.spyOn(Share, "share");
  const view = render(<AboutScreen />);
  fireEvent.press(view.getByRole("button", { name: "Diagnostics" }));
  expect(view.getByRole("button", { name: "Share diagnostics" })).toBeDisabled();
  fireEvent.press(view.getByRole("button", { name: "Hide diagnostics" }));
  await act(async () => resolve({ type: "outcome", outcome: snapshot }));
  expect(view.queryByText(/Node: Running/u)).toBeNull();
  expect(share).not.toHaveBeenCalled();
});
