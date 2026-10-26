import type { Contact, GroupConversation } from "@prns-internal/expo";

import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Card, KeyValue, Subheading } from "@/ui/primitives";
import { groupDeviceLabel, identifierHex, invitationStateLabel } from "./format";
import { GroupDisclosure } from "./group-disclosure";
import { useGroupInvitations } from "./use-group-invitations";

export function GroupDetails({
  conversation,
  contacts,
}: {
  readonly conversation: GroupConversation;
  readonly contacts: readonly Contact[];
}) {
  const data = useGroupInvitations();
  const groupKey = identifierHex(conversation.groupId);
  const invitations = data.invitations.filter((entry) => identifierHex(entry.groupId) === groupKey);
  return (
    <Card>
      <Subheading>Members</Subheading>
      <BodyText muted>
        {conversation.memberCount} enrolled devices · Saved membership on this device
      </BodyText>
      {conversation.members.map((member) => (
        <Card key={`${identifierHex(member.endpointId)}:${member.endpointIncarnation}`}>
          <BodyText>
            {member.isLocal ? "You" : groupDeviceLabel(member.identity, contacts)}
          </BodyText>
          {member.isOwner ? <Badge>Owner</Badge> : null}
          <GroupDisclosure
            label={`Device identity · ${member.isLocal ? "You" : groupDeviceLabel(member.identity, contacts)}`}
          >
            <KeyValue label="Identity" value={identifierHex(member.identity)} />
            <KeyValue label="Endpoint" value={identifierHex(member.endpointId)} />
          </GroupDisclosure>
        </Card>
      ))}
      <BodyText muted>
        Names come from your contacts. Membership does not mean a device is online.
      </BodyText>
      {conversation.canInvite ? (
        <NavigationLink
          href={{ pathname: "/inbox/group/[groupId]/invite", params: { groupId: groupKey } }}
        >
          Invite a device
        </NavigationLink>
      ) : (
        <BodyText muted>
          {conversation.ownerIdentity === undefined
            ? "Invitations are unavailable for this older group."
            : "Ask the group owner to invite another device."}
        </BodyText>
      )}
      {invitations.length > 0 ? <Subheading>Invitations</Subheading> : null}
      {invitations.map((invitation) => (
        <Card key={identifierHex(invitation.invitationId)}>
          <BodyText>{groupDeviceLabel(invitation.recipientIdentity, contacts)}</BodyText>
          <BodyText>{invitationStateLabel(invitation)}</BodyText>
          <NavigationLink
            href={{
              pathname: "/inbox/invitation/[invitationId]",
              params: { invitationId: identifierHex(invitation.invitationId) },
            }}
          >
            View invitation
          </NavigationLink>
        </Card>
      ))}
      {data.failure === null ? null : <BodyText>{data.failure}</BodyText>}
      <GroupDisclosure label="Membership details">
        <KeyValue label="Group" value={groupKey} />
        <KeyValue
          label="Authenticated membership version"
          value={identifierHex(conversation.controlHead)}
        />
      </GroupDisclosure>
    </Card>
  );
}
