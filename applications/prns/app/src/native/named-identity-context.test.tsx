import * as Bindings from "@prns-internal/expo";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { act, fireEvent, render, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";
import { IdentitiesScreen, IdentityDetailScreen } from "@/features/identities/identities-screen";
import { IdentityBanner } from "@/features/identities/identity-banner";
import { identityHref } from "./identity-navigation";
import {
  identityReferenceKey,
  NamedIdentityProvider,
  useNamedIdentity,
} from "./named-identity-context";
import { readPendingIdentityCreation } from "./pending-identity-creation";

const mockRuntimeState = Bindings.DevelopmentNodeRuntime;
let mockHostGeneration = 1n;
let mockHostRunning = true;
let mockHostStopping = false;
jest.mock("./development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({
    phase: "ready",
    stoppingNode: mockHostStopping,
    snapshot: {
      generationId: mockHostGeneration,
      runtime: mockHostRunning ? mockRuntimeState.Running : mockRuntimeState.Stopped,
    },
  }),
}));
jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
  usePathname: () => "/inbox",
}));
const reference = (sequence: bigint): Bindings.NamedIdentityReference => ({
  incarnation: new Uint8Array(16).fill(7),
  sequence,
  publicHash: new Uint8Array(16).fill(Number(sequence)),
});
const a = reference(1n);
const b = reference(2n);
function fixture() {
  let catalog: Bindings.NamedIdentityCatalog = {
    selected: a,
    identities: [a, b].map((reference, index) => ({
      reference,
      label: index === 0 ? "Personal" : "Work",
      publishedName: undefined,
      state: Bindings.NamedIdentityState.Enabled,
      ready: true,
      transitioning: false,
    })),
  };
  const listNamedIdentities = jest.fn(
    async (): Promise<Bindings.NamedIdentityCatalogOutcome> =>
      Bindings.NamedIdentityCatalogOutcome.Ready.new({ catalog }),
  );
  const changeNamedIdentity = jest.fn(
    async (
      input: Bindings.ChangeNamedIdentityInput,
    ): Promise<Bindings.NamedIdentityCatalogOutcome> => {
      if (input.change.tag === "Select") catalog = { ...catalog, selected: input.identity };
      if (input.change.tag === "Pause" || input.change.tag === "Resume") {
        const state =
          input.change.tag === "Pause"
            ? Bindings.NamedIdentityState.Paused
            : Bindings.NamedIdentityState.Enabled;
        catalog = {
          ...catalog,
          identities: catalog.identities.map((entry) =>
            entry.reference.sequence === input.identity.sequence
              ? { ...entry, state, ready: state === Bindings.NamedIdentityState.Enabled }
              : entry,
          ),
        };
      }
      return Bindings.NamedIdentityCatalogOutcome.Ready.new({ catalog });
    },
  );
  const markIdentityActivitySeen = jest.fn();
  const listNamedIdentityActivity = jest.fn(async () =>
    Bindings.NamedIdentityActivityListOutcome.Ready.new({
      activities: [
        {
          identity: b,
          activity: Bindings.IdentityActivityOutcome.Ready.new({
            summary: {
              actionableInvitations: 1,
              unreadGroups: 2,
              unreadDirect: 0,
              conversations: [],
              partial: false,
            },
          }),
        },
      ],
    }),
  );
  const forIdentity = jest.fn((owner: Bindings.NamedIdentityReference) => ({
    listContacts: async () => ({ tag: "Listed", owner: owner.sequence }),
    markIdentityActivitySeen,
  }));
  const createNamedIdentity = jest.fn(
    async (
      _input: Bindings.CreateNamedIdentityInput,
    ): Promise<Bindings.NamedIdentityCatalogOutcome> =>
      Bindings.NamedIdentityCatalogOutcome.Ready.new({ catalog }),
  );
  const runtime = {
    listNamedIdentities,
    listNamedIdentityActivity,
    changeNamedIdentity,
    forIdentity,
    createNamedIdentity,
    createGroupRequestId: jest.fn(async () =>
      Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: new Uint8Array(32).fill(9) }),
    ),
  } as unknown as Bindings.DevelopmentRuntime;
  return {
    runtime,
    markIdentityActivitySeen,
    listNamedIdentities,
    changeNamedIdentity,
    forIdentity,
    createNamedIdentity,
    catalog: () => catalog,
  };
}
let view: NonNullable<ReturnType<typeof useNamedIdentity>>;
function Probe() {
  const value = useNamedIdentity();
  if (value !== null) view = value;
  return null;
}
beforeEach(async () => {
  mockHostGeneration = 1n;
  mockHostRunning = true;
  mockHostStopping = false;
  await AsyncStorage.clear();
});

test("selection changes only the default; already captured journeys retain their own client", async () => {
  const f = fixture();
  const screen = render(
    <NamedIdentityProvider pathname="/contacts/import" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.identity?.label).toBe("Personal"));
  await waitFor(() => expect(view.activity?.[0]?.identity).toBe(b));
  const original = view.runtime;
  await act(async () => {
    await view.change(b, Bindings.NamedIdentityChange.Select.new());
  });
  expect(view.identity?.label).toBe("Personal");
  expect(view.runtime).toBe(original);
  expect(f.markIdentityActivitySeen).not.toHaveBeenCalled();
  expect(
    f
      .catalog()
      .identities.every((identity) => identity.state === Bindings.NamedIdentityState.Enabled),
  ).toBe(true);
  screen.rerender(
    <NamedIdentityProvider pathname="/contacts" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.identity?.label).toBe("Work"));
  expect(view.runtime).not.toBe(original);
  await expect(original?.listContacts()).resolves.toMatchObject({ owner: 1n });
  await expect(view.runtime?.listContacts()).resolves.toMatchObject({ owner: 2n });
});

test("explicit route ownership survives a changed default; a missing owner never falls back", async () => {
  const f = fixture();
  const screen = render(
    <NamedIdentityProvider
      pathname="/inbox"
      requestedOwner={identityReferenceKey(a)}
      runtime={f.runtime}
    >
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.identity?.label).toBe("Personal"));
  await act(async () => {
    await view.change(b, Bindings.NamedIdentityChange.Select.new());
  });
  expect(view.ownerKey).toBe(identityReferenceKey(a));
  screen.rerender(
    <NamedIdentityProvider
      pathname="/inbox"
      requestedOwner={identityReferenceKey(reference(50n))}
      runtime={f.runtime}
    >
      <Probe />
    </NamedIdentityProvider>,
  );
  expect(view.identity).toBeNull();
  expect(view.runtime).toBeNull();
  expect(view.canPublish).toBe(false);
});

test("pause retains the scoped history client, fences publication and ignores stale catalog reads", async () => {
  const f = fixture();
  render(
    <NamedIdentityProvider pathname="/inbox" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.canPublish).toBe(true));
  const original = view.runtime;
  const oldCatalog = f.catalog();
  let resolve!: (value: Bindings.NamedIdentityCatalogOutcome) => void;
  f.listNamedIdentities.mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    }),
  );
  let pending!: Promise<void>;
  act(() => {
    pending = view.refresh();
  });
  await act(async () => {
    await view.change(a, Bindings.NamedIdentityChange.Pause.new());
  });
  await act(async () => {
    resolve(Bindings.NamedIdentityCatalogOutcome.Ready.new({ catalog: oldCatalog }));
    await pending;
  });
  expect(view.canPublish).toBe(false);
  expect(view.identity?.state).toBe(Bindings.NamedIdentityState.Paused);
  expect(view.runtime).toBe(original);
  await expect(view.runtime?.listContacts()).resolves.toMatchObject({ owner: 1n });
  await act(async () => {
    await view.change(a, Bindings.NamedIdentityChange.Resume.new());
  });
  expect(view.canPublish).toBe(true);
});

test("an uncertain creation survives process loss and retries the same request", async () => {
  const f = fixture();
  f.createNamedIdentity.mockRejectedValueOnce(new Error("reply lost"));
  const screen = render(
    <NamedIdentityProvider pathname="/more/identities" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.creationReady).toBe(true));
  await act(async () => {
    expect(await view.create("Friends")).toBe(false);
  });
  const input = f.createNamedIdentity.mock.calls[0]?.[0];
  expect(await readPendingIdentityCreation()).toMatchObject({
    label: "Friends",
    requestId: input?.requestId,
  });
  screen.unmount();
  render(
    <NamedIdentityProvider pathname="/more/identities" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.pendingCreationLabel).toBe("Friends"));
  await act(async () => {
    expect(await view.create("Other")).toBe(false);
  });
  expect(f.createNamedIdentity).toHaveBeenCalledTimes(1);
  await act(async () => {
    expect(await view.create("Friends")).toBe(true);
  });
  expect(f.createNamedIdentity.mock.calls[1]?.[0]).toEqual(input);
  expect(await readPendingIdentityCreation()).toBeNull();
});

test("identity UI separates choosing from pausing and keeps history reachable", async () => {
  const f = fixture();
  const screen = render(
    <NamedIdentityProvider pathname="/more/identities" runtime={f.runtime}>
      <Probe />
      <IdentitiesScreen />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(screen.getByText("Use Work by default")).toBeTruthy());
  fireEvent.press(screen.getByText("Use Work by default"));
  await waitFor(() =>
    expect(f.changeNamedIdentity).toHaveBeenCalledWith({
      identity: b,
      change: Bindings.NamedIdentityChange.Select.new(),
    }),
  );
  screen.rerender(
    <NamedIdentityProvider pathname="/more/identities/work" runtime={f.runtime}>
      <Probe />
      <IdentityDetailScreen identityId={identityReferenceKey(b)} />
    </NamedIdentityProvider>,
  );
  fireEvent.press(screen.getByText("Pause identity"));
  await waitFor(() => expect(screen.getByText("Resume identity")).toBeTruthy());
  expect(screen.getByText("Open Inbox")).toBeTruthy();
  expect(screen.getByText("Open Contacts")).toBeTruthy();
  expect(screen.getByText(/Saved contacts, messages and groups remain here/)).toBeTruthy();
});

test("navigation pins message paths without leaking local references into shared links or node management", () => {
  const owner = identityReferenceKey(a);
  expect(identityHref("/contacts/import", owner)).toBe(`/contacts/import?owner=${owner}`);
  expect(
    identityHref({ pathname: "/inbox/group/[groupId]", params: { groupId: "abc" } }, owner),
  ).toMatchObject({ params: { owner, groupId: "abc" } });
  expect(identityHref({ pathname: "/inbox", params: { owner: "another" } }, owner)).toMatchObject({
    params: { owner: "another" },
  });
  expect(identityHref("/nodes", owner)).toBe("/nodes");
  expect(identityReferenceKey({ ...a, sequence: 9n })).not.toBe(owner);
});

test("a definitive unadmitted creation rejection unlocks the name without preserving a retry", async () => {
  const f = fixture();
  f.createNamedIdentity.mockResolvedValueOnce(
    Bindings.NamedIdentityCatalogOutcome.Rejected.new({ detail: "Identity name is too long" }),
  );
  render(
    <NamedIdentityProvider pathname="/more/identities" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.creationReady).toBe(true));
  await act(async () => {
    expect(await view.create("Invalid")).toBe(false);
  });
  expect(view.pendingCreationLabel).toBeNull();
  expect(await readPendingIdentityCreation()).toBeNull();
  await act(async () => {
    expect(await view.create("Corrected")).toBe(true);
  });
  expect(f.createNamedIdentity.mock.calls[1]?.[0].label).toBe("Corrected");
});

test("whole-host stop immediately fences publication and restart requires fresh-generation readiness", async () => {
  const f = fixture();
  const screen = render(
    <NamedIdentityProvider pathname="/inbox" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(view.canPublish).toBe(true));
  const historyClient = view.runtime;
  mockHostStopping = true;
  screen.rerender(
    <NamedIdentityProvider pathname="/inbox" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  expect(view.canPublish).toBe(false);
  mockHostRunning = false;
  mockHostStopping = false;
  screen.rerender(
    <NamedIdentityProvider pathname="/inbox" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  expect(view.canPublish).toBe(false);
  expect(view.runtime).toBe(historyClient);
  await expect(view.runtime?.listContacts()).resolves.toMatchObject({ owner: 1n });
  let resolve!: (value: Bindings.NamedIdentityCatalogOutcome) => void;
  f.listNamedIdentities.mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    }),
  );
  mockHostRunning = true;
  mockHostGeneration = 2n;
  screen.rerender(
    <NamedIdentityProvider pathname="/inbox" runtime={f.runtime}>
      <Probe />
    </NamedIdentityProvider>,
  );
  expect(view.canPublish).toBe(false);
  await act(async () => {
    resolve(Bindings.NamedIdentityCatalogOutcome.Ready.new({ catalog: f.catalog() }));
  });
  expect(view.canPublish).toBe(true);
  expect(view.runtime).toBe(historyClient);
});

test("pending lifecycle changes keep history open without claiming a completed pause or allowing publication", async () => {
  const f = fixture();
  const catalog = (state: Bindings.NamedIdentityState, transitioning: boolean) => ({
    ...f.catalog(),
    identities: f
      .catalog()
      .identities.map((entry) =>
        entry.reference.sequence === a.sequence
          ? { ...entry, state, transitioning, ready: state === Bindings.NamedIdentityState.Enabled }
          : entry,
      ),
  });
  f.listNamedIdentities.mockResolvedValue(
    Bindings.NamedIdentityCatalogOutcome.Ready.new({
      catalog: catalog(Bindings.NamedIdentityState.Paused, true),
    }),
  );
  const screen = render(
    <NamedIdentityProvider pathname="/inbox" runtime={f.runtime}>
      <Probe />
      <IdentityBanner />
      <IdentityDetailScreen identityId={identityReferenceKey(a)} />
    </NamedIdentityProvider>,
  );
  await waitFor(() => expect(screen.getByRole("button", { name: "Finish pausing" })).toBeEnabled());
  expect(screen.queryByRole("button", { name: "Resume identity" })).toBeNull();
  expect(screen.getByTestId("identity-paused-banner")).toHaveTextContent(/^Pausing…/);
  expect(screen.getByText("Open Inbox")).toBeTruthy();
  expect(screen.getByText("Open Contacts")).toBeTruthy();
  expect(view.canPublish).toBe(false);
  const historyClient = view.runtime;
  let finish!: (outcome: Bindings.NamedIdentityCatalogOutcome) => void;
  f.changeNamedIdentity.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  fireEvent.press(screen.getByRole("button", { name: "Finish pausing" }));
  expect(screen.getByRole("button", { name: "Finish pausing" })).toBeDisabled();
  expect(f.changeNamedIdentity).toHaveBeenLastCalledWith({
    identity: a,
    change: Bindings.NamedIdentityChange.Pause.new(),
  });
  await act(async () =>
    finish(Bindings.NamedIdentityCatalogOutcome.Unavailable.new({ detail: "Still draining" })),
  );
  expect(screen.getByRole("button", { name: "Finish pausing" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: "Resume identity" })).toBeNull();
  f.listNamedIdentities.mockResolvedValue(
    Bindings.NamedIdentityCatalogOutcome.Ready.new({
      catalog: catalog(Bindings.NamedIdentityState.Enabled, true),
    }),
  );
  await act(async () => view.refresh());
  expect(screen.getByRole("button", { name: "Finish starting" })).toBeEnabled();
  expect(screen.getByTestId("identity-paused-banner")).toHaveTextContent(/^Starting…/);
  expect(view.canPublish).toBe(false);
  expect(view.runtime).toBe(historyClient);
  fireEvent.press(screen.getByRole("button", { name: "Finish starting" }));
  await waitFor(() =>
    expect(f.changeNamedIdentity).toHaveBeenLastCalledWith({
      identity: a,
      change: Bindings.NamedIdentityChange.Resume.new(),
    }),
  );
  f.listNamedIdentities.mockResolvedValue(
    Bindings.NamedIdentityCatalogOutcome.Ready.new({
      catalog: catalog(Bindings.NamedIdentityState.Paused, false),
    }),
  );
  await act(async () => view.refresh());
  expect(screen.getByRole("button", { name: "Resume identity" })).toBeEnabled();
  expect(screen.getByTestId("identity-paused-banner")).toHaveTextContent(/^Paused$/);
  expect(view.canPublish).toBe(false);
});

test("navigating during an admitted local Pause preserves history and fences only its owner immediately", async () => {
  const f = fixture();
  let complete!: (outcome: Bindings.NamedIdentityCatalogOutcome) => void;
  f.changeNamedIdentity.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const journey = (pathname: string, owner = a) => (
    <NamedIdentityProvider
      pathname={pathname}
      requestedOwner={identityReferenceKey(owner)}
      runtime={f.runtime}
    >
      <Probe />
      <IdentityBanner />
    </NamedIdentityProvider>
  );
  const screen = render(journey("/more/identities/personal"));
  await waitFor(() => expect(view.canPublish).toBe(true));
  const historyClient = view.runtime;
  let pending!: Promise<boolean>;
  await act(async () => {
    pending = view.change(a, Bindings.NamedIdentityChange.Pause.new());
  });
  expect(view.loading).toBe(true);
  expect(view.canPublish).toBe(false);
  expect(view.identity).toMatchObject({
    state: Bindings.NamedIdentityState.Paused,
    transitioning: true,
  });
  screen.rerender(journey("/inbox/group/saved"));
  expect(screen.getByTestId("identity-paused-banner")).toHaveTextContent(/^Pausing…/);
  expect(view.runtime).toBe(historyClient);
  await expect(view.runtime?.listContacts()).resolves.toMatchObject({ owner: 1n });
  screen.rerender(journey("/inbox", b));
  expect(view.canPublish).toBe(true);
  expect(view.identity?.label).toBe("Work");
  screen.rerender(journey("/inbox/group/saved"));
  expect(view.canPublish).toBe(false);
  const completedCatalog = {
    ...f.catalog(),
    identities: f.catalog().identities.map((entry) =>
      entry.reference.sequence === a.sequence
        ? {
            ...entry,
            state: Bindings.NamedIdentityState.Paused,
            ready: false,
            transitioning: false,
          }
        : entry,
    ),
  };
  await act(async () => {
    complete(Bindings.NamedIdentityCatalogOutcome.Ready.new({ catalog: completedCatalog }));
    expect(await pending).toBe(true);
  });
  expect(view.loading).toBe(false);
  expect(view.canPublish).toBe(false);
  expect(view.identity?.transitioning).toBe(false);
  expect(screen.getByTestId("identity-paused-banner")).toHaveTextContent(/^Paused$/);
  await expect(view.runtime?.listContacts()).resolves.toMatchObject({ owner: 1n });
  await expect(historyClient?.listContacts()).resolves.toMatchObject({ owner: 1n });
});

test("an unconfirmed lifecycle result stays fenced for its owner until authoritative refresh", async () => {
  const f = fixture();
  const journey = (owner = a) => (
    <NamedIdentityProvider
      pathname="/inbox"
      requestedOwner={identityReferenceKey(owner)}
      runtime={f.runtime}
    >
      <Probe />
    </NamedIdentityProvider>
  );
  const screen = render(journey());
  await waitFor(() => expect(view.canPublish).toBe(true));
  f.changeNamedIdentity.mockResolvedValueOnce(
    Bindings.NamedIdentityCatalogOutcome.Unavailable.new({ detail: "Pause outcome unknown" }),
  );
  f.listNamedIdentities.mockRejectedValueOnce(new Error("Catalog unavailable"));
  await act(async () =>
    expect(await view.change(a, Bindings.NamedIdentityChange.Pause.new())).toBe(false),
  );
  expect(view.canPublish).toBe(false);
  expect(view.identity?.ready).toBe(false);
  screen.rerender(journey(b));
  expect(view.canPublish).toBe(true);
  screen.rerender(journey());
  expect(view.canPublish).toBe(false);
  await act(async () => view.refresh());
  expect(view.canPublish).toBe(true);
});
