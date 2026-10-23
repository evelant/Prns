import * as Bindings from "@prns-internal/expo";
import { act, render, waitFor } from "@testing-library/react-native";
import { Effect } from "effect";
import { AppState } from "react-native";
import {
  DevelopmentMessagingScope,
  DevelopmentRuntimeProvider,
  type DevelopmentRuntimeView,
  useDevelopmentRuntime,
} from "./development-runtime-context";
import type { RuntimeProvider } from "./runtime-provider.types";

const globalSnapshot: Bindings.DevelopmentNodeSnapshot = {
  contractFingerprint: "test",
  generationId: 7n,
  revision: 11n,
  runtime: Bindings.DevelopmentNodeRuntime.Running,
  primaryIdentity: Bindings.PrimaryIdentityState.Missing.new(),
  localHost: Bindings.LocalHostState.Stopped.new({ lastStartFailure: undefined }),
  bluetooth: {
    desiredEnabled: false,
    state: Bindings.LocalBluetoothState.Stopped.new(),
    peers: [],
  },
  network: {
    state: Bindings.LocalNetworkState.Stopped.new(),
    routes: [],
    announces: [],
    activityRevision: 0n,
    droppedAnnounceCount: 0n,
  },
  lxmf: {
    state: Bindings.LxmfHealthState.Ready,
    inboundOverflowCount: 0n,
    mailboxRevision: 0n,
    projectionRevision: 0n,
  },
  pairing: Bindings.RemoteControlPairingState.Searching.new(),
  pairingCandidates: [],
  pairedTargets: [],
};
const provider: RuntimeProvider = {
  availability: { type: "available", platform: "ios" },
  runtime: {} as Bindings.DevelopmentRuntime,
  acquire: () =>
    Effect.succeed({
      initialSnapshot: globalSnapshot,
      runtime: {},
    } as Bindings.DevelopmentRuntimeSession),
};
const originalState = Object.getOwnPropertyDescriptor(AppState, "currentState");
beforeEach(() =>
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "active" }),
);
afterAll(() => {
  if (originalState) Object.defineProperty(AppState, "currentState", originalState);
});
function fixture() {
  const pending: {
    signal: AbortSignal | undefined;
    resolve: (health: Bindings.LxmfHealth | undefined) => void;
  }[] = [];
  const waitLxmfChange = jest.fn(
    (_cursor?: bigint, signal?: AbortSignal): Promise<Bindings.LxmfHealth | undefined> =>
      new Promise((resolve) => pending.push({ signal, resolve })),
  );
  return {
    runtime: { waitLxmfChange } as unknown as Bindings.DevelopmentRuntime,
    pending,
    waitLxmfChange,
  };
}

test("the messaging scope publishes its own live health while preserving the installation snapshot", async () => {
  const first = fixture();
  const second = fixture();
  let current!: DevelopmentRuntimeView;
  function Probe() {
    current = useDevelopmentRuntime();
    return null;
  }
  function Tree({ replacement = false }: { replacement?: boolean }) {
    return (
      <DevelopmentRuntimeProvider provider={provider} refreshActive={false}>
        <DevelopmentMessagingScope
          runtime={replacement ? second.runtime : first.runtime}
          ownerKey={replacement ? "work" : "personal"}
          identityHash={new Uint8Array(16).fill(replacement ? 2 : 1)}
          canPublish
        >
          <Probe />
        </DevelopmentMessagingScope>
      </DevelopmentRuntimeProvider>
    );
  }
  const view = render(<Tree />);
  await waitFor(() => expect(first.pending).toHaveLength(1));
  await act(async () =>
    first.pending[0]?.resolve({
      ...globalSnapshot.lxmf,
      mailboxRevision: 5n,
      projectionRevision: 7n,
    }),
  );
  expect(current.messagingHealth?.mailboxRevision).toBe(5n);
  expect(current.messagingRevision).toBe(1);
  expect(current.snapshot?.lxmf).toBe(globalSnapshot.lxmf);
  expect(current.snapshot?.revision).toBe(11n);
  view.rerender(<Tree replacement />);
  expect(first.pending[1]?.signal?.aborted).toBe(true);
  expect(current.messagingHealth).toBeNull();
  await act(async () =>
    first.pending[1]?.resolve({ ...globalSnapshot.lxmf, projectionRevision: 99n }),
  );
  expect(current.messagingHealth).toBeNull();
  await act(async () =>
    second.pending[0]?.resolve({ ...globalSnapshot.lxmf, projectionRevision: 1n }),
  );
  expect(current.messagingHealth?.projectionRevision).toBe(1n);
  expect(current.ownerKey).toBe("work");
  view.unmount();
});

test("installation-only routes keep legacy snapshot refresh and do not observe a named service", async () => {
  const f = fixture();
  let current!: DevelopmentRuntimeView;
  function Probe() {
    current = useDevelopmentRuntime();
    return null;
  }
  const view = render(
    <DevelopmentRuntimeProvider provider={provider} refreshActive={false}>
      <DevelopmentMessagingScope
        runtime={f.runtime}
        ownerKey="personal"
        identityHash={undefined}
        canPublish
        projectIdentity={false}
      >
        <Probe />
      </DevelopmentMessagingScope>
    </DevelopmentRuntimeProvider>,
  );
  await waitFor(() => expect(current.phase).toBe("ready"));
  expect(current.messagingHealth).toBeUndefined();
  expect(current.messagingRevision).toBeUndefined();
  expect(current.snapshot).toBe(globalSnapshot);
  expect(f.waitLxmfChange).not.toHaveBeenCalled();
  view.unmount();
});
