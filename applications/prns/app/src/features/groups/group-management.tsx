import * as Bindings from "@prns-internal/expo";

import { groupDeviceLabel, identifierHex } from "./format";
import { GroupManagementAction } from "./group-management-action";

export function GroupManagement({
  groupId,
  conversation,
  contacts,
  visible,
  onSettled,
}: {
  readonly groupId: Uint8Array;
  readonly conversation: Bindings.GroupConversation | undefined;
  readonly contacts: readonly Bindings.Contact[];
  readonly visible: boolean;
  readonly onSettled: () => Promise<void>;
}) {
  return (
    <>
      <GroupManagementAction
        kind="RemoveMember"
        scope={groupId}
        onSettled={onSettled}
        choices={
          visible && conversation?.canRemoveMembers
            ? conversation.members
                .filter((member) => !member.isOwner && !member.isLocal)
                .map((member) => ({
                  key: `${identifierHex(member.endpointId)}:${member.endpointIncarnation}`,
                  label: `Remove ${groupDeviceLabel(member.identity, contacts)}`,
                  detail: `Device ${identifierHex(member.endpointId).slice(0, 8)} · enrollment ${member.endpointIncarnation}`,
                  consequence:
                    "This device will lose access to future group messages after the membership change. Messages and keys it already received cannot be erased. Unfinished deliveries may be interrupted.",
                  makeInput: (clientRequestId: Uint8Array) =>
                    Bindings.GroupActionInput.RemoveMember.new({
                      input: {
                        groupId,
                        clientRequestId,
                        endpointId: member.endpointId,
                        endpointIncarnation: member.endpointIncarnation,
                        expectedControlHead: conversation.controlHead,
                      },
                    }),
                }))
            : []
        }
      />
      <GroupManagementAction
        kind="LeaveGroup"
        scope={groupId}
        onSettled={onSettled}
        choices={
          visible && conversation?.canLeave
            ? [
                {
                  key: "leave",
                  label: "Leave group",
                  consequence:
                    "Sending will stop on this device. The owner must receive your leave request to finish removing your membership. Your saved history will remain available.",
                  makeInput: (clientRequestId) =>
                    Bindings.GroupActionInput.LeaveGroup.new({
                      input: { groupId, clientRequestId },
                    }),
                },
              ]
            : []
        }
      />
      <GroupManagementAction
        kind="CloseGroup"
        scope={groupId}
        onSettled={onSettled}
        choices={
          visible && conversation?.canClose
            ? [
                {
                  key: "close",
                  label: "Close group for everyone",
                  consequence:
                    "This permanently closes the group and cancels its invitations. Devices stop using the group when they receive the closure. Everyone keeps their saved messages. Unfinished deliveries may be interrupted.",
                  makeInput: (clientRequestId) =>
                    Bindings.GroupActionInput.CloseGroup.new({
                      input: {
                        groupId,
                        clientRequestId,
                        expectedControlHead: conversation.controlHead,
                      },
                    }),
                },
              ]
            : []
        }
      />
      <GroupManagementAction
        kind="DeleteLocalGroup"
        scope={groupId}
        onSettled={onSettled}
        choices={
          visible && conversation?.canDeleteLocal
            ? [
                {
                  key: "delete",
                  label: "Delete group from this device",
                  consequence:
                    "Your saved group history will be deleted from this device. Other devices keep their copies." +
                    (conversation.lifecycle === Bindings.GroupLifecycle.Active
                      ? " Deleting locally leaves remote membership unchanged; leave the group first to request removal."
                      : "") +
                    " Pending membership notifications remain saved for retry.",
                  makeInput: (clientRequestId) =>
                    Bindings.GroupActionInput.DeleteLocalGroup.new({
                      input: { groupId, clientRequestId },
                    }),
                },
              ]
            : []
        }
      />
    </>
  );
}
