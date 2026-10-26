import * as Bindings from "@prns-internal/expo";
import { act, renderHook, waitFor } from "@testing-library/react-native";
import { AppState, type AppStateStatus } from "react-native";
import { useVisibleIdentityActivity } from "./use-visible-identity-activity";

let mockFocused = true;
const mockRefresh = jest.fn(async () => {});
let mockOwner: {
  ownerKey: string;
  runtime: Bindings.DevelopmentRuntime;
  refreshActivity: typeof mockRefresh;
};
jest.mock("./named-identity-context", () => ({ useNamedIdentity: () => mockOwner }));
jest.mock("expo-router", () => ({
  useFocusEffect: (effect: () => undefined | (() => void)) => {
    const { useEffect } = jest.requireActual<typeof import("react")>("react");
    useEffect(() => (mockFocused ? effect() : undefined), [effect, mockFocused]);
  },
}));
const group = Bindings.IdentityActivityScope.Group.new({ groupId: new Uint8Array(32).fill(3) });
function client() {
  const markIdentityActivitySeen = jest
    .fn<Promise<Bindings.IdentityActivitySeenOutcome>, [Bindings.MarkIdentityActivitySeenInput]>()
    .mockResolvedValue(Bindings.IdentityActivitySeenOutcome.Saved.new());
  return {
    runtime: { markIdentityActivitySeen } as unknown as Bindings.DevelopmentRuntime,
    markIdentityActivitySeen,
  };
}
const originalAppState = Object.getOwnPropertyDescriptor(AppState, "currentState");
beforeEach(() => {
  mockFocused = true;
  mockRefresh.mockClear();
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "active" });
});
afterAll(() => {
  if (originalAppState) Object.defineProperty(AppState, "currentState", originalAppState);
});
afterEach(() => {
  jest.restoreAllMocks();
  jest.useRealTimers();
});

test("only a focused foreground detail acknowledges its observed watermark, once", async () => {
  const f = client();
  mockOwner = { ownerKey: "personal", runtime: f.runtime, refreshActivity: mockRefresh };
  mockFocused = false;
  const hook = renderHook(
    ({
      sequence,
      scope,
    }: {
      sequence: bigint | undefined;
      scope: Bindings.IdentityActivityScope | null;
    }) => useVisibleIdentityActivity(scope, sequence),
    {
      initialProps: {
        sequence: 5n as bigint | undefined,
        scope: group as Bindings.IdentityActivityScope | null,
      },
    },
  );
  expect(f.markIdentityActivitySeen).not.toHaveBeenCalled();
  mockFocused = true;
  hook.rerender({ sequence: undefined, scope: null });
  expect(f.markIdentityActivitySeen).not.toHaveBeenCalled();
  hook.rerender({ sequence: 5n, scope: group });
  await waitFor(() => expect(mockRefresh).toHaveBeenCalledTimes(1));
  expect(f.markIdentityActivitySeen).toHaveBeenCalledWith({ scope: group, throughSequence: 5n });
  hook.rerender({
    sequence: 5n,
    scope: Bindings.IdentityActivityScope.Group.new({ groupId: new Uint8Array(32).fill(3) }),
  });
  expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(1);
  hook.rerender({ sequence: 6n, scope: group });
  await waitFor(() => expect(mockRefresh).toHaveBeenCalledTimes(2));
  expect(f.markIdentityActivitySeen).toHaveBeenLastCalledWith({
    scope: group,
    throughSequence: 6n,
  });
});

test("background arrivals stay unread until the conversation is visible again", async () => {
  const f = client();
  mockOwner = { ownerKey: "personal", runtime: f.runtime, refreshActivity: mockRefresh };
  let change!: (state: AppStateStatus) => void;
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
    change = listener;
    return { remove: jest.fn() };
  });
  const hook = renderHook(
    ({ sequence }: { sequence: bigint }) => useVisibleIdentityActivity(group, sequence),
    {
      initialProps: { sequence: 5n },
    },
  );
  await waitFor(() => expect(mockRefresh).toHaveBeenCalledTimes(1));
  act(() => change("background"));
  hook.rerender({ sequence: 8n });
  expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(1);
  act(() => change("active"));
  await waitFor(() => expect(mockRefresh).toHaveBeenCalledTimes(2));
  expect(f.markIdentityActivitySeen).toHaveBeenLastCalledWith({
    scope: group,
    throughSequence: 8n,
  });
});

test("a late acknowledgment remains bound to its old identity and cannot clear the new owner's watermark", async () => {
  const personal = client();
  const work = client();
  let finish!: (result: Bindings.IdentityActivitySeenOutcome) => void;
  personal.markIdentityActivitySeen.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  mockOwner = { ownerKey: "personal", runtime: personal.runtime, refreshActivity: mockRefresh };
  const hook = renderHook(
    ({ sequence }: { sequence: bigint }) => useVisibleIdentityActivity(group, sequence),
    {
      initialProps: { sequence: 5n },
    },
  );
  await waitFor(() => expect(personal.markIdentityActivitySeen).toHaveBeenCalledTimes(1));
  mockOwner = { ownerKey: "work", runtime: work.runtime, refreshActivity: mockRefresh };
  hook.rerender({ sequence: 9n });
  await act(async () => finish(Bindings.IdentityActivitySeenOutcome.Saved.new()));
  await waitFor(() => expect(work.markIdentityActivitySeen).toHaveBeenCalledTimes(1));
  expect(personal.markIdentityActivitySeen).toHaveBeenCalledWith({
    scope: group,
    throughSequence: 5n,
  });
  expect(work.markIdentityActivitySeen).toHaveBeenCalledWith({ scope: group, throughSequence: 9n });
  expect(mockRefresh).toHaveBeenCalledTimes(1);
});

test("newly displayed records advance after a pending older acknowledgment without clearing unseen future records", async () => {
  const f = client();
  let finish!: (result: Bindings.IdentityActivitySeenOutcome) => void;
  f.markIdentityActivitySeen.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  mockOwner = { ownerKey: "personal", runtime: f.runtime, refreshActivity: mockRefresh };
  const hook = renderHook(
    ({ sequence }: { sequence: bigint }) => useVisibleIdentityActivity(group, sequence),
    {
      initialProps: { sequence: 5n },
    },
  );
  hook.rerender({ sequence: 7n });
  await act(async () => finish(Bindings.IdentityActivitySeenOutcome.Saved.new()));
  await waitFor(() => expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(2));
  expect(f.markIdentityActivitySeen.mock.calls.map(([input]) => input.throughSequence)).toEqual([
    5n,
    7n,
  ]);
});

test("failed acknowledgments retry after delay and stop retrying while unfocused", async () => {
  jest.useFakeTimers();
  const f = client();
  f.markIdentityActivitySeen.mockRejectedValueOnce(new Error("Busy"));
  mockOwner = { ownerKey: "personal", runtime: f.runtime, refreshActivity: mockRefresh };
  const hook = renderHook(() => useVisibleIdentityActivity(group, 5n));
  await act(async () => {});
  expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(1);
  mockFocused = false;
  hook.rerender({});
  await act(async () => jest.advanceTimersByTime(5000));
  expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(1);
  mockFocused = true;
  hook.rerender({});
  await act(async () => {});
  expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(2);
  expect(mockRefresh).toHaveBeenCalledTimes(1);
});

test("unavailable storage retries while the same conversation remains visible", async () => {
  jest.useFakeTimers();
  const f = client();
  f.markIdentityActivitySeen.mockResolvedValueOnce(
    Bindings.IdentityActivitySeenOutcome.Unavailable.new({ detail: "Busy" }),
  );
  mockOwner = { ownerKey: "personal", runtime: f.runtime, refreshActivity: mockRefresh };
  renderHook(() => useVisibleIdentityActivity(group, 5n));
  await act(async () => {});
  await act(async () => jest.advanceTimersByTime(5000));
  expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(2);
  expect(mockRefresh).toHaveBeenCalledTimes(1);
});

test.each([
  ["capacity", Bindings.IdentityActivitySeenOutcome.Capacity.new()],
  ["invalid input", Bindings.IdentityActivitySeenOutcome.InvalidInput.new({ detail: "Invalid" })],
])(
  "%s waits for a new cursor or refocus instead of retrying on a timer",
  async (_label, result) => {
    jest.useFakeTimers();
    const f = client();
    f.markIdentityActivitySeen.mockResolvedValue(result as Bindings.IdentityActivitySeenOutcome);
    mockOwner = { ownerKey: "personal", runtime: f.runtime, refreshActivity: mockRefresh };
    const hook = renderHook(
      ({ sequence }: { sequence: bigint }) => useVisibleIdentityActivity(group, sequence),
      { initialProps: { sequence: 5n } },
    );
    await act(async () => {});
    await act(async () => jest.advanceTimersByTime(15000));
    expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(1);
    hook.rerender({ sequence: 6n });
    await act(async () => {});
    expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(2);
    mockFocused = false;
    hook.rerender({ sequence: 6n });
    mockFocused = true;
    hook.rerender({ sequence: 6n });
    await act(async () => {});
    expect(f.markIdentityActivitySeen).toHaveBeenCalledTimes(3);
    expect(mockRefresh).not.toHaveBeenCalled();
  },
);
