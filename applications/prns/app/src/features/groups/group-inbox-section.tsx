import { type GroupInvitation, GroupInvitationState, GroupLifecycle } from "@prns-internal/expo";

import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Button, Card, Subheading } from "@/ui/primitives";
import { identifierHex, invitationStateLabel } from "./format";
import { GroupAnnounceSettings } from "./group-announce-settings";
import { GroupDisclosure } from "./group-disclosure";
import { GroupSavedActions } from "./group-saved-actions";
import { useGroupData } from "./use-group-data";
import { useGroupInvitations } from "./use-group-invitations";

export function GroupInboxSection() {
  const data = useGroupData(null);
  const invitations = useGroupInvitations();
  const activeIncoming = (invitation: GroupInvitation) =>
    !invitation.outgoing &&
    (invitation.state === GroupInvitationState.Offered ||
      invitation.state === GroupInvitationState.Joining);
  const incoming = invitations.invitations.filter(activeIncoming);
  const history = invitations.invitations.filter((invitation) => !activeIncoming(invitation));
  return (
    <>
      {incoming.length > 0 ? <Subheading>Invitations</Subheading> : null}
      {incoming.map((invitation) => (
        <InvitationCard key={identifierHex(invitation.invitationId)} invitation={invitation} />
      ))}
      <Subheading>Groups</Subheading>
      <GroupSavedActions />
      <NavigationLink href="/inbox/group/new">New group</NavigationLink>
      {data.failure === null ? null : (
        <Card>
          <BodyText>{data.failure}</BodyText>
          <Button tone="secondary" disabled={data.pending} onPress={() => void data.refresh()}>
            Refresh groups
          </Button>
        </Card>
      )}
      {!data.loaded && data.failure === null ? (
        <BodyText muted>Loading saved groups…</BodyText>
      ) : null}
      {data.loaded && data.conversations.length === 0 ? (
        <BodyText muted>No groups yet.</BodyText>
      ) : null}
      {data.conversations.map((conversation) => (
        <Card key={identifierHex(conversation.groupId)}>
          <Badge>
            {conversation.lifecycle === GroupLifecycle.Leaving
              ? "Leaving group"
              : conversation.lifecycle === GroupLifecycle.Removed
                ? "No longer a member"
                : conversation.lifecycle === GroupLifecycle.Closed
                  ? "Closed group"
                  : "Group"}
          </Badge>
          <Subheading>{conversation.title}</Subheading>
          <BodyText muted>{conversation.memberCount} members</BodyText>
          {conversation.synchronization.tag === "Unavailable" ? (
            <Badge tone="warning">Group synchronization unavailable</Badge>
          ) : null}
          <NavigationLink
            accessibilityLabel={`Open group ${conversation.title}`}
            href={{
              pathname: "/inbox/group/[groupId]",
              params: { groupId: identifierHex(conversation.groupId) },
            }}
          >
            Open group conversation
          </NavigationLink>
        </Card>
      ))}
      {invitations.failure === null ? null : <BodyText>{invitations.failure}</BodyText>}
      {history.length > 0 ? (
        <GroupDisclosure label="Invitation history">
          {history.map((invitation) => (
            <InvitationCard key={identifierHex(invitation.invitationId)} invitation={invitation} />
          ))}
        </GroupDisclosure>
      ) : null}
      <GroupDisclosure label="Group connection settings">
        <GroupAnnounceSettings />
        <Button
          tone="secondary"
          disabled={invitations.pending}
          onPress={() => void invitations.refresh()}
        >
          Refresh invitations
        </Button>
      </GroupDisclosure>
      <Subheading>Direct messages</Subheading>
    </>
  );
}

function InvitationCard({ invitation }: { readonly invitation: GroupInvitation }) {
  return (
    <Card key={identifierHex(invitation.invitationId)}>
      <Subheading>{invitation.title}</Subheading>
      <Badge>{invitation.outgoing ? "Your invitation" : "Received invitation"}</Badge>
      <BodyText>{invitationStateLabel(invitation)}</BodyText>
      <NavigationLink
        accessibilityLabel={`View invitation for ${invitation.title}`}
        href={{
          pathname: "/inbox/invitation/[invitationId]",
          params: { invitationId: identifierHex(invitation.invitationId) },
        }}
      >
        View invitation
      </NavigationLink>
    </Card>
  );
}
