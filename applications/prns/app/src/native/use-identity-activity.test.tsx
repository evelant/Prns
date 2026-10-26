import * as Bindings from "@prns-internal/expo";
import { act, renderHook, waitFor } from "@testing-library/react-native";
import { type PropsWithChildren, StrictMode } from "react";
import { AppState, type AppStateStatus } from "react-native";
import { useIdentityActivity } from "./use-identity-activity";

const identity = {
  incarnation: new Uint8Array(16).fill(7),
  sequence: 2n,
  publicHash: new Uint8Array(16).fill(2),
};
const unavailableOwner = () =>
  Bindings.NamedIdentityActivityListOutcome.Ready.new({
    activities: [
      {
        identity,
        activity: Bindings.IdentityActivityOutcome.Unavailable.new({ detail: "Owner is busy" }),
      },
    ],
  });
const originalAppState = Object.getOwnPropertyDescriptor(AppState, "currentState");
beforeEach(() => {
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "active" });
});
afterEach(() => {
  jest.restoreAllMocks();
  jest.useRealTimers();
});
afterAll(() => {
  if (originalAppState) Object.defineProperty(AppState, "currentState", originalAppState);
});
const ready = (unread: number) =>
  Bindings.NamedIdentityActivityListOutcome.Ready.new({
    activities: [
      {
        identity,
        activity: Bindings.IdentityActivityOutcome.Ready.new({
          summary: {
            actionableInvitations: 1,
            unreadGroups: unread,
            unreadDirect: 0,
            conversations: [],
            partial: false,
          },
        }),
      },
    ],
  });
function fixture() {
  const listNamedIdentityActivity = jest
    .fn<Promise<Bindings.NamedIdentityActivityListOutcome>, []>()
    .mockResolvedValue(ready(2));
  const markIdentityActivitySeen = jest.fn();
  const runtime = {
    listNamedIdentityActivity,
    markIdentityActivitySeen,
  } as unknown as Bindings.DevelopmentRuntime;
  return { runtime, listNamedIdentityActivity, markIdentityActivitySeen };
}

test("activity refresh reports all owners without marking any messages seen", async () => {
  const f = fixture();
  const hook = renderHook(() => useIdentityActivity(f.runtime, 1n));
  await waitFor(() => expect(hook.result.current.activity).toHaveLength(1));
  expect(hook.result.current.activity?.[0]?.identity).toBe(identity);
  await act(async () => hook.result.current.refreshActivity());
  expect(f.markIdentityActivitySeen).not.toHaveBeenCalled();
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(2);
});

test("a late summary from a previous host generation cannot replace current activity", async () => {
  const f = fixture();
  let finish!: (value: Bindings.NamedIdentityActivityListOutcome) => void;
  f.listNamedIdentityActivity.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const hook = renderHook(
    ({ generation }: { generation: bigint }) => useIdentityActivity(f.runtime, generation),
    {
      initialProps: { generation: 1n },
    },
  );
  hook.rerender({ generation: 2n });
  await waitFor(() => expect(hook.result.current.activity).toHaveLength(1));
  const current = hook.result.current.activity;
  await act(async () => finish(ready(99)));
  expect(hook.result.current.activity).toBe(current);
});

test("busy or failed activity is unknown rather than a fabricated zero", async () => {
  const f = fixture();
  const hook = renderHook(() => useIdentityActivity(f.runtime, 1n));
  await waitFor(() => expect(hook.result.current.activity).toHaveLength(1));
  f.listNamedIdentityActivity.mockResolvedValueOnce(
    Bindings.NamedIdentityActivityListOutcome.Busy.new(),
  );
  await act(async () => hook.result.current.refreshActivity());
  expect(hook.result.current.activity).toBeNull();
  expect(hook.result.current.activityFailure).toContain("busy");
  f.listNamedIdentityActivity.mockRejectedValueOnce(new Error("Storage unavailable"));
  await act(async () => hook.result.current.refreshActivity());
  expect(hook.result.current.activityFailure).toBe("Activity could not be loaded.");
});

test("refreshes coalesce while an activity read is pending", async () => {
  const f = fixture();
  let finish!: (value: Bindings.NamedIdentityActivityListOutcome) => void;
  f.listNamedIdentityActivity.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const hook = renderHook(() => useIdentityActivity(f.runtime, 1n));
  await act(async () => {
    await hook.result.current.refreshActivity();
    await hook.result.current.refreshActivity();
  });
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(1);
  await act(async () => finish(ready(1)));
  expect(hook.result.current.activity).toHaveLength(1);
});

test("strict effect replay does not strand the initial activity read", async () => {
  const f = fixture();
  const hook = renderHook(() => useIdentityActivity(f.runtime, 1n), {
    wrapper: ({ children }: PropsWithChildren) => <StrictMode>{children}</StrictMode>,
  });
  await waitFor(() => expect(hook.result.current.activity).toHaveLength(1));
});

test("one offset read recovers repeated three-second Inbox poll collisions without showing stale counts", async () => {
  jest.useFakeTimers();
  jest.setSystemTime(0);
  const f = fixture();
  const times: number[] = [];
  f.listNamedIdentityActivity.mockImplementation(async () => {
    times.push(Date.now());
    return Date.now() % 3000 === 0 ? unavailableOwner() : ready(2);
  });
  const hook = renderHook(() => useIdentityActivity(f.runtime, 1n));
  await act(async () => {});
  for (let cycle = 0; cycle < 3; cycle++) {
    expect(hook.result.current.activity?.[0]?.activity.tag).toBe("Unavailable");
    await act(async () => jest.advanceTimersByTime(400));
    expect(hook.result.current.activity?.[0]?.activity.tag).toBe("Ready");
    if (cycle < 2) await act(async () => jest.advanceTimersByTime(2600));
  }
  expect(times).toEqual([0, 400, 3000, 3400, 6000, 6400]);
  expect(f.markIdentityActivitySeen).not.toHaveBeenCalled();
});

test.each(["owner", "global"])(
  "a persistent %s busy result gets at most one offset retry",
  async (kind) => {
    jest.useFakeTimers();
    const f = fixture();
    f.listNamedIdentityActivity.mockResolvedValue(
      kind === "owner" ? unavailableOwner() : Bindings.NamedIdentityActivityListOutcome.Busy.new(),
    );
    const hook = renderHook(() => useIdentityActivity(f.runtime, 1n));
    await act(async () => {});
    await act(async () => jest.advanceTimersByTime(400));
    await act(async () => jest.advanceTimersByTime(2000));
    expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(2);
    if (kind === "owner")
      expect(hook.result.current.activity?.[0]?.activity.tag).toBe("Unavailable");
    else expect(hook.result.current.activity).toBeNull();
  },
);

test("a newer refresh or host generation cancels the old offset retry", async () => {
  jest.useFakeTimers();
  const f = fixture();
  f.listNamedIdentityActivity.mockResolvedValueOnce(unavailableOwner());
  const hook = renderHook(
    ({ generation }: { generation: bigint }) => useIdentityActivity(f.runtime, generation),
    { initialProps: { generation: 1n } },
  );
  await act(async () => {});
  await act(async () => hook.result.current.refreshActivity());
  await act(async () => jest.advanceTimersByTime(400));
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(2);
  f.listNamedIdentityActivity.mockResolvedValueOnce(unavailableOwner());
  await act(async () => hook.result.current.refreshActivity());
  hook.rerender({ generation: 2n });
  await act(async () => {});
  await act(async () => jest.advanceTimersByTime(400));
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(4);
  expect(hook.result.current.activity?.[0]?.activity.tag).toBe("Ready");
});

test("backgrounding and unmounting cancel offset reads", async () => {
  jest.useFakeTimers();
  const f = fixture();
  f.listNamedIdentityActivity.mockResolvedValue(unavailableOwner());
  let change!: (state: AppStateStatus) => void;
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
    change = listener;
    return { remove: jest.fn() };
  });
  const hook = renderHook(() => useIdentityActivity(f.runtime, 1n));
  await act(async () => {});
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "background" });
  act(() => change("background"));
  await act(async () => jest.advanceTimersByTime(400));
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(1);
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "active" });
  await act(async () => change("active"));
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(2);
  hook.unmount();
  await act(async () => jest.advanceTimersByTime(400));
  expect(f.listNamedIdentityActivity).toHaveBeenCalledTimes(2);
});
