import { type GroupActionResult, GroupRejectionReason } from "@prns-internal/expo";

import { BodyText } from "@/ui/primitives";
import { GroupDisclosure } from "./group-disclosure";

export function groupRejectionMessage(reason: GroupRejectionReason): string {
  switch (reason) {
    case GroupRejectionReason.InvalidInput:
      return "This action could not be accepted. Review the saved result, then check the name, message or device identity before trying again.";
    case GroupRejectionReason.NoRecipients:
      return "This group needs another enrolled device. Invite a device and wait for it to finish joining before sending.";
    case GroupRejectionReason.RequestConflict:
      return "This request conflicts with a saved action. Review the group's messages and invitations before starting another action.";
    case GroupRejectionReason.InvitationExpired:
      return "This invitation has expired. Ask the group owner for a new invitation.";
    case GroupRejectionReason.NotFound:
      return "This group or invitation is no longer available on this device. Refresh the list and ask the owner for a new invitation if needed.";
    case GroupRejectionReason.MembershipConflict:
      return "The current membership or invitation state does not allow this action. Refresh and review the group's members and invitations before trying again.";
    case GroupRejectionReason.NotOwner:
      return "Only the group owner can perform this action.";
    case GroupRejectionReason.NotJoined:
      return "This device has not finished joining. Keep prns open and reconnect with the owner to confirm membership.";
    case GroupRejectionReason.AuthenticationFailed:
      return "This invitation or membership could not be verified. Confirm the owner's device identity through a trusted channel and ask for a new invitation.";
    case GroupRejectionReason.AlreadyJoined:
      return "This device has already been enrolled. Remove the member from Group details instead of cancelling its invitation.";
    case GroupRejectionReason.GroupClosed:
      return "The owner has closed this group. Saved messages remain available, but the group cannot accept new activity.";
    case GroupRejectionReason.OwnerMustClose:
      return "As the owner, close the group for everyone before deleting it from this device.";
    case GroupRejectionReason.MemberNotFound:
      return "This device is no longer in the saved membership. Refresh Group details before starting another action.";
    case GroupRejectionReason.StaleMembership:
      return "Membership changed after you reviewed it. Refresh Group details and review the current members before confirming again.";
    case GroupRejectionReason.UnsupportedVersion:
      return "This group uses an unsupported version. Check the devices' compatibility before trying again.";
    default:
      return "This action was not accepted. Review the group and its invitations before trying again. More information is available in Error details.";
  }
}

export function GroupActionFeedback({
  failure,
  result,
}: {
  readonly failure: string | null;
  readonly result: GroupActionResult | null;
}) {
  return (
    <>
      {failure === null ? null : <BodyText>{failure}</BodyText>}
      {result?.tag === "Rejected" ? (
        <GroupDisclosure label="Error details">
          <BodyText>{result.inner.detail}</BodyText>
        </GroupDisclosure>
      ) : null}
    </>
  );
}
