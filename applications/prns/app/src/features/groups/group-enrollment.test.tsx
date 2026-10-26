import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, renderHook, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";

import type { RuntimeCommandResult } from "@/native/development-runtime-context";
import { createGroupActionTestStore } from "@/testkit/group-action-store";
import { CreateGroupScreen } from "./create-group-screen";
import { identifierHex } from "./format";
import { GroupInboxSection } from "./group-inbox-section";
import { GroupInvitationScreen } from "./group-invitation-screen";
import { InviteToGroupScreen } from "./invite-to-group-screen";
import { useGroupContacts } from "./use-group-contacts";
import { useGroupInvitations } from "./use-group-invitations";

const mockGroupId = new Uint8Array(32).fill(0x11);
const mockInvitationId = new Uint8Array(32).fill(0x22);
const mockRequestId = new Uint8Array(32).fill(0x33);
const mockOwner = new Uint8Array(16).fill(0x44);
const mockRecipient = new Uint8Array(16).fill(0x55);
const mockConversation: Bindings.GroupConversation = {
  groupId: mockGroupId,
  title: "Field crew",
  revision: 1n,
  memberCount: 1,
  controlHead: new Uint8Array(32).fill(0x66),
  members: [
    {
      endpointId: new Uint8Array(32).fill(0x44),
      endpointIncarnation: 1n,
      identity: mockOwner,
      isLocal: true,
      isOwner: true,
    },
  ],
  ownerIdentity: mockOwner,
  canInvite: true,
  synchronization: Bindings.GroupSynchronizationState.Stopped.new(),
  progress: { running: false, observed: false, peers: [] },
};
const mockInvitation: Bindings.GroupInvitation = {
  groupId: mockGroupId,
  invitationId: mockInvitationId,
  title: "Field crew",
  inviterIdentity: mockOwner,
  recipientIdentity: mockRecipient,
  expiresAtMillis: 1_900_000_000_000n,
  outgoing: false,
  state: Bindings.GroupInvitationState.Offered,
  waitingForDelivery: false,
};
let mockSnapshot: Pick<
  Bindings.DevelopmentNodeSnapshot,
  "generationId" | "runtime" | "primaryIdentity"
> = {
  generationId: 1n,
  runtime: Bindings.DevelopmentNodeRuntime.Stopped,
  primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
    identityHash: new Uint8Array(16).fill(0x44),
  }),
};
const mockCreateRequestId = jest.fn(async () => ({
  type: "outcome" as const,
  outcome: Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: mockRequestId }),
}));
const mockCreateGroup = jest.fn(
  async (
    _input: Bindings.CreateGroupInput,
  ): Promise<RuntimeCommandResult<Bindings.CreateGroupOutcome>> => ({
    type: "outcome",
    outcome: Bindings.CreateGroupOutcome.Accepted.new({ groupId: mockGroupId }),
  }),
);
const mockInviteToGroup = jest.fn(
  async (
    _input: Bindings.InviteToGroupInput,
  ): Promise<RuntimeCommandResult<Bindings.InviteToGroupOutcome>> => ({
    type: "outcome",
    outcome: Bindings.InviteToGroupOutcome.Queued.new({ invitationId: mockInvitationId }),
  }),
);
const mockRespond = jest.fn(
  async (
    _input: Bindings.RespondGroupInvitationInput,
  ): Promise<RuntimeCommandResult<Bindings.RespondGroupInvitationOutcome>> => ({
    type: "outcome",
    outcome: Bindings.RespondGroupInvitationOutcome.Saved.new(),
  }),
);
const mockListGroups = jest.fn(
  async (): Promise<RuntimeCommandResult<Bindings.GroupConversationListOutcome>> => ({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({ conversations: [mockConversation] }),
  }),
);
const mockListInvitations = jest.fn(
  async (): Promise<RuntimeCommandResult<Bindings.GroupInvitationListOutcome>> =>
    invitationsResult([mockInvitation]),
);
const mockListContacts = jest.fn(async () =>
  Bindings.ContactListOutcome.Listed.new({ contacts: [] }),
);
const mockCard: Bindings.ContactCard = {
  uri: "rns-contact://v1/YWJj",
  displayName: "Scanned Ranger",
  identity: mockRecipient,
  destination: new Uint8Array(16).fill(0xab),
  serviceNameHash: new Uint8Array(10).fill(0xcd),
};
const mockPreviewCard = jest.fn(
  async (_uri: string): Promise<Bindings.ContactCardPreviewOutcome> =>
    Bindings.ContactCardPreviewOutcome.Valid.new({ card: mockCard }),
);
const mockImportCard = jest.fn(
  async (_input: Bindings.ImportContactCardInput): Promise<Bindings.ContactCardImportOutcome> =>
    Bindings.ContactCardImportOutcome.Saved.new({
      contact: {
        destination: mockCard.destination,
        identity: mockCard.identity,
        alias: mockCard.displayName,
        announcedName: undefined,
        pinned: false,
        isMessaging: true,
      },
    }),
);
const mockContactRuntime = {
  listContacts: mockListContacts,
  previewContactCard: mockPreviewCard,
  importContactCard: mockImportCard,
};
jest.mock("@/features/contacts/contact-qr-scanner", () => ({
  ContactQrScanner: ({ onScan }: { onScan: (value: string) => void }) => {
    const { Button } = jest.requireActual<typeof import("@/ui/primitives")>("@/ui/primitives");
    return <Button onPress={() => onScan(mockCard.uri)}>Read test QR</Button>;
  },
}));

const mockActions = createGroupActionTestStore({
  create: (input) => mockCreateGroup(input),
  invite: (input) => mockInviteToGroup(input),
  respond: (input) => mockRespond(input),
});

jest.mock("./group-announce-settings", () => ({ GroupAnnounceSettings: () => null }));

jest.mock("expo-router", () => ({
  useFocusEffect: (effect: () => undefined | (() => void)) => {
    const { useEffect } = jest.requireActual<typeof import("react")>("react");
    useEffect(effect, [effect]);
  },
  Link: ({ children }: { readonly children: ReactNode }) => children,
}));
jest.mock("@/native/contact-runtime-context", () => ({
  useContactRuntime: () => ({ runtime: mockContactRuntime }),
}));
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({
    ...mockActions,
    availability: { type: "available", platform: "ios" },
    phase: "ready",
    snapshot: mockSnapshot,
    createGroupRequestId: mockCreateRequestId,
    createGroup: mockCreateGroup,
    inviteToGroup: mockInviteToGroup,
    listGroupInvitations: mockListInvitations,
    respondGroupInvitation: mockRespond,
    listGroupConversations: mockListGroups,
  }),
}));

function invitationsResult(
  invitations: Bindings.GroupInvitation[],
): RuntimeCommandResult<Bindings.GroupInvitationListOutcome> {
  return {
    type: "outcome",
    outcome: Bindings.GroupInvitationListOutcome.Ready.new({ invitations }),
  };
}

beforeEach(() => {
  jest.clearAllMocks();
  mockActions.actions.clear();
  mockSnapshot = {
    generationId: 1n,
    runtime: Bindings.DevelopmentNodeRuntime.Stopped,
    primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
      identityHash: new Uint8Array(16).fill(0x44),
    }),
  };
  mockCreateGroup.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.CreateGroupOutcome.Accepted.new({ groupId: mockGroupId }),
  });
  mockInviteToGroup.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.InviteToGroupOutcome.Queued.new({ invitationId: mockInvitationId }),
  });
  mockRespond.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.RespondGroupInvitationOutcome.Saved.new(),
  });
  mockListGroups.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({ conversations: [mockConversation] }),
  });
  mockListInvitations.mockResolvedValue(invitationsResult([mockInvitation]));
  mockListContacts.mockResolvedValue(Bindings.ContactListOutcome.Listed.new({ contacts: [] }));
});

test("creates an owner-only group while the node is stopped and prevents rapid duplicate admission", async () => {
  let finish: ((result: RuntimeCommandResult<Bindings.CreateGroupOutcome>) => void) | undefined;
  mockCreateGroup.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const screen = render(<CreateGroupScreen />);
  await waitFor(() => expect(screen.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(screen.getByLabelText("Group name"), "Field crew");
  const create = screen.getByRole("button", { name: "Create group" });
  fireEvent.press(create);
  fireEvent.press(create);
  await waitFor(() => expect(mockCreateGroup).toHaveBeenCalledTimes(1));
  expect(mockCreateGroup).toHaveBeenCalledWith({
    clientRequestId: mockRequestId,
    title: "Field crew",
  });
  await act(async () =>
    finish?.({
      type: "outcome",
      outcome: Bindings.CreateGroupOutcome.Accepted.new({ groupId: mockGroupId }),
    }),
  );
  expect(screen.getByText("Group created. You are its owner and its only member.")).toBeTruthy();
  expect(screen.getByRole("link", { name: "Invite a device" })).toBeTruthy();
});

test("ambiguous creation retries the same immutable title and request ID", async () => {
  mockCreateGroup.mockResolvedValueOnce({ type: "operationFailure", detail: "lost result" });
  const screen = render(<CreateGroupScreen />);
  await waitFor(() => expect(screen.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(screen.getByLabelText("Group name"), "Field crew");
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  fireEvent.press(await screen.findByRole("button", { name: "Retry same creation" }));
  await screen.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
  expect(mockCreateGroup.mock.calls.map(([input]) => input)).toEqual([
    { clientRequestId: mockRequestId, title: "Field crew" },
    { clientRequestId: mockRequestId, title: "Field crew" },
  ]);
});

test("node replacement rejects a late creation result and retains its retry ID", async () => {
  let finish: ((result: RuntimeCommandResult<Bindings.CreateGroupOutcome>) => void) | undefined;
  mockCreateGroup.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const screen = render(<CreateGroupScreen />);
  await waitFor(() => expect(screen.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(screen.getByLabelText("Group name"), "Field crew");
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  await waitFor(() => expect(mockCreateGroup).toHaveBeenCalledTimes(1));
  mockSnapshot = { ...mockSnapshot, generationId: 2n };
  screen.rerender(<CreateGroupScreen />);
  await act(async () =>
    finish?.({
      type: "outcome",
      outcome: Bindings.CreateGroupOutcome.Accepted.new({ groupId: mockGroupId }),
    }),
  );
  expect(screen.queryByText("Group created. You are its owner and its only member.")).toBeNull();
  fireEvent.press(screen.getByRole("button", { name: "Retry same creation" }));
  await screen.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
});

test("a completed creation is restored from its journal result after the node changes", async () => {
  const screen = render(<CreateGroupScreen />);
  await waitFor(() => expect(screen.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(screen.getByLabelText("Group name"), "Field crew");
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  await screen.findByText("Group created. You are its owner and its only member.");
  mockSnapshot = { ...mockSnapshot, generationId: 2n };
  screen.rerender(<CreateGroupScreen />);
  expect(screen.queryByRole("link", { name: "Invite a device" })).toBeNull();
  await screen.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateGroup.mock.calls.map(([input]) => input)).toEqual([
    { clientRequestId: mockRequestId, title: "Field crew" },
  ]);
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
});

test("contact selection invites its primary identity and explains the history and delivery limits", async () => {
  mockListContacts.mockResolvedValue(
    Bindings.ContactListOutcome.Listed.new({
      contacts: [
        {
          destination: new Uint8Array(16).fill(0xaa),
          identity: mockRecipient,
          alias: "Ranger",
          announcedName: undefined,
          pinned: true,
          isMessaging: false,
        },
        {
          destination: new Uint8Array(16).fill(0xbb),
          identity: undefined,
          alias: "Address only",
          announcedName: undefined,
          pinned: false,
          isMessaging: true,
        },
      ],
    }),
  );
  mockInviteToGroup.mockResolvedValueOnce({ type: "operationFailure", detail: "lost result" });
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  await screen.findByText(/Device identity is not known/);
  fireEvent.press(screen.getByRole("button", { name: "Choose Ranger" }));
  expect(screen.getByText(identifierHex(mockRecipient))).toBeTruthy();
  expect(screen.queryByLabelText("Search contacts")).toBeNull();
  expect(screen.getByText(/Joining waits for this device’s pending deliveries/)).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "About invitations" }));
  expect(
    screen.getByText(/Pending deliveries on other devices can still be interrupted/),
  ).toBeTruthy();
  expect(screen.getByText(/New members receive future messages only/)).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Send invitation" }));
  const retry = await screen.findByRole("button", { name: "Retry same invitation" });
  expect(
    screen.getByRole("button", { name: "Choose another device" }).props.accessibilityState.disabled,
  ).toBe(true);
  fireEvent.press(retry);
  await screen.findByText("Invitation saved. The device still needs to accept and finish joining.");
  expect(mockInviteToGroup.mock.calls.map(([input]) => input)).toEqual([
    { clientRequestId: mockRequestId, groupId: mockGroupId, recipientIdentity: mockRecipient },
    { clientRequestId: mockRequestId, groupId: mockGroupId, recipientIdentity: mockRecipient },
  ]);
});

test("legacy or non-owner groups do not offer invitation controls", async () => {
  mockListGroups.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({
      conversations: [{ ...mockConversation, canInvite: false, ownerIdentity: undefined }],
    }),
  });
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  await screen.findByText("Invitations are unavailable for this older group.");
  expect(screen.queryByRole("button", { name: "Send invitation" })).toBeNull();
  expect(mockInviteToGroup).not.toHaveBeenCalled();
});

test("Inbox shows invitations before the first group exists", async () => {
  mockListGroups.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.NotProvisioned.new(),
  });
  const screen = render(<GroupInboxSection />);
  await screen.findByText("No groups yet.");
  expect(await screen.findByText("Invitation received")).toBeTruthy();
  expect(screen.getByRole("link", { name: "View invitation for Field crew" })).toBeTruthy();
});

test("saved contact matching never replaces the authenticated fingerprint with a verified-person claim", async () => {
  mockListContacts.mockResolvedValue(
    Bindings.ContactListOutcome.Listed.new({
      contacts: [
        {
          destination: new Uint8Array(16).fill(0x99),
          identity: mockOwner,
          alias: "Ranger",
          announcedName: undefined,
          pinned: true,
          isMessaging: true,
        },
      ],
    }),
  );
  const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  expect(await screen.findByText("From: Ranger")).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Invitation identity details" }));
  expect(screen.getByText("Matches saved contact: Ranger")).toBeTruthy();
  expect(screen.getByText(identifierHex(mockOwner))).toBeTruthy();
  expect(screen.getByText(/Contact names do not verify a person/)).toBeTruthy();
  expect(screen.getByText(/The invitation is authenticated for this device/)).toBeTruthy();
});

test("acceptance is a saved join request until native membership is confirmed", async () => {
  const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  fireEvent.press(await screen.findByRole("button", { name: "Accept invitation" }));
  expect(
    await screen.findByText("Join request saved. Waiting for membership confirmation."),
  ).toBeTruthy();
  expect(screen.queryByRole("link", { name: "Open group conversation" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
  expect(mockRespond).toHaveBeenCalledWith({
    invitationId: mockInvitationId,
    clientRequestId: mockRequestId,
    accept: true,
  });
  mockListInvitations.mockResolvedValue(
    invitationsResult([{ ...mockInvitation, state: Bindings.GroupInvitationState.Joining }]),
  );
  fireEvent.press(screen.getByRole("button", { name: "Refresh invitation" }));
  await screen.findByText("Joining · waiting for membership confirmation");
  expect(screen.queryByRole("link", { name: "Open group conversation" })).toBeNull();
  mockListInvitations.mockResolvedValue(
    invitationsResult([{ ...mockInvitation, state: Bindings.GroupInvitationState.Joined }]),
  );
  fireEvent.press(screen.getByRole("button", { name: "Refresh invitation" }));
  expect(await screen.findByRole("link", { name: "Open group conversation" })).toBeTruthy();
});

test.each([false, true])(
  "deferred joining explains pending delivery without offering another acceptance (outgoing %s)",
  async (outgoing) => {
    mockListInvitations.mockResolvedValue(
      invitationsResult([
        {
          ...mockInvitation,
          outgoing,
          state: Bindings.GroupInvitationState.Joining,
          waitingForDelivery: true,
        },
      ]),
    );
    const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
    await screen.findByText("Waiting for current deliveries");
    expect(
      screen.getByText(
        outgoing
          ? "Joining will continue automatically when this device’s pending deliveries finish or expire."
          : "The owner is finishing pending deliveries. Your acceptance is saved; joining will continue automatically.",
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Open group conversation" })).toBeNull();
    expect(mockRespond).not.toHaveBeenCalled();
    mockListInvitations.mockResolvedValue(
      invitationsResult([
        {
          ...mockInvitation,
          outgoing,
          state: Bindings.GroupInvitationState.Joined,
          waitingForDelivery: false,
        },
      ]),
    );
    fireEvent.press(screen.getByRole("button", { name: "Refresh invitation" }));
    expect(await screen.findByRole("link", { name: "Open group conversation" })).toBeTruthy();
    expect(screen.queryByText("Waiting for current deliveries")).toBeNull();
    expect(mockRespond).not.toHaveBeenCalled();
  },
);

test("an ambiguous decline cannot become acceptance when retrying", async () => {
  mockRespond.mockResolvedValueOnce({ type: "operationFailure", detail: "lost response" });
  const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  fireEvent.press(await screen.findByRole("button", { name: "Decline invitation" }));
  const retry = await screen.findByRole("button", { name: "Retry same decision" });
  expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
  fireEvent.press(retry);
  await screen.findByText("Decline saved.");
  expect(mockRespond.mock.calls.map(([input]) => input)).toEqual([
    { invitationId: mockInvitationId, clientRequestId: mockRequestId, accept: false },
    { invitationId: mockInvitationId, clientRequestId: mockRequestId, accept: false },
  ]);
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
});

test.each([Bindings.GroupInvitationState.Expired, Bindings.GroupInvitationState.Declined])(
  "terminal invitation %s has no accept or decline action",
  async (state) => {
    mockListInvitations.mockResolvedValue(invitationsResult([{ ...mockInvitation, state }]));
    const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
    fireEvent.press(await screen.findByRole("button", { name: "Invitation identity details" }));
    await screen.findByText("Owner identity");
    expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Decline invitation" })).toBeNull();
  },
);

test("a stale invitation list cannot republish a replaced node's invitations", async () => {
  let finish:
    | ((result: RuntimeCommandResult<Bindings.GroupInvitationListOutcome>) => void)
    | undefined;
  mockListInvitations.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = renderHook(() => useGroupInvitations());
  await waitFor(() => expect(mockListInvitations).toHaveBeenCalledTimes(1));
  mockSnapshot = { ...mockSnapshot, generationId: 2n };
  mockListInvitations.mockResolvedValue(invitationsResult([]));
  view.rerender({});
  await waitFor(() => expect(view.result.current.loaded).toBe(true));
  await act(async () => finish?.(invitationsResult([mockInvitation])));
  expect(view.result.current.invitations).toEqual([]);
});

type ActionKind = "create" | "invite" | "respond";
async function mountFailedAction(kind: ActionKind, tag: "StorageFull" | "RetryExpired") {
  if (kind === "create")
    mockCreateGroup.mockResolvedValueOnce({
      type: "outcome",
      outcome: Bindings.CreateGroupOutcome[tag].new(),
    });
  if (kind === "invite")
    mockInviteToGroup.mockResolvedValueOnce({
      type: "outcome",
      outcome: Bindings.InviteToGroupOutcome[tag].new(),
    });
  if (kind === "respond")
    mockRespond.mockResolvedValueOnce({
      type: "outcome",
      outcome: Bindings.RespondGroupInvitationOutcome[tag].new(),
    });
  const element =
    kind === "create" ? (
      <CreateGroupScreen />
    ) : kind === "invite" ? (
      <InviteToGroupScreen groupId={mockGroupId} />
    ) : (
      <GroupInvitationScreen invitationId={mockInvitationId} />
    );
  const screen = render(element);
  await waitFor(() => expect(mockActions.listGroupActions).toHaveBeenCalled());
  await act(async () => {});
  if (kind === "create") fireEvent.changeText(screen.getByLabelText("Group name"), "Field crew");
  if (kind === "invite") {
    fireEvent.press(await screen.findByRole("button", { name: "Enter device identity" }));
    fireEvent.changeText(
      await screen.findByLabelText("Device identity"),
      identifierHex(mockRecipient),
    );
  }
  const freshLabel =
    kind === "create"
      ? "Create group"
      : kind === "invite"
        ? "Send invitation"
        : "Decline invitation";
  const retryLabel =
    kind === "create"
      ? "Retry same creation"
      : kind === "invite"
        ? "Retry same invitation"
        : "Retry same decision";
  fireEvent.press(await screen.findByRole("button", { name: freshLabel }));
  await screen.findByText(
    tag === "StorageFull" ? /Group storage is full/ : /This retry has expired/,
  );
  const calls = () =>
    kind === "create"
      ? mockCreateGroup.mock.calls.map(([input]) => input)
      : kind === "invite"
        ? mockInviteToGroup.mock.calls.map(([input]) => input)
        : mockRespond.mock.calls.map(([input]) => input);
  return { screen, element, calls, freshLabel, retryLabel };
}

test.each<ActionKind>(["create", "invite", "respond"])(
  "%s keeps immutable input and ID when storage is full",
  async (kind) => {
    const { screen, calls, retryLabel } = await mountFailedAction(kind, "StorageFull");
    const first = calls()[0];
    if (kind === "create") {
      expect(screen.getByLabelText("Group name").props.editable).toBe(false);
      fireEvent.changeText(screen.getByLabelText("Group name"), "Changed input");
    }
    if (kind === "invite") {
      expect(
        screen.getByRole("button", { name: "Choose another device" }).props.accessibilityState
          .disabled,
      ).toBe(true);
      expect(screen.queryByLabelText("Device identity")).toBeNull();
    }
    fireEvent.press(screen.getByRole("button", { name: retryLabel }));
    await waitFor(() => expect(calls()).toHaveLength(2));
    expect(calls()[1]).toEqual(first);
    expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
    if (kind === "respond") expect(calls()[1]).toMatchObject({ accept: false });
  },
);

test.each<ActionKind>(["create", "invite", "respond"])(
  "%s requires review after retry expiry before preparing a new action",
  async (kind) => {
    const { screen, element, calls, freshLabel, retryLabel } = await mountFailedAction(
      kind,
      "RetryExpired",
    );
    expect(screen.getByRole("button", { name: retryLabel }).props.accessibilityState.disabled).toBe(
      true,
    );
    fireEvent.press(screen.getByRole("button", { name: retryLabel }));
    mockSnapshot = { ...mockSnapshot, generationId: 2n };
    screen.rerender(element);
    expect(screen.getByRole("button", { name: retryLabel }).props.accessibilityState.disabled).toBe(
      true,
    );
    expect(calls()).toHaveLength(1);
    expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
    const nextId = new Uint8Array(32).fill(0x66);
    mockCreateRequestId.mockResolvedValueOnce({
      type: "outcome",
      outcome: Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: nextId }),
    });
    fireEvent.press(
      screen.getByRole("button", {
        name:
          kind === "create"
            ? "I've reviewed saved groups"
            : "I've reviewed membership and invitations",
      }),
    );
    expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: freshLabel }).props.accessibilityState.disabled,
      ).toBe(false),
    );
    fireEvent.press(await screen.findByRole("button", { name: freshLabel }));
    await waitFor(() => expect(calls()).toHaveLength(2));
    expect(calls()[1]).toMatchObject({ clientRequestId: nextId });
  },
);

test("a prepared creation survives unmount and restores its exact title and request ID", async () => {
  mockCreateGroup.mockResolvedValueOnce({ type: "operationFailure", detail: "interrupted" });
  const first = render(<CreateGroupScreen />);
  await waitFor(() => expect(first.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(first.getByLabelText("Group name"), "Prepared on the phone");
  fireEvent.press(first.getByRole("button", { name: "Create group" }));
  await first.findByText(/Confirmation was interrupted/);
  first.unmount();
  const restored = render(<CreateGroupScreen />);
  const retry = await restored.findByRole("button", { name: "Retry same creation" });
  expect(restored.getByLabelText("Group name").props.value).toBe("Prepared on the phone");
  expect(restored.getByLabelText("Group name").props.editable).toBe(false);
  fireEvent.press(retry);
  await restored.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
  expect(mockCreateGroup.mock.calls[1]?.[0]).toEqual(mockCreateGroup.mock.calls[0]?.[0]);
});

test("a lost preparation reply is recovered before any creation is dispatched", async () => {
  const prepare = mockActions.prepareGroupAction.getMockImplementation();
  mockActions.prepareGroupAction.mockImplementationOnce(async (input) => {
    await prepare?.(input);
    throw new Error("bridge died after durable prepare");
  });
  const first = render(<CreateGroupScreen />);
  await waitFor(() => expect(first.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(first.getByLabelText("Group name"), "Durably prepared");
  fireEvent.press(first.getByRole("button", { name: "Create group" }));
  await first.findByText(/Confirmation was interrupted/);
  expect(mockCreateGroup).not.toHaveBeenCalled();
  first.unmount();
  const restored = render(<CreateGroupScreen />);
  fireEvent.press(await restored.findByRole("button", { name: "Retry same creation" }));
  await restored.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
  expect(mockCreateGroup).toHaveBeenCalledWith({
    title: "Durably prepared",
    clientRequestId: mockRequestId,
  });
});

test("a completed creation with a lost reply is restored without creating another group", async () => {
  const execute = mockActions.executeGroupAction.getMockImplementation();
  mockActions.executeGroupAction.mockImplementationOnce(async (id) => {
    await execute?.(id);
    return { type: "operationFailure", detail: "bridge died after completed result" };
  });
  const first = render(<CreateGroupScreen />);
  await waitFor(() => expect(first.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(first.getByLabelText("Group name"), "Already accepted");
  fireEvent.press(first.getByRole("button", { name: "Create group" }));
  await first.findByText(/Confirmation was interrupted/);
  first.unmount();
  const restored = render(<CreateGroupScreen />);
  await restored.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateGroup).toHaveBeenCalledTimes(1);
  expect(mockActions.acknowledgeGroupAction).not.toHaveBeenCalled();
  fireEvent.press(restored.getByRole("link", { name: "Open group conversation" }));
  await waitFor(() =>
    expect(mockActions.acknowledgeGroupAction).toHaveBeenCalledWith({
      ownerIdentity: mockOwner,
      clientRequestId: mockRequestId,
    }),
  );
});

test("an invitation restores its recipient identity after unmount", async () => {
  mockInviteToGroup.mockResolvedValueOnce({ type: "operationFailure", detail: "lost reply" });
  const first = render(<InviteToGroupScreen groupId={mockGroupId} />);
  fireEvent.press(await first.findByRole("button", { name: "Enter device identity" }));
  fireEvent.changeText(
    await first.findByLabelText("Device identity"),
    identifierHex(mockRecipient),
  );
  fireEvent.press(first.getByRole("button", { name: "Send invitation" }));
  await first.findByText(/Confirmation was interrupted/);
  first.unmount();
  const restored = render(<InviteToGroupScreen groupId={mockGroupId} />);
  fireEvent.press(await restored.findByRole("button", { name: "Retry same invitation" }));
  await restored.findByText(
    "Invitation saved. The device still needs to accept and finish joining.",
  );
  expect(mockInviteToGroup.mock.calls[1]?.[0]).toEqual(mockInviteToGroup.mock.calls[0]?.[0]);
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
});

test("a declined invitation stays a decline when the app UI is reopened", async () => {
  mockRespond.mockResolvedValueOnce({ type: "operationFailure", detail: "lost reply" });
  const first = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  fireEvent.press(await first.findByRole("button", { name: "Decline invitation" }));
  await first.findByText(/Confirmation was interrupted/);
  first.unmount();
  const restored = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  fireEvent.press(await restored.findByRole("button", { name: "Retry same decision" }));
  await restored.findByText("Decline saved.");
  expect(mockRespond.mock.calls[1]?.[0]).toEqual({
    invitationId: mockInvitationId,
    accept: false,
    clientRequestId: mockRequestId,
  });
  expect(mockCreateRequestId).toHaveBeenCalledTimes(1);
});

test("failed journal reads block fresh creation until recovery can be checked", async () => {
  mockActions.listGroupActions.mockResolvedValueOnce({
    type: "operationFailure",
    detail: "storage not ready",
  });
  const screen = render(<CreateGroupScreen />);
  await screen.findByText(/Saved actions could not be checked/);
  fireEvent.changeText(screen.getByLabelText("Group name"), "Must wait");
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  expect(mockCreateRequestId).not.toHaveBeenCalled();
  fireEvent.press(screen.getByRole("button", { name: "Reload saved actions" }));
  await waitFor(() => expect(screen.getByLabelText("Group name").props.editable).toBe(true));
  fireEvent.changeText(screen.getByLabelText("Group name"), "Must wait");
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  await screen.findByText("Group created. You are its owner and its only member.");
});

test("Inbox exposes a recovered action without silently executing it", async () => {
  await mockActions.prepareGroupAction({
    ownerIdentity: mockOwner,
    input: Bindings.GroupActionInput.Create.new({
      input: { title: "Restored action", clientRequestId: mockRequestId },
    }),
  });
  const screen = render(<GroupInboxSection />);
  expect(await screen.findByRole("link", { name: "Review saved group creation" })).toBeTruthy();
  expect(screen.getByText("Needs attention")).toBeTruthy();
  expect(mockCreateGroup).not.toHaveBeenCalled();
});

test("a decision remains recoverable after maintenance removes its invitation projection", async () => {
  const input = Bindings.GroupActionInput.Respond.new({
    input: { invitationId: mockInvitationId, clientRequestId: mockRequestId, accept: false },
  });
  await mockActions.prepareGroupAction({ ownerIdentity: mockOwner, input });
  mockListInvitations.mockResolvedValue(invitationsResult([]));
  const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  await screen.findByText("This invitation is not saved on this device.");
  expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Decline invitation" })).toBeNull();
  expect(
    screen.getByText("Saved decision: decline invitation. Retrying keeps that decision unchanged."),
  ).toBeTruthy();
  fireEvent.press(await screen.findByRole("button", { name: "Retry same decision" }));
  await screen.findByText("Decline saved.");
  expect(mockRespond).toHaveBeenCalledWith({
    invitationId: mockInvitationId,
    clientRequestId: mockRequestId,
    accept: false,
  });
  expect(mockCreateRequestId).not.toHaveBeenCalled();
  fireEvent.press(screen.getByRole("link", { name: "Back to Inbox" }));
  await waitFor(() => expect(mockActions.actions.size).toBe(0));
});

test.each(["RetryExpired", "Rejected"] as const)(
  "a %s decision can be reviewed and retired after its invitation disappears",
  async (state) => {
    const input = Bindings.GroupActionInput.Respond.new({
      input: { invitationId: mockInvitationId, clientRequestId: mockRequestId, accept: true },
    });
    mockActions.actions.set(identifierHex(mockRequestId), {
      ownerIdentity: mockOwner,
      input,
      state:
        state === "RetryExpired"
          ? Bindings.GroupActionState.RetryExpired.new()
          : Bindings.GroupActionState.Completed.new({
              result: Bindings.GroupActionResult.Rejected.new({
                reason: Bindings.GroupRejectionReason.InvitationExpired,
                detail: "The invitation no longer permits this action.",
              }),
            }),
    });
    mockListInvitations.mockResolvedValue(invitationsResult([]));
    const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
    const review = await screen.findByRole("button", {
      name: "I've reviewed membership and invitations",
    });
    expect(
      screen.getByRole("button", { name: "Retry same decision" }).props.accessibilityState.disabled,
    ).toBe(true);
    expect(mockActions.actions.size).toBe(1);
    fireEvent.press(review);
    await waitFor(() => expect(mockActions.actions.size).toBe(0));
    expect(mockRespond).not.toHaveBeenCalled();
    expect(mockCreateRequestId).not.toHaveBeenCalled();
    expect(mockActions.discardExpiredGroupAction).toHaveBeenCalledTimes(
      state === "RetryExpired" ? 1 : 0,
    );
    expect(mockActions.acknowledgeGroupAction).toHaveBeenCalledWith({
      ownerIdentity: mockOwner,
      clientRequestId: mockRequestId,
    });
    expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
  },
);

test("opening a joined group acknowledges the displayed completed invitation decision", async () => {
  mockActions.actions.set(identifierHex(mockRequestId), {
    ownerIdentity: mockOwner,
    input: Bindings.GroupActionInput.Respond.new({
      input: { invitationId: mockInvitationId, clientRequestId: mockRequestId, accept: true },
    }),
    state: Bindings.GroupActionState.Completed.new({
      result: Bindings.GroupActionResult.Responded.new(),
    }),
  });
  mockListInvitations.mockResolvedValue(
    invitationsResult([{ ...mockInvitation, state: Bindings.GroupInvitationState.Joined }]),
  );
  const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  const open = await screen.findByRole("link", { name: "Open group conversation" });
  expect(mockActions.acknowledgeGroupAction).not.toHaveBeenCalled();
  fireEvent.press(open);
  await waitFor(() => expect(mockActions.actions.size).toBe(0));
  expect(mockActions.acknowledgeGroupAction).toHaveBeenCalledWith({
    ownerIdentity: mockOwner,
    clientRequestId: mockRequestId,
  });
  expect(mockRespond).not.toHaveBeenCalled();
});

test("a multibyte group name stays editable without admitting an oversized request", async () => {
  const screen = render(<CreateGroupScreen />);
  await act(async () => {});
  fireEvent.changeText(screen.getByLabelText("Group name"), "山".repeat(43));
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  expect(screen.getByText(/Shorten the group name/)).toBeTruthy();
  expect(mockCreateRequestId).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Group name").props.editable).toBe(true);
  fireEvent.changeText(screen.getByLabelText("Group name"), "山".repeat(42));
  fireEvent.press(screen.getByRole("button", { name: "Create group" }));
  await screen.findByText("Group created. You are its owner and its only member.");
  expect(mockCreateGroup.mock.calls[0]?.[0].title).toBe("山".repeat(42));
});

test("manual selection prevents inviting self or duplicating an active invitation", async () => {
  mockListInvitations.mockResolvedValue(invitationsResult([{ ...mockInvitation, outgoing: true }]));
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  fireEvent.press(await screen.findByRole("button", { name: "Enter device identity" }));
  fireEvent.changeText(screen.getByLabelText("Device identity"), identifierHex(mockOwner));
  expect(screen.getByText("This is your device. It is already a member.")).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "Send invitation" }).props.accessibilityState.disabled,
  ).toBe(true);
  fireEvent.changeText(screen.getByLabelText("Device identity"), identifierHex(mockRecipient));
  expect(screen.getByRole("link", { name: "View existing invitation" })).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "Send invitation" }).props.accessibilityState.disabled,
  ).toBe(true);
  expect(mockCreateRequestId).not.toHaveBeenCalled();
  expect(mockInviteToGroup).not.toHaveBeenCalled();
});

test("contact loading failure is retryable without presenting an empty directory", async () => {
  mockListContacts.mockRejectedValueOnce(new Error("unavailable"));
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  await screen.findByText("Saved contacts could not be loaded. Try reloading them.");
  expect(screen.queryByText(/No saved contacts/)).toBeNull();
  fireEvent.press(screen.getByRole("button", { name: "Reload contacts" }));
  await screen.findByText(/No saved contacts. Add one/);
  expect(mockListContacts).toHaveBeenCalledTimes(2);
});

test("typed expired rejection gives a next step while keeping native detail secondary", async () => {
  mockRespond.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.RespondGroupInvitationOutcome.InvalidInput.new({
      reason: Bindings.GroupRejectionReason.InvitationExpired,
      detail: "Offer eligibility expired at native boundary",
    }),
  });
  const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
  fireEvent.press(await screen.findByRole("button", { name: "Accept invitation" }));
  await screen.findByText("This invitation has expired. Ask the group owner for a new invitation.");
  expect(screen.queryByText("Offer eligibility expired at native boundary")).toBeNull();
  expect(
    screen.getByRole("button", { name: "Retry same decision" }).props.accessibilityState.disabled,
  ).toBe(true);
  fireEvent.press(screen.getByRole("button", { name: "Error details" }));
  expect(screen.getByText("Offer eligibility expired at native boundary")).toBeTruthy();
  expect(mockRespond).toHaveBeenCalledTimes(1);
  expect(mockActions.actions.size).toBe(1);
});

test("Inbox keeps incoming decisions visible and moves completed invitations into history", async () => {
  mockListInvitations.mockResolvedValue(
    invitationsResult([
      mockInvitation,
      {
        ...mockInvitation,
        invitationId: new Uint8Array(32).fill(0x99),
        title: "Completed crew",
        state: Bindings.GroupInvitationState.Joined,
      },
    ]),
  );
  const screen = render(<GroupInboxSection />);
  await screen.findByText("Invitation received");
  expect(screen.queryByText("Completed crew")).toBeNull();
  fireEvent.press(screen.getByRole("button", { name: "Invitation history" }));
  expect(screen.getByText("Completed crew")).toBeTruthy();
  expect(screen.getByRole("link", { name: "View invitation for Field crew" })).toBeTruthy();
  expect(screen.getByRole("link", { name: "View invitation for Completed crew" })).toBeTruthy();
});

test("contact search finds a matching alias even when the same identity has an earlier contact", async () => {
  mockListContacts.mockResolvedValue(
    Bindings.ContactListOutcome.Listed.new({
      contacts: [
        {
          destination: new Uint8Array(16).fill(0xaa),
          identity: mockRecipient,
          alias: "First name",
          announcedName: undefined,
          pinned: true,
          isMessaging: true,
        },
        {
          destination: new Uint8Array(16).fill(0xbb),
          identity: mockRecipient,
          alias: "Ranger",
          announcedName: undefined,
          pinned: false,
          isMessaging: false,
        },
      ],
    }),
  );
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  await screen.findByRole("button", { name: "Choose First name" });
  fireEvent.changeText(screen.getByLabelText("Search contacts"), "Ranger");
  fireEvent.press(screen.getByRole("button", { name: "Choose Ranger" }));
  expect(screen.getByText(identifierHex(mockRecipient))).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Send invitation" }));
  await screen.findByText("Invitation saved. The device still needs to accept and finish joining.");
  expect(mockInviteToGroup).toHaveBeenCalledWith({
    clientRequestId: mockRequestId,
    groupId: mockGroupId,
    recipientIdentity: mockRecipient,
  });
});

test.each([
  [
    Bindings.GroupInvitationState.Joining,
    "Waiting for the invited device to finish joining. Keep prns open and both devices connected.",
  ],
  [
    Bindings.GroupInvitationState.Expired,
    "Send a new invitation from the group if this device should still join.",
  ],
] as const)(
  "outgoing invitation %s gives the owner the correct next step",
  async (state, guidance) => {
    mockListInvitations.mockResolvedValue(
      invitationsResult([{ ...mockInvitation, outgoing: true, state }]),
    );
    const screen = render(<GroupInvitationScreen invitationId={mockInvitationId} />);
    expect(await screen.findByText(guidance)).toBeTruthy();
    expect(screen.queryByText(/Your acceptance is saved/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Accept invitation" })).toBeNull();
  },
);

test("mounted Inbox rejects late groups, invitations and actions after an identity change within one generation", async () => {
  let finishGroups:
    | ((value: RuntimeCommandResult<Bindings.GroupConversationListOutcome>) => void)
    | undefined;
  let finishInvitations:
    | ((value: RuntimeCommandResult<Bindings.GroupInvitationListOutcome>) => void)
    | undefined;
  let finishActions:
    | ((value: RuntimeCommandResult<Bindings.GroupActionListOutcome>) => void)
    | undefined;
  mockListGroups.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishGroups = resolve;
      }),
  );
  mockListInvitations.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishInvitations = resolve;
      }),
  );
  mockActions.listGroupActions.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishActions = resolve;
      }),
  );
  const screen = render(<GroupInboxSection />);
  await waitFor(() => expect(finishActions).toBeDefined());
  mockSnapshot = {
    ...mockSnapshot,
    primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
      identityHash: new Uint8Array(16).fill(0x88),
    }),
  };
  mockListGroups.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({ conversations: [] }),
  });
  mockListInvitations.mockResolvedValue(invitationsResult([]));
  screen.rerender(<GroupInboxSection />);
  await screen.findByText("No groups yet.");
  await act(async () => {
    finishGroups?.({
      type: "outcome",
      outcome: Bindings.GroupConversationListOutcome.Ready.new({
        conversations: [mockConversation],
      }),
    });
    finishInvitations?.(invitationsResult([mockInvitation]));
    finishActions?.({
      type: "outcome",
      outcome: Bindings.GroupActionListOutcome.Ready.new({
        actions: [
          {
            ownerIdentity: mockOwner,
            input: Bindings.GroupActionInput.Create.new({
              input: { clientRequestId: mockRequestId, title: "Previous identity" },
            }),
            state: Bindings.GroupActionState.Pending.new(),
          },
        ],
      }),
    });
  });
  expect(screen.queryByText("Field crew")).toBeNull();
  expect(screen.queryByRole("link", { name: "View invitation for Field crew" })).toBeNull();
  expect(screen.queryByRole("link", { name: "Review saved group creation" })).toBeNull();
  expect(mockCreateGroup).not.toHaveBeenCalled();
});

test("late contact labels are rejected after switching identity without changing generation", async () => {
  let finish:
    | ((value: ReturnType<typeof Bindings.ContactListOutcome.Listed.new>) => void)
    | undefined;
  mockListContacts.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const hook = renderHook(() => useGroupContacts());
  await waitFor(() => expect(finish).toBeDefined());
  mockSnapshot = {
    ...mockSnapshot,
    primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
      identityHash: new Uint8Array(16).fill(0x88),
    }),
  };
  hook.rerender({});
  await waitFor(() => expect(hook.result.current.loaded).toBe(true));
  await act(async () =>
    finish?.(
      Bindings.ContactListOutcome.Listed.new({
        contacts: [
          {
            destination: new Uint8Array(16).fill(0xaa),
            identity: mockRecipient,
            alias: "Previous identity contact",
            announcedName: undefined,
            pinned: false,
            isMessaging: true,
          },
        ],
      }),
    ),
  );
  expect(hook.result.current.contacts).toEqual([]);
  expect(hook.result.current.failure).toBeNull();
});

test("scan-to-invite reviews and saves a contact before a separately confirmed directed invitation", async () => {
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  const scan = await screen.findByRole("button", { name: "Scan or paste contact" });
  await waitFor(() => expect(scan.props.accessibilityState.disabled).toBe(false));
  fireEvent.press(scan);
  fireEvent.press(screen.getByText("Scan contact QR"));
  await act(async () => fireEvent.press(screen.getByText("Read test QR")));
  expect(mockPreviewCard).toHaveBeenCalledWith(mockCard.uri);
  expect(mockImportCard).not.toHaveBeenCalled();
  expect(mockInviteToGroup).not.toHaveBeenCalled();
  await act(async () => fireEvent.press(screen.getByText("Save and select contact")));
  expect(screen.getByText("Scanned Ranger")).toBeTruthy();
  expect(mockInviteToGroup).not.toHaveBeenCalled();
  fireEvent.press(screen.getByRole("button", { name: "Send invitation" }));
  await waitFor(() =>
    expect(mockInviteToGroup).toHaveBeenCalledWith({
      groupId: mockGroupId,
      recipientIdentity: mockRecipient,
      clientRequestId: mockRequestId,
    }),
  );
});

test("a scanned existing group member is selected but cannot be invited again", async () => {
  mockListGroups.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({
      conversations: [
        {
          ...mockConversation,
          members: [
            ...mockConversation.members,
            {
              endpointId: new Uint8Array(32).fill(0x55),
              endpointIncarnation: 1n,
              identity: mockRecipient,
              isLocal: false,
              isOwner: false,
            },
          ],
          memberCount: 2,
        },
      ],
    }),
  });
  const screen = render(<InviteToGroupScreen groupId={mockGroupId} />);
  const scan = await screen.findByRole("button", { name: "Scan or paste contact" });
  await waitFor(() => expect(scan.props.accessibilityState.disabled).toBe(false));
  fireEvent.press(scan);
  fireEvent.changeText(screen.getByLabelText("Contact link"), mockCard.uri);
  await act(async () => fireEvent.press(screen.getByText("Review contact card")));
  await act(async () => fireEvent.press(screen.getByText("Save and select contact")));
  expect(screen.getByText("This device is already a member.")).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Send invitation" }));
  expect(mockInviteToGroup).not.toHaveBeenCalled();
});
