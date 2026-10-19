jest.mock("@/native/use-visible-identity-activity", () => ({
  useVisibleIdentityActivity: jest.fn(),
}));

import type {
  GroupMessageListOutcome,
  GroupTextMessage,
  GroupTextSendOutcome,
  ListGroupMessagesInput,
  SendGroupTextInput,
} from "@prns-internal/expo";
import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, renderHook, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";

import type { RuntimeCommandResult } from "@/native/development-runtime-context";
import { useVisibleIdentityActivity } from "@/native/use-visible-identity-activity";
import { createGroupActionTestStore } from "@/testkit/group-action-store";
import { identifierHex } from "./format";
import { GroupConversationScreen } from "./group-conversation-screen";
import { GroupInboxSection } from "./group-inbox-section";
import { useGroupData } from "./use-group-data";
import { useGroupMutation } from "./use-group-mutation";

const mockGroupId = new Uint8Array(32).fill(0x11);
const mockRequestId = new Uint8Array(32).fill(0x77);
const mockConversation: Bindings.GroupConversation = {
  groupId: mockGroupId,
  title: "Development crew",
  revision: 1n,
  memberCount: 3,
  controlHead: new Uint8Array(32).fill(0x66),
  members: [0x44, 0x33, 0x55].map((value) => ({
    endpointId: new Uint8Array(32).fill(value),
    endpointIncarnation: 1n,
    identity: new Uint8Array(16).fill(value),
    isLocal: value === 0x44,
    isOwner: value === 0x55,
  })),
  ownerIdentity: undefined,
  canInvite: false,
  synchronization: Bindings.GroupSynchronizationState.Running.new(),
  progress: { running: true, observed: false, peers: [] },
};
const mockMessage: GroupTextMessage = {
  messageId: new Uint8Array(32).fill(0x22),
  sequence: 1n,
  author: new Uint8Array(16).fill(0x33),
  text: "First group message",
  outgoing: true,
  receivedCount: 1,
  recipientCount: 2,
  membershipChanged: false,
  sentAtMillis: 1_700_000_000_000n,
  clientRequestId: undefined,
};
let mockSnapshot: Pick<
  Bindings.DevelopmentNodeSnapshot,
  "generationId" | "runtime" | "primaryIdentity"
> = {
  generationId: 1n,
  runtime: Bindings.DevelopmentNodeRuntime.Running,
  primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
    identityHash: new Uint8Array(16).fill(0x44),
  }),
};
const mockListGroupConversations = jest.fn(async () => ({
  type: "outcome" as const,
  outcome: Bindings.GroupConversationListOutcome.Ready.new({ conversations: [mockConversation] }),
}));
const mockListGroupInvitations = jest.fn(async () => ({
  type: "outcome" as const,
  outcome: Bindings.GroupInvitationListOutcome.Ready.new({ invitations: [] }),
}));
const mockListGroupMessages = jest.fn(
  async (_input: ListGroupMessagesInput): Promise<RuntimeCommandResult<GroupMessageListOutcome>> =>
    messagesResult([mockMessage]),
);
const mockCreateGroupRequestId = jest.fn(async () => ({
  type: "outcome" as const,
  outcome: Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: mockRequestId }),
}));
const mockSendGroupText = jest.fn(
  async (_input: SendGroupTextInput): Promise<RuntimeCommandResult<GroupTextSendOutcome>> => ({
    type: "outcome",
    outcome: Bindings.GroupTextSendOutcome.Accepted.new({ messageId: mockMessage.messageId }),
  }),
);

let mockCanPublish = true;
const mockActions = createGroupActionTestStore({ send: (input) => mockSendGroupText(input) });

jest.mock("./group-announce-settings", () => ({ GroupAnnounceSettings: () => null }));

jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
}));
const mockListContacts = jest.fn(async () =>
  Bindings.ContactListOutcome.Listed.new({ contacts: [] }),
);
const mockContactRuntime = { listContacts: mockListContacts };
jest.mock("@/native/contact-runtime-context", () => ({
  useContactRuntime: () => ({ runtime: mockContactRuntime }),
}));
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({
    ...mockActions,
    canPublish: mockCanPublish,
    availability: { type: "available", platform: "ios" },
    phase: "ready",
    snapshot: mockSnapshot,
    listGroupConversations: mockListGroupConversations,
    listGroupInvitations: mockListGroupInvitations,
    listGroupMessages: mockListGroupMessages,
    createGroupRequestId: mockCreateGroupRequestId,
    sendGroupText: mockSendGroupText,
  }),
}));

function messagesResult(
  messages: readonly GroupTextMessage[],
  nextBeforeSequence?: bigint,
  revision = 1n,
): RuntimeCommandResult<GroupMessageListOutcome> {
  return {
    type: "outcome",
    outcome: Bindings.GroupMessageListOutcome.Ready.new({
      messages: [...messages],
      revision,
      nextBeforeSequence,
    }),
  };
}

beforeEach(() => {
  jest.clearAllMocks();
  mockCanPublish = true;
  mockActions.actions.clear();
  mockListContacts.mockResolvedValue(Bindings.ContactListOutcome.Listed.new({ contacts: [] }));
  mockSnapshot = {
    generationId: 1n,
    runtime: Bindings.DevelopmentNodeRuntime.Running,
    primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
      identityHash: new Uint8Array(16).fill(0x44),
    }),
  };
  mockListGroupConversations.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({ conversations: [mockConversation] }),
  });
  mockListGroupMessages.mockImplementation(async () => messagesResult([mockMessage]));
  mockCreateGroupRequestId.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: mockRequestId }),
  });
  mockSendGroupText.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupTextSendOutcome.Accepted.new({ messageId: mockMessage.messageId }),
  });
});

test("links the enrolled group from Inbox and labels device receipts accurately", async () => {
  const inbox = render(<GroupInboxSection />);
  expect(await inbox.findByText("Development crew")).toBeTruthy();
  expect(inbox.getByRole("link", { name: "Open group Development crew" })).toBeTruthy();
  inbox.unmount();
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  expect(await screen.findByText("First group message")).toBeTruthy();
  expect(
    screen.getByTestId(`group-record-${identifierHex(mockMessage.messageId)}`).props.children,
  ).toBe("First group message");
  expect(screen.getByText("Waiting for devices · Received by 1 of 2 devices")).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Delivery details" }));
  expect(screen.getByText(/Receipt counts show devices, not people reading/)).toBeTruthy();
  expect(mockListGroupMessages).toHaveBeenCalledWith({
    groupId: mockGroupId,
    beforeSequence: undefined,
    limit: 50,
  });
});

test("acknowledges the loaded incoming cursor without treating outgoing sends as read activity", async () => {
  mockListGroupMessages.mockResolvedValue(
    messagesResult([
      { ...mockMessage, outgoing: true, sequence: 20n },
      { ...mockMessage, outgoing: false, sequence: 7n, messageId: new Uint8Array(32).fill(7) },
    ]),
  );
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  await screen.findAllByText("First group message");
  expect(useVisibleIdentityActivity).toHaveBeenLastCalledWith(
    Bindings.IdentityActivityScope.Group.new({ groupId: mockGroupId }),
    7n,
  );
});

test("a group synchronization failure keeps durable history readable", async () => {
  mockListGroupConversations.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({
      conversations: [
        {
          ...mockConversation,
          synchronization: Bindings.GroupSynchronizationState.Unavailable.new({
            detail: "The group connection needs a restart.",
          }),
        },
      ],
    }),
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  expect(
    await screen.findByText("Needs attention · Group synchronization unavailable"),
  ).toBeTruthy();
  expect(screen.queryByText("The group connection needs a restart.")).toBeNull();
  fireEvent.press(screen.getByRole("button", { name: "Delivery details" }));
  expect(screen.getByText("The group connection needs a restart.")).toBeTruthy();
  expect(screen.getByText("First group message")).toBeTruthy();
  expect(screen.getByText(/Open connection setup/)).toBeTruthy();
});

test("membership changes label interrupted delivery instead of leaving it queued", async () => {
  mockListGroupMessages.mockResolvedValue(
    messagesResult([{ ...mockMessage, membershipChanged: true, receivedCount: 0 }]),
  );
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  expect(
    await screen.findByText("Needs attention · Delivery interrupted by membership change"),
  ).toBeTruthy();
  expect(screen.queryByText("Queued · 0 of 2 devices received")).toBeNull();
});

test("an owner-only group offers invitation before sending", async () => {
  mockListGroupConversations.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({
      conversations: [
        {
          ...mockConversation,
          memberCount: 1,
          ownerIdentity: new Uint8Array(16).fill(7),
          canInvite: true,
        },
      ],
    }),
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  expect(await screen.findByText("Invite someone to start the conversation.")).toBeTruthy();
  expect(screen.getByRole("link", { name: "Invite a device" })).toBeTruthy();
  expect(screen.queryByLabelText("Group message")).toBeNull();
});

test("an ambiguous send retries the same immutable payload and native request ID", async () => {
  mockSendGroupText.mockResolvedValueOnce({ type: "operationFailure", detail: "lost response" });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "A durable send");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  const retry = await screen.findByRole("button", { name: "Retry same message" });
  expect(screen.getByLabelText("Group message").props.editable).toBe(false);
  fireEvent.press(retry);
  await waitFor(() => expect(mockSendGroupText).toHaveBeenCalledTimes(2));
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
  expect(mockSendGroupText.mock.calls.map(([input]) => input)).toEqual([
    { groupId: mockGroupId, clientRequestId: mockRequestId, text: "A durable send" },
    { groupId: mockGroupId, clientRequestId: mockRequestId, text: "A durable send" },
  ]);
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
});

test("history reconciles a lost acceptance without another send", async () => {
  const execute = mockActions.executeGroupAction.getMockImplementation();
  mockActions.executeGroupAction.mockImplementationOnce(async (id) => {
    await execute?.(id);
    return { type: "operationFailure", detail: "lost response" };
  });
  mockSendGroupText.mockImplementationOnce(async () => {
    mockListGroupMessages.mockResolvedValue(
      messagesResult([
        { ...mockMessage, clientRequestId: mockRequestId, text: "Accepted but response lost" },
      ]),
    );
    return {
      type: "outcome",
      outcome: Bindings.GroupTextSendOutcome.Accepted.new({ messageId: mockMessage.messageId }),
    };
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Accepted but response lost");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  expect(await screen.findByText("Accepted but response lost")).toBeTruthy();
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
  expect(screen.queryByRole("button", { name: "Retry same message" })).toBeNull();
  expect(mockSendGroupText).toHaveBeenCalledTimes(1);
});

test("rapid send presses admit only one native mutation", async () => {
  let finish: ((result: RuntimeCommandResult<GroupTextSendOutcome>) => void) | undefined;
  mockSendGroupText.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "One send");
  const button = screen.getByRole("button", { name: "Send to group" });
  fireEvent.press(button);
  fireEvent.press(button);
  await waitFor(() => expect(mockSendGroupText).toHaveBeenCalledTimes(1));
  await act(async () =>
    finish?.({
      type: "outcome",
      outcome: Bindings.GroupTextSendOutcome.Accepted.new({ messageId: mockMessage.messageId }),
    }),
  );
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
});

test("a restarted observer ignores the prior generation's history result", async () => {
  let finish: ((result: RuntimeCommandResult<GroupMessageListOutcome>) => void) | undefined;
  mockListGroupMessages.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = renderHook(() => useGroupData(mockGroupId));
  await waitFor(() => expect(mockListGroupMessages).toHaveBeenCalledTimes(1));
  mockSnapshot = { ...mockSnapshot, generationId: 2n };
  mockListGroupMessages.mockResolvedValue(
    messagesResult([{ ...mockMessage, text: "Fresh generation" }]),
  );
  view.rerender({});
  await waitFor(() => expect(view.result.current.messages[0]?.text).toBe("Fresh generation"));
  await act(async () => finish?.(messagesResult([{ ...mockMessage, text: "Stale generation" }])));
  expect(view.result.current.messages.map((message) => message.text)).toEqual(["Fresh generation"]);
});

test("pages bounded history and refreshes receipt changes in older loaded messages", async () => {
  let rows = Array.from({ length: 125 }, (_, index) => ({
    ...mockMessage,
    messageId: new Uint8Array(32).fill(index),
    sequence: BigInt(125 - index),
  }));
  mockListGroupMessages.mockImplementation(async (input) => {
    const remaining = rows.filter(
      (row) => input.beforeSequence === undefined || row.sequence < input.beforeSequence,
    );
    const page = remaining.slice(0, input.limit);
    return messagesResult(page, remaining.length > input.limit ? page.at(-1)?.sequence : undefined);
  });
  const view = renderHook(() => useGroupData(mockGroupId));
  await waitFor(() => expect(view.result.current.messages).toHaveLength(50));
  await act(async () => view.result.current.loadOlder());
  await act(async () => view.result.current.loadOlder());
  expect(view.result.current.messages).toHaveLength(125);
  expect(view.result.current.hasMore).toBe(false);
  rows = rows.map((row) => ({ ...row, receivedCount: 2 }));
  await act(async () => view.result.current.refresh());
  expect(view.result.current.messages).toHaveLength(125);
  expect(view.result.current.messages.at(-1)?.receivedCount).toBe(2);
  expect(mockListGroupMessages.mock.calls.every(([input]) => input.limit <= 50)).toBe(true);
  expect(
    new Set(view.result.current.messages.map((message) => identifierHex(message.messageId))).size,
  ).toBe(125);
});

test("a conversation change does not clear a new group's composer when an old send finishes", async () => {
  let finish: ((result: RuntimeCommandResult<GroupTextSendOutcome>) => void) | undefined;
  mockSendGroupText.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Old group");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await waitFor(() => expect(mockSendGroupText).toHaveBeenCalledTimes(1));
  const nextGroup = new Uint8Array(32).fill(0x88);
  mockListGroupConversations.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.GroupConversationListOutcome.Ready.new({
      conversations: [{ ...mockConversation, groupId: nextGroup }],
    }),
  });
  screen.rerender(<GroupConversationScreen groupId={nextGroup} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "New group draft");
  await act(async () =>
    finish?.({
      type: "outcome",
      outcome: Bindings.GroupTextSendOutcome.Accepted.new({ messageId: mockMessage.messageId }),
    }),
  );
  expect(screen.getByLabelText("Group message").props.value).toBe("New group draft");
});

test("full storage preserves the same message and retry ID", async () => {
  mockSendGroupText.mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.GroupTextSendOutcome.StorageFull.new(),
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Keep this exact message");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await screen.findByText(/Group storage is full/);
  expect(screen.getByLabelText("Group message").props.editable).toBe(false);
  fireEvent.changeText(screen.getByLabelText("Group message"), "Changed draft");
  fireEvent.press(screen.getByRole("button", { name: "Retry same message" }));
  await waitFor(() => expect(mockSendGroupText).toHaveBeenCalledTimes(2));
  expect(mockSendGroupText.mock.calls[1]?.[0]).toEqual(mockSendGroupText.mock.calls[0]?.[0]);
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
});

test("expired message retry cannot mint a new ID until explicit history review", async () => {
  mockSendGroupText.mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.GroupTextSendOutcome.RetryExpired.new(),
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Possibly sent already");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await screen.findByText(/This retry has expired/);
  expect(
    screen.getByRole("button", { name: "Retry same message" }).props.accessibilityState.disabled,
  ).toBe(true);
  fireEvent.press(screen.getByRole("button", { name: "Retry same message" }));
  mockSnapshot = { ...mockSnapshot, generationId: 2n };
  screen.rerender(<GroupConversationScreen groupId={mockGroupId} />);
  await screen.findByText(/This retry has expired/);
  expect(mockSendGroupText).toHaveBeenCalledTimes(1);
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
  const nextId = new Uint8Array(32).fill(0x88);
  mockCreateGroupRequestId.mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: nextId }),
  });
  fireEvent.press(screen.getByRole("button", { name: "I've reviewed saved messages" }));
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Send to group" }).props.accessibilityState.disabled,
    ).toBe(false),
  );
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await waitFor(() => expect(mockSendGroupText).toHaveBeenCalledTimes(2));
  expect(mockSendGroupText.mock.calls[1]?.[0]).toMatchObject({ clientRequestId: nextId });
});

test("loading older history removes retired cached rows and preserves pagination depth", async () => {
  let rows = Array.from({ length: 125 }, (_, index) => ({
    ...mockMessage,
    messageId: new Uint8Array(32).fill(index),
    sequence: BigInt(125 - index),
    text: `row ${index}`,
  }));
  let revision = 1n;
  mockListGroupMessages.mockImplementation(async (input) => {
    const remaining = rows.filter(
      (row) => input.beforeSequence === undefined || row.sequence < input.beforeSequence,
    );
    const page = remaining.slice(0, input.limit);
    return messagesResult(
      page,
      remaining.length > input.limit ? page.at(-1)?.sequence : undefined,
      revision,
    );
  });
  const view = renderHook(() => useGroupData(mockGroupId));
  await waitFor(() => expect(view.result.current.messages).toHaveLength(50));
  rows = rows.filter((row) => row.text !== "row 10");
  revision = 2n;
  await act(async () => view.result.current.loadOlder());
  expect(view.result.current.messages).toHaveLength(100);
  expect(view.result.current.messages.some((row) => row.text === "row 10")).toBe(false);
  expect(view.result.current.hasMore).toBe(true);
  rows = rows.filter((row) => row.text !== "row 90");
  revision = 3n;
  await act(async () => view.result.current.refresh());
  expect(view.result.current.messages).toHaveLength(100);
  expect(view.result.current.messages.some((row) => row.text === "row 90")).toBe(false);
  await act(async () => view.result.current.loadOlder());
  expect(view.result.current.messages).toHaveLength(123);
  expect(view.result.current.hasMore).toBe(false);
});

test("a revision change between pages restarts the authoritative read", async () => {
  const old = Array.from({ length: 60 }, (_, index) => ({
    ...mockMessage,
    messageId: new Uint8Array(32).fill(index),
    sequence: BigInt(60 - index),
  }));
  mockListGroupMessages.mockImplementation(async () => messagesResult(old.slice(0, 50), 11n));
  const view = renderHook(() => useGroupData(mockGroupId));
  await waitFor(() => expect(view.result.current.messages).toHaveLength(50));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(old.slice(0, 50), 11n, 1n));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(old.slice(50), undefined, 2n));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(old.slice(0, 1), undefined, 2n));
  await act(async () => view.result.current.loadOlder());
  expect(view.result.current.messages).toHaveLength(1);
  expect(view.result.current.hasMore).toBe(false);
  expect(view.result.current.failure).toBeNull();
});

test("repeated revision changes clear known-stale rows and reset pagination safely", async () => {
  const rows = Array.from({ length: 60 }, (_, index) => ({
    ...mockMessage,
    messageId: new Uint8Array(32).fill(index),
    sequence: BigInt(60 - index),
  }));
  mockListGroupMessages.mockResolvedValue(messagesResult(rows.slice(0, 50), 11n));
  const view = renderHook(() => useGroupData(mockGroupId));
  await waitFor(() => expect(view.result.current.messages).toHaveLength(50));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(rows.slice(0, 50), 11n, 1n));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(rows.slice(50), undefined, 2n));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(rows.slice(0, 50), 11n, 2n));
  mockListGroupMessages.mockResolvedValueOnce(messagesResult(rows.slice(50), undefined, 3n));
  await act(async () => view.result.current.loadOlder());
  expect(view.result.current.messages).toHaveLength(0);
  expect(view.result.current.hasMore).toBe(false);
  expect(view.result.current.failure).toMatch(/changed while loading/);
  mockListGroupMessages.mockResolvedValue(messagesResult(rows.slice(0, 2), undefined, 3n));
  await act(async () => view.result.current.refresh());
  expect(view.result.current.messages).toHaveLength(2);
  expect(view.result.current.failure).toBeNull();
});

test("an uncertain message survives unmount and resumes the same immutable send", async () => {
  mockSendGroupText.mockResolvedValueOnce({ type: "operationFailure", detail: "lost reply" });
  const first = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await first.findByLabelText("Group message"), "Survives reopening");
  fireEvent.press(first.getByRole("button", { name: "Send to group" }));
  await first.findByText(/Confirmation was interrupted/);
  first.unmount();
  const restored = render(<GroupConversationScreen groupId={mockGroupId} />);
  const retry = await restored.findByRole("button", { name: "Retry same message" });
  expect(restored.getByLabelText("Group message").props.value).toBe("Survives reopening");
  expect(restored.getByLabelText("Group message").props.editable).toBe(false);
  fireEvent.press(retry);
  await waitFor(() => expect(restored.getByLabelText("Group message").props.value).toBe(""));
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
  expect(mockSendGroupText.mock.calls[1]?.[0]).toEqual(mockSendGroupText.mock.calls[0]?.[0]);
});

test("reopening after a completed send lost its reply restores acceptance without resubmission", async () => {
  const execute = mockActions.executeGroupAction.getMockImplementation();
  mockActions.executeGroupAction.mockImplementationOnce(async (id) => {
    await execute?.(id);
    return { type: "operationFailure", detail: "bridge died" };
  });
  const first = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await first.findByLabelText("Group message"), "Committed before restart");
  fireEvent.press(first.getByRole("button", { name: "Send to group" }));
  await first.findByText(/Confirmation was interrupted/);
  first.unmount();
  const restored = render(<GroupConversationScreen groupId={mockGroupId} />);
  await restored.findByText("Saved locally.");
  await waitFor(() => expect(restored.getByLabelText("Group message").props.value).toBe(""));
  expect(mockSendGroupText).toHaveBeenCalledTimes(1);
  expect(mockCreateGroupRequestId).toHaveBeenCalledTimes(1);
});

test("oversized multibyte drafts remain editable without creating a saved action", async () => {
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  const oversized = "😀".repeat(600);
  fireEvent.changeText(await screen.findByLabelText("Group message"), oversized);
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await screen.findByText(/Message too long/);
  expect(mockCreateGroupRequestId).not.toHaveBeenCalled();
  expect(mockActions.prepareGroupAction).not.toHaveBeenCalled();
  expect(mockSendGroupText).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Group message").props.editable).toBe(true);
  expect(screen.getByLabelText("Group message").props.value).toBe(oversized);
  fireEvent.changeText(screen.getByLabelText("Group message"), "Fits");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
  expect(mockSendGroupText).toHaveBeenCalledWith({
    groupId: mockGroupId,
    clientRequestId: mockRequestId,
    text: "Fits",
  });
});

test("message preparation never claims durability before the native commit", async () => {
  let finish: (() => void) | undefined;
  const prepare = mockActions.prepareGroupAction.getMockImplementation();
  mockActions.prepareGroupAction.mockImplementationOnce(async (input) => {
    await new Promise<void>((resolve) => {
      finish = resolve;
    });
    const response = await prepare?.(input);
    if (response === undefined) throw new Error("missing prepare");
    return response;
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Not yet durable");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await screen.findByText("Preparing this action…");
  expect(screen.queryByText(/Saved locally/)).toBeNull();
  expect(mockSendGroupText).not.toHaveBeenCalled();
  await act(async () => finish?.());
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
});

test("a completed send with an acknowledgement failure can resume composing without sending again", async () => {
  mockActions.acknowledgeGroupAction.mockResolvedValueOnce({
    type: "operationFailure",
    detail: "bridge lost",
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Already saved");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  fireEvent.press(await screen.findByRole("button", { name: "Continue composing" }));
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
  expect(mockSendGroupText).toHaveBeenCalledTimes(1);
  expect(mockActions.acknowledgeGroupAction).toHaveBeenCalledTimes(2);
});

test.each(["result", "failure"])(
  "a delayed journal %s cannot restore a completed action after acknowledgement",
  async (reply) => {
    const input = Bindings.GroupActionInput.Send.new({
      input: { groupId: mockGroupId, clientRequestId: mockRequestId, text: "Already saved" },
    });
    const action: Bindings.GroupAction = {
      ownerIdentity: new Uint8Array(16).fill(0x44),
      input,
      state: Bindings.GroupActionState.Completed.new({
        result: Bindings.GroupActionResult.Sent.new({ messageId: mockMessage.messageId }),
      }),
    };
    mockActions.actions.set(identifierHex(mockRequestId), action);
    const view = renderHook(() => useGroupMutation("Send", mockGroupId));
    await waitFor(() => expect(view.result.current.result?.tag).toBe("Sent"));
    let finish: (() => void) | undefined;
    mockActions.listGroupActions.mockImplementationOnce(async () => {
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      if (reply === "failure") throw new Error("late storage failure");
      return {
        type: "outcome",
        outcome: Bindings.GroupActionListOutcome.Ready.new({ actions: [action] }),
      };
    });
    let read: Promise<void> | undefined;
    act(() => {
      read = view.result.current.refresh();
    });
    await act(async () => {
      await view.result.current.acknowledge();
    });
    expect(view.result.current.action).toBeNull();
    await act(async () => {
      finish?.();
      await read;
    });
    expect(view.result.current.action).toBeNull();
    expect(mockActions.actions.size).toBe(0);
    expect(view.result.current.ready).toBe(true);
  },
);

test("a delayed pending journal read cannot replace a newly confirmed native result", async () => {
  const input = Bindings.GroupActionInput.Send.new({
    input: { groupId: mockGroupId, clientRequestId: mockRequestId, text: "Finishes after read" },
  });
  await mockActions.prepareGroupAction({ ownerIdentity: new Uint8Array(16).fill(0x44), input });
  const pending = [...mockActions.actions.values()];
  const view = renderHook(() => useGroupMutation("Send", mockGroupId));
  await waitFor(() => expect(view.result.current.ready).toBe(true));
  let completeSend: ((result: RuntimeCommandResult<GroupTextSendOutcome>) => void) | undefined;
  mockSendGroupText.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        completeSend = resolve;
      }),
  );
  let submission: Promise<void> | undefined;
  act(() => {
    submission = view.result.current.submit(() => input);
  });
  await waitFor(() => expect(mockSendGroupText).toHaveBeenCalledTimes(1));
  let finishRead: (() => void) | undefined;
  mockActions.listGroupActions.mockImplementationOnce(async () => {
    await new Promise<void>((resolve) => {
      finishRead = resolve;
    });
    return {
      type: "outcome",
      outcome: Bindings.GroupActionListOutcome.Ready.new({ actions: pending }),
    };
  });
  let read: Promise<void> | undefined;
  act(() => {
    read = view.result.current.refresh();
  });
  await act(async () => {
    completeSend?.({
      type: "outcome",
      outcome: Bindings.GroupTextSendOutcome.Accepted.new({ messageId: mockMessage.messageId }),
    });
    await submission;
  });
  expect(view.result.current.result?.tag).toBe("Sent");
  await act(async () => {
    finishRead?.();
    await read;
  });
  expect(view.result.current.result?.tag).toBe("Sent");
});

test("a stopped node can save a message locally while synchronization waits", async () => {
  mockSnapshot = { ...mockSnapshot, runtime: Bindings.DevelopmentNodeRuntime.Stopped };
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Save while offline");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await screen.findByText("Saved locally.");
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
  expect(screen.getByRole("link", { name: "Reconnect to synchronize" })).toBeTruthy();
  expect(mockSendGroupText).toHaveBeenCalledTimes(1);
  expect(mockActions.prepareGroupAction).toHaveBeenCalledWith({
    ownerIdentity: new Uint8Array(16).fill(0x44),
    input: Bindings.GroupActionInput.Send.new({
      input: { groupId: mockGroupId, clientRequestId: mockRequestId, text: "Save while offline" },
    }),
  });
});

test("a replacement identity cannot inherit another identity's unsent composer draft", async () => {
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(
    await screen.findByLabelText("Group message"),
    "Private draft for old identity",
  );
  mockSnapshot = {
    ...mockSnapshot,
    generationId: 2n,
    primaryIdentity: Bindings.PrimaryIdentityState.Present.new({
      identityHash: new Uint8Array(16).fill(0x55),
    }),
  };
  screen.rerender(<GroupConversationScreen groupId={mockGroupId} />);
  await waitFor(() => expect(screen.getByLabelText("Group message").props.editable).toBe(true));
  expect(screen.getByLabelText("Group message").props.value).toBe("");
  expect(mockActions.prepareGroupAction).not.toHaveBeenCalled();
});

test("new group actions wait for a known device identity", async () => {
  mockSnapshot = { ...mockSnapshot, primaryIdentity: Bindings.PrimaryIdentityState.Missing.new() };
  const view = renderHook(() => useGroupMutation("Send", mockGroupId));
  await waitFor(() => expect(view.result.current.ready).toBe(true));
  await act(async () => {
    await view.result.current.submit((clientRequestId) =>
      Bindings.GroupActionInput.Send.new({
        input: { groupId: mockGroupId, clientRequestId, text: "No owner" },
      }),
    );
  });
  expect(view.result.current.failure).toMatch(/identity is not ready/);
  expect(mockActions.prepareGroupAction).not.toHaveBeenCalled();
  expect(mockCreateGroupRequestId).not.toHaveBeenCalled();
});

test("roster comes from saved membership and contact labels while progress remains secondary", async () => {
  mockListContacts.mockResolvedValue(
    Bindings.ContactListOutcome.Listed.new({
      contacts: [
        {
          destination: new Uint8Array(16).fill(0x99),
          identity: mockMessage.author,
          alias: "Ranger",
          announcedName: undefined,
          pinned: true,
          isMessaging: true,
        },
      ],
    }),
  );
  mockListGroupMessages.mockResolvedValue(messagesResult([{ ...mockMessage, outgoing: false }]));
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  expect(await screen.findByText("Ranger")).toBeTruthy();
  expect(screen.getByTestId("group-compose-input")).toBeTruthy();
  expect(screen.queryByText("Members")).toBeNull();
  expect(screen.queryByText("Delivery activity")).toBeNull();
  fireEvent.press(screen.getByRole("button", { name: "Group details" }));
  expect(await screen.findByText("Members")).toBeTruthy();
  expect(screen.getAllByText("Ranger")).toHaveLength(2);
  expect(screen.getByText("You")).toBeTruthy();
  expect(screen.getByText("Owner")).toBeTruthy();
  expect(screen.getByText(/Membership does not mean a device is online/)).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Membership details" }));
  expect(screen.getByText(identifierHex(mockConversation.controlHead))).toBeTruthy();
  fireEvent.press(screen.getByRole("button", { name: "Device identity · Ranger" }));
  expect(screen.getByText(identifierHex(mockMessage.author))).toBeTruthy();
  expect(
    screen.getByTestId(`group-record-${identifierHex(mockMessage.messageId)}`).props.children,
  ).toBe("First group message");
  expect(
    screen.getByTestId(`group-receipt-${identifierHex(mockMessage.messageId)}`).props.children,
  ).toBe("Received on this device");
});

test("authoritative preparation rejection still unlocks an unadmitted draft", async () => {
  mockActions.prepareGroupAction.mockResolvedValueOnce({
    type: "outcome",
    outcome: Bindings.GroupActionOutcome.InvalidInput.new({
      detail: "The native action was not admitted.",
    }),
  });
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  fireEvent.changeText(await screen.findByLabelText("Group message"), "Original draft");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await screen.findByText("The native action was not admitted.");
  expect(mockSendGroupText).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Group message").props.editable).toBe(true);
  expect(screen.getByLabelText("Group message").props.value).toBe("Original draft");
  fireEvent.changeText(screen.getByLabelText("Group message"), "Corrected draft");
  fireEvent.press(screen.getByRole("button", { name: "Send to group" }));
  await waitFor(() => expect(screen.getByLabelText("Group message").props.value).toBe(""));
  expect(mockSendGroupText).toHaveBeenCalledWith({
    groupId: mockGroupId,
    clientRequestId: mockRequestId,
    text: "Corrected draft",
  });
});

test("a paused identity retains group history but cannot publish a new message", async () => {
  mockCanPublish = false;
  const screen = render(<GroupConversationScreen groupId={mockGroupId} />);
  expect(await screen.findByText("First group message")).toBeTruthy();
  expect(screen.queryByText(/Messages can be saved while this device is offline/)).toBeNull();
  expect(screen.getByLabelText("Group message").props.editable).toBe(false);
  const mutation = renderHook(() => useGroupMutation("Send", mockGroupId));
  await act(async () => {
    await mutation.result.current.submit((clientRequestId) =>
      Bindings.GroupActionInput.Send.new({
        input: { groupId: mockGroupId, clientRequestId, text: "Paused send" },
      }),
    );
  });
  expect(mockActions.prepareGroupAction).not.toHaveBeenCalled();
  expect(mockCreateGroupRequestId).not.toHaveBeenCalled();
  expect(mockSendGroupText).not.toHaveBeenCalled();
});
