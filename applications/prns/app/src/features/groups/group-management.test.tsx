import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, render, renderHook, waitFor } from "@testing-library/react-native";

import { createGroupActionTestStore } from "@/testkit/group-action-store";
import { identifierHex } from "./format";
import { GroupInvitationManagement } from "./group-invitation-management";
import { GroupManagement } from "./group-management";
import { useGroupIdentityScope, useGroupMutation } from "./use-group-mutation";

const groupId = new Uint8Array(32).fill(1);
const invitationId = new Uint8Array(32).fill(2);
const controlHead = new Uint8Array(32).fill(3);
const owner = new Uint8Array(16).fill(4);
const member = {
  endpointId: new Uint8Array(32).fill(5),
  endpointIncarnation: 7n,
  identity: new Uint8Array(16).fill(6),
  isLocal: false,
  isOwner: false,
};
const conversation: Bindings.GroupConversation = {
  groupId,
  title: "Field group",
  revision: 1n,
  memberCount: 2,
  controlHead,
  members: [
    {
      endpointId: new Uint8Array(32).fill(4),
      endpointIncarnation: 1n,
      identity: owner,
      isLocal: true,
      isOwner: true,
    },
    member,
  ],
  ownerIdentity: owner,
  canInvite: true,
  canSend: true,
  canRemoveMembers: true,
  canLeave: false,
  canClose: true,
  canDeleteLocal: false,
  lifecycle: Bindings.GroupLifecycle.Active,
  synchronization: Bindings.GroupSynchronizationState.Stopped.new(),
  progress: { running: false, observed: false, peers: [] },
};
const invitation: Bindings.GroupInvitation = {
  invitationId,
  groupId,
  title: conversation.title,
  inviterIdentity: owner,
  recipientIdentity: member.identity,
  expiresAtMillis: 1_900_000_000_000n,
  outgoing: true,
  state: Bindings.GroupInvitationState.Offered,
  waitingForDelivery: false,
  canCancel: true,
  canDismiss: false,
};
let mockIdentity = owner;
let mockCanPublish = true;
let mockRequestNumber = 10;
let mockGeneration = 1n;
const mockManage = jest.fn(
  async (_input: Bindings.GroupActionInput): Promise<Bindings.GroupActionResult> =>
    Bindings.GroupActionResult.RemovedMember.new(),
);
const mockActions = createGroupActionTestStore({ manage: (input) => mockManage(input) });
const mockRefresh = jest.fn(async () => {});
function mockRuntime() {
  return {
    ...mockActions,
    canPublish: mockCanPublish,
    availability: { type: "available", platform: "ios" },
    phase: "ready",
    ownerKey: identifierHex(mockIdentity),
    snapshot: {
      generationId: mockGeneration,
      runtime: Bindings.DevelopmentNodeRuntime.Stopped,
      primaryIdentity: Bindings.PrimaryIdentityState.Present.new({ identityHash: mockIdentity }),
    },
    createGroupRequestId: async () => ({
      type: "outcome",
      outcome: Bindings.GroupRequestIdOutcome.Ready.new({
        clientRequestId: new Uint8Array(32).fill(mockRequestNumber++),
      }),
    }),
  };
}
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => mockRuntime(),
}));

function Management({
  current = conversation,
}: {
  readonly current?: Bindings.GroupConversation | null;
}) {
  return (
    <GroupManagement
      key={useGroupIdentityScope(identifierHex(groupId))}
      groupId={groupId}
      conversation={current ?? undefined}
      contacts={[]}
      visible
      onSettled={mockRefresh}
    />
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  mockIdentity = owner;
  mockCanPublish = true;
  mockRequestNumber = 10;
  mockGeneration = 1n;
  mockActions.actions.clear();
  mockManage.mockResolvedValue(Bindings.GroupActionResult.RemovedMember.new());
});

test("removing a device requires confirmation and binds the exact enrollment and membership version", async () => {
  const view = render(<Management />);
  const remove = await view.findByRole("button", { name: "Remove Device 06060606" });
  await waitFor(() => expect(remove.props.accessibilityState.disabled).toBe(false));
  fireEvent.press(remove);
  expect(mockManage).not.toHaveBeenCalled();
  view.rerender(
    <Management
      current={{
        ...conversation,
        controlHead: new Uint8Array(32).fill(8),
        members: [{ ...member, endpointIncarnation: 8n }],
      }}
    />,
  );
  const confirm = view.getByRole("button", { name: "Confirm remove device 06060606" });
  fireEvent.press(confirm);
  fireEvent.press(confirm);
  await waitFor(() => expect(mockManage).toHaveBeenCalledTimes(1));
  expect(mockManage).toHaveBeenCalledWith(
    Bindings.GroupActionInput.RemoveMember.new({
      input: {
        groupId,
        clientRequestId: new Uint8Array(32).fill(10),
        endpointId: member.endpointId,
        endpointIncarnation: 7n,
        expectedControlHead: controlHead,
      },
    }),
  );
  expect(view.queryByRole("button", { name: "Remove You" })).toBeNull();
});

test("an uncertain removal restores its exact target after unmount even when the group is absent", async () => {
  mockManage.mockRejectedValueOnce(new Error("Reply lost"));
  const first = render(<Management />);
  await waitFor(() =>
    expect(
      first.getByRole("button", { name: "Remove Device 06060606" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(first.getByRole("button", { name: "Remove Device 06060606" }));
  fireEvent.press(first.getByRole("button", { name: "Confirm remove device 06060606" }));
  await first.findByText(/Confirmation was interrupted/);
  const original = mockManage.mock.calls[0]?.[0];
  first.unmount();
  const restored = render(<Management current={null} />);
  fireEvent.press(await restored.findByRole("button", { name: "Retry same member removal" }));
  await waitFor(() => expect(mockManage).toHaveBeenCalledTimes(2));
  expect(mockManage.mock.calls[1]?.[0]).toEqual(original);
  expect(mockRequestNumber).toBe(11);
});

test("switching identities clears a pending destructive confirmation", async () => {
  const view = render(<Management />);
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Close group for everyone" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Close group for everyone" }));
  mockIdentity = new Uint8Array(16).fill(9);
  view.rerender(<Management current={null} />);
  await act(async () => {});
  expect(view.queryByRole("button", { name: "Confirm close group for everyone" })).toBeNull();
  expect(mockManage).not.toHaveBeenCalled();
});

test.each(["identity", "generation"])(
  "local deletion confirmation is fenced by %s even when both owners have the same group",
  async (change) => {
    const closed = {
      ...conversation,
      lifecycle: Bindings.GroupLifecycle.Closed,
      canRemoveMembers: false,
      canClose: false,
      canDeleteLocal: true,
    };
    const element = (
      <GroupManagement
        groupId={groupId}
        conversation={closed}
        contacts={[]}
        visible
        onSettled={mockRefresh}
      />
    );
    const view = render(element);
    await waitFor(() =>
      expect(
        view.getByRole("button", { name: "Delete group from this device" }).props.accessibilityState
          .disabled,
      ).toBe(false),
    );
    fireEvent.press(view.getByRole("button", { name: "Delete group from this device" }));
    expect(
      view.getByRole("button", { name: "Confirm delete group from this device" }),
    ).toBeTruthy();
    if (change === "identity") mockIdentity = new Uint8Array(16).fill(9);
    else mockGeneration = 2n;
    view.rerender(
      <GroupManagement
        groupId={groupId}
        conversation={closed}
        contacts={[]}
        visible
        onSettled={mockRefresh}
      />,
    );
    expect(
      view.queryByRole("button", { name: "Confirm delete group from this device" }),
    ).toBeNull();
    await waitFor(() =>
      expect(
        view.getByRole("button", { name: "Delete group from this device" }).props.accessibilityState
          .disabled,
      ).toBe(false),
    );
    expect(mockManage).not.toHaveBeenCalled();
    expect(mockActions.actions.size).toBe(0);
  },
);

test("native capabilities expose leave to a member and forbid owner-only changes", async () => {
  const view = render(
    <Management
      current={{ ...conversation, canRemoveMembers: false, canClose: false, canLeave: true }}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Leave group" }).props.accessibilityState.disabled,
    ).toBe(false),
  );
  expect(view.queryByRole("button", { name: "Close group for everyone" })).toBeNull();
  expect(view.queryByRole("button", { name: "Remove Device 06060606" })).toBeNull();
  expect(view.queryByRole("button", { name: "Delete group from this device" })).toBeNull();
});

test("closing a group does not automatically delete local history", async () => {
  mockManage.mockResolvedValue(Bindings.GroupActionResult.ClosedGroup.new());
  const view = render(<Management />);
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Close group for everyone" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Close group for everyone" }));
  fireEvent.press(view.getByRole("button", { name: "Confirm close group for everyone" }));
  await view.findByText("Saved group closure");
  expect(mockManage).toHaveBeenCalledTimes(1);
  expect(mockManage.mock.calls[0]?.[0]).toEqual(
    Bindings.GroupActionInput.CloseGroup.new({
      input: {
        groupId,
        clientRequestId: new Uint8Array(32).fill(10),
        expectedControlHead: controlHead,
      },
    }),
  );
});

test("a paused identity cannot confirm a new destructive action", async () => {
  const view = render(<Management />);
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Close group for everyone" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Close group for everyone" }));
  mockCanPublish = false;
  view.rerender(<Management />);
  fireEvent.press(view.getByRole("button", { name: "Confirm close group for everyone" }));
  await act(async () => {});
  expect(mockManage).not.toHaveBeenCalled();
});

test("an outgoing invitation can be cancelled with explicit confirmation", async () => {
  mockManage.mockResolvedValue(Bindings.GroupActionResult.CancelledInvitation.new());
  const view = render(
    <GroupInvitationManagement
      invitationId={invitationId}
      invitation={invitation}
      onSettled={mockRefresh}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Cancel invitation" }).props.accessibilityState.disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Cancel invitation" }));
  expect(mockManage).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Confirm cancel invitation" }));
  await waitFor(() =>
    expect(mockManage).toHaveBeenCalledWith(
      Bindings.GroupActionInput.CancelInvitation.new({
        input: { invitationId, clientRequestId: new Uint8Array(32).fill(10) },
      }),
    ),
  );
});

test("terminal invitation dismissal recovers its completed result after the invitation disappears", async () => {
  mockManage.mockResolvedValue(Bindings.GroupActionResult.DismissedInvitation.new());
  const view = render(
    <GroupInvitationManagement
      invitationId={invitationId}
      invitation={{
        ...invitation,
        state: Bindings.GroupInvitationState.Cancelled,
        canCancel: false,
        canDismiss: true,
      }}
      onSettled={mockRefresh}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Remove invitation from list" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Remove invitation from list" }));
  fireEvent.press(view.getByRole("button", { name: "Confirm remove invitation from list" }));
  await view.findByText("Invitation removed from your list.");
  view.unmount();
  const restored = render(
    <GroupInvitationManagement
      invitationId={invitationId}
      invitation={undefined}
      onSettled={mockRefresh}
    />,
  );
  await restored.findByText("Invitation removed from your list.");
  expect(mockManage).toHaveBeenCalledTimes(1);
});

test("cancellation that loses the race with enrollment reports member removal without submitting it", async () => {
  mockManage.mockResolvedValue(
    Bindings.GroupActionResult.Rejected.new({
      reason: Bindings.GroupRejectionReason.AlreadyJoined,
      detail: "Enrollment already committed",
    }),
  );
  const view = render(
    <GroupInvitationManagement
      invitationId={invitationId}
      invitation={invitation}
      onSettled={mockRefresh}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Cancel invitation" }).props.accessibilityState.disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Cancel invitation" }));
  fireEvent.press(view.getByRole("button", { name: "Confirm cancel invitation" }));
  await view.findByText(
    "This device has already been enrolled. Remove the member from Group details instead of cancelling its invitation.",
  );
  expect(mockManage).toHaveBeenCalledTimes(1);
  expect(mockManage.mock.calls[0]?.[0].tag).toBe("CancelInvitation");
});

test("local deletion retains a recoverable result after group history disappears", async () => {
  mockManage.mockResolvedValue(Bindings.GroupActionResult.DeletedLocalGroup.new());
  const view = render(
    <Management
      current={{
        ...conversation,
        lifecycle: Bindings.GroupLifecycle.Closed,
        canRemoveMembers: false,
        canClose: false,
        canDeleteLocal: true,
      }}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Delete group from this device" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Delete group from this device" }));
  fireEvent.press(view.getByRole("button", { name: "Confirm delete group from this device" }));
  await view.findByText("Group history deleted from this device. Other devices keep their copies.");
  view.unmount();
  const restored = render(<Management current={null} />);
  await restored.findByText(
    "Group history deleted from this device. Other devices keep their copies.",
  );
  fireEvent.press(restored.getByRole("button", { name: "Done" }));
  await waitFor(() => expect(mockActions.actions.size).toBe(0));
  expect(mockManage).toHaveBeenCalledTimes(1);
});

test("a paused identity can explicitly delete local history", async () => {
  mockCanPublish = false;
  mockManage.mockResolvedValue(Bindings.GroupActionResult.DeletedLocalGroup.new());
  const view = render(
    <Management
      current={{
        ...conversation,
        lifecycle: Bindings.GroupLifecycle.Closed,
        canRemoveMembers: false,
        canClose: false,
        canDeleteLocal: true,
      }}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Delete group from this device" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Delete group from this device" }));
  fireEvent.press(view.getByRole("button", { name: "Confirm delete group from this device" }));
  await view.findByText("Group history deleted from this device. Other devices keep their copies.");
  expect(mockManage.mock.calls[0]?.[0].tag).toBe("DeleteLocalGroup");
});

test("a paused identity can dismiss a finished invitation", async () => {
  mockCanPublish = false;
  mockManage.mockResolvedValue(Bindings.GroupActionResult.DismissedInvitation.new());
  const view = render(
    <GroupInvitationManagement
      invitationId={invitationId}
      invitation={{
        ...invitation,
        state: Bindings.GroupInvitationState.Expired,
        canCancel: false,
        canDismiss: true,
      }}
      onSettled={mockRefresh}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Remove invitation from list" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Remove invitation from list" }));
  fireEvent.press(view.getByRole("button", { name: "Confirm remove invitation from list" }));
  await view.findByText("Invitation removed from your list.");
  expect(mockManage.mock.calls[0]?.[0].tag).toBe("DismissInvitation");
});

test("local cleanup admission cannot execute a network mutation while paused", async () => {
  mockCanPublish = false;
  const mutation = renderHook(() => useGroupMutation("DeleteLocalGroup", groupId));
  await waitFor(() => expect(mutation.result.current.ready).toBe(true));
  await act(async () => {
    await mutation.result.current.submit((clientRequestId) =>
      Bindings.GroupActionInput.CloseGroup.new({
        input: { groupId, clientRequestId, expectedControlHead: controlHead },
      }),
    );
  });
  expect(mockActions.prepareGroupAction).not.toHaveBeenCalled();
  expect(mockManage).not.toHaveBeenCalled();
  expect(mockActions.actions.size).toBe(0);
});

test("an active member can delete locally after reviewing that remote membership is unchanged", async () => {
  mockManage.mockResolvedValue(Bindings.GroupActionResult.DeletedLocalGroup.new());
  const view = render(
    <Management
      current={{
        ...conversation,
        canRemoveMembers: false,
        canClose: false,
        canLeave: true,
        canDeleteLocal: true,
      }}
    />,
  );
  await waitFor(() =>
    expect(
      view.getByRole("button", { name: "Delete group from this device" }).props.accessibilityState
        .disabled,
    ).toBe(false),
  );
  fireEvent.press(view.getByRole("button", { name: "Delete group from this device" }));
  expect(
    view.getByText(
      /Deleting locally leaves remote membership unchanged; leave the group first to request removal/,
    ),
  ).toBeTruthy();
  expect(mockManage).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Confirm delete group from this device" }));
  await view.findByText("Group history deleted from this device. Other devices keep their copies.");
  expect(mockManage).toHaveBeenCalledTimes(1);
  expect(mockManage.mock.calls[0]?.[0].tag).toBe("DeleteLocalGroup");
});
