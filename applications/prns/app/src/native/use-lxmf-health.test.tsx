import * as Bindings from "@prns-internal/expo";
import { act, renderHook } from "@testing-library/react-native";
import { type PropsWithChildren, StrictMode } from "react";
import { AppState, type AppStateStatus } from "react-native";
import { useLxmfHealth } from "./use-lxmf-health";

const originalState = Object.getOwnPropertyDescriptor(AppState, "currentState");
const listeners = new Set<(state: AppStateStatus) => void>();
const health = (revision: bigint, mailboxRevision = revision): Bindings.LxmfHealth => ({
  state: Bindings.LxmfHealthState.Ready,
  inboundOverflowCount: 0n,
  mailboxRevision,
  projectionRevision: revision,
});
function fixture() {
  const requests: {
    cursor: bigint | undefined;
    signal: AbortSignal;
    resolve: (value: Bindings.LxmfHealth | undefined) => void;
    reject: (error: Error) => void;
  }[] = [];
  const waitLxmfChange = jest.fn(
    (cursor?: bigint, signal?: AbortSignal): Promise<Bindings.LxmfHealth | undefined> =>
      new Promise((resolve, reject) => {
        if (signal === undefined) throw new Error("Observation must be cancellable");
        requests.push({ cursor, signal, resolve, reject });
      }),
  );
  return { runtime: { waitLxmfChange }, waitLxmfChange, requests };
}
function transition(state: AppStateStatus) {
  Object.defineProperty(AppState, "currentState", { configurable: true, value: state });
  for (const listener of listeners) listener(state);
}
beforeEach(() => {
  transition("active");
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
    listeners.add(listener);
    return { remove: () => listeners.delete(listener) };
  });
});
afterEach(() => {
  listeners.clear();
  jest.restoreAllMocks();
  jest.useRealTimers();
});
afterAll(() => {
  if (originalState) Object.defineProperty(AppState, "currentState", originalState);
});

test("observes mailbox and in-memory delivery revisions without a radio update", async () => {
  const f = fixture();
  const hook = renderHook(() => useLxmfHealth(f.runtime, "personal", 7n, true));
  expect(f.requests[0]?.cursor).toBeUndefined();
  await act(async () => f.requests[0]?.resolve(health(5n, 3n)));
  expect(hook.result.current.health?.mailboxRevision).toBe(3n);
  expect(f.requests[1]?.cursor).toBe(5n);
  await act(async () => f.requests[1]?.resolve(health(6n, 4n)));
  expect(hook.result.current.health?.mailboxRevision).toBe(4n);
  await act(async () => f.requests[2]?.resolve(health(7n, 4n)));
  expect(hook.result.current.health?.projectionRevision).toBe(7n);
  expect(hook.result.current.revision).toBe(3);
  hook.unmount();
  expect(f.requests[3]?.signal.aborted).toBe(true);
  expect(listeners.size).toBe(0);
});

test.each(["identity", "generation"])(
  "a late %s completion is fenced and the replacement starts without an old cursor",
  async (change) => {
    const first = fixture();
    const second = fixture();
    const hook = renderHook(
      ({ replaced }: { replaced: boolean }) =>
        useLxmfHealth(
          replaced && change === "identity" ? second.runtime : first.runtime,
          replaced && change === "identity" ? "work" : "personal",
          replaced && change === "generation" ? 8n : 7n,
          true,
        ),
      { initialProps: { replaced: false } },
    );
    await act(async () => first.requests[0]?.resolve(health(4n)));
    hook.rerender({ replaced: true });
    expect(first.requests[1]?.signal.aborted).toBe(true);
    expect(hook.result.current.health).toBeNull();
    const replacement = change === "identity" ? second.requests[0] : first.requests[2];
    expect(replacement?.cursor).toBeUndefined();
    await act(async () => replacement?.resolve(health(0n)));
    const current = hook.result.current;
    await act(async () => first.requests[1]?.resolve(health(99n)));
    expect(hook.result.current).toBe(current);
    hook.unmount();
  },
);

test("background aborts immediately and foreground rereads even the same service counter", async () => {
  const f = fixture();
  const hook = renderHook(() => useLxmfHealth(f.runtime, "personal", 7n, true));
  await act(async () => f.requests[0]?.resolve(health(4n)));
  act(() => transition("background"));
  expect(f.requests[1]?.signal.aborted).toBe(true);
  await act(async () => f.requests[1]?.resolve(health(99n)));
  expect(hook.result.current.health?.projectionRevision).toBe(4n);
  act(() => transition("active"));
  expect(f.requests[2]?.cursor).toBeUndefined();
  await act(async () => f.requests[2]?.resolve(health(4n)));
  expect(hook.result.current.health?.projectionRevision).toBe(4n);
  expect(hook.result.current.revision).toBe(2);
  hook.unmount();
});

test.each(["unavailable", "failure"])(
  "%s retries are bounded and reset the retired service cursor",
  async (kind) => {
    jest.useFakeTimers();
    const f = fixture();
    const hook = renderHook(() => useLxmfHealth(f.runtime, "personal", 7n, true));
    await act(async () => f.requests[0]?.resolve(health(55n)));
    await act(async () => {
      if (kind === "unavailable") f.requests[1]?.resolve(undefined);
      else f.requests[1]?.reject(new Error("Owner temporarily unavailable"));
    });
    expect(hook.result.current.health).toBeNull();
    await act(async () => jest.advanceTimersByTime(999));
    expect(f.requests).toHaveLength(2);
    await act(async () => jest.advanceTimersByTime(1));
    expect(f.requests[2]?.cursor).toBeUndefined();
    await act(async () => f.requests[2]?.resolve(health(0n)));
    expect(hook.result.current.health?.projectionRevision).toBe(0n);
    hook.unmount();
  },
);

test("pause cancels a pending watch and resume attaches to the current service", async () => {
  const f = fixture();
  const hook = renderHook(
    ({ enabled }: { enabled: boolean }) => useLxmfHealth(f.runtime, "personal", 7n, enabled),
    { initialProps: { enabled: true } },
  );
  hook.rerender({ enabled: false });
  expect(f.requests[0]?.signal.aborted).toBe(true);
  await act(async () => f.requests[0]?.resolve(health(99n)));
  expect(hook.result.current.health).toBeNull();
  hook.rerender({ enabled: true });
  expect(f.requests[1]?.cursor).toBeUndefined();
  hook.unmount();
});

test("a stopped service resets its cursor without repeatedly refreshing the mailbox", async () => {
  jest.useFakeTimers();
  const f = fixture();
  const hook = renderHook(() => useLxmfHealth(f.runtime, "personal", 7n, true));
  await act(async () => f.requests[0]?.resolve(health(55n)));
  const stopped = { ...health(55n), state: Bindings.LxmfHealthState.Stopped };
  await act(async () => f.requests[1]?.resolve(stopped));
  expect(hook.result.current.health).toBeNull();
  const revision = hook.result.current.revision;
  await act(async () => jest.advanceTimersByTime(1000));
  expect(f.requests[2]?.cursor).toBeUndefined();
  await act(async () => f.requests[2]?.resolve(stopped));
  expect(hook.result.current.revision).toBe(revision);
  await act(async () => jest.advanceTimersByTime(1000));
  expect(f.requests[3]?.cursor).toBeUndefined();
  await act(async () => f.requests[3]?.resolve(health(0n)));
  expect(hook.result.current.health?.projectionRevision).toBe(0n);
  expect(hook.result.current.revision).toBe(revision + 1);
  hook.unmount();
});

test("strict rendering leaves a usable observer and retires it on unmount", async () => {
  const f = fixture();
  const hook = renderHook(() => useLxmfHealth(f.runtime, "personal", 7n, true), {
    wrapper: ({ children }: PropsWithChildren) => <StrictMode>{children}</StrictMode>,
  });
  const active = f.requests.find((request) => !request.signal.aborted);
  await act(async () => active?.resolve(health(4n)));
  expect(hook.result.current.health?.projectionRevision).toBe(4n);
  hook.unmount();
  expect(f.requests.every((request) => request.signal.aborted || request === active)).toBe(true);
});
