import * as Bindings from "@prns-internal/expo";

import { GroupManagementAction } from "./group-management-action";

export function GroupInvitationManagement({
  invitationId,
  invitation,
  onSettled,
}: {
  readonly invitationId: Uint8Array;
  readonly invitation: Bindings.GroupInvitation | undefined;
  readonly onSettled: () => Promise<void>;
}) {
  return (
    <>
      <GroupManagementAction
        kind="CancelInvitation"
        scope={invitationId}
        onSettled={onSettled}
        choices={
          invitation?.canCancel
            ? [
                {
                  key: "cancel",
                  label: "Cancel invitation",
                  consequence:
                    "This invitation will no longer allow this device to join. If enrollment has already committed, remove the member from Group details instead.",
                  makeInput: (clientRequestId) =>
                    Bindings.GroupActionInput.CancelInvitation.new({
                      input: { invitationId, clientRequestId },
                    }),
                },
              ]
            : []
        }
      />
      <GroupManagementAction
        kind="DismissInvitation"
        scope={invitationId}
        onSettled={onSettled}
        choices={
          invitation?.canDismiss
            ? [
                {
                  key: "dismiss",
                  label: "Remove invitation from list",
                  consequence:
                    "This removes the finished invitation from your list. It does not change group membership.",
                  makeInput: (clientRequestId) =>
                    Bindings.GroupActionInput.DismissInvitation.new({
                      input: { invitationId, clientRequestId },
                    }),
                },
              ]
            : []
        }
      />
    </>
  );
}
