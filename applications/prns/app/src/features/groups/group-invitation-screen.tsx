import * as Bindings from "@prns-internal/expo";

import { timestampLabel } from "@/features/inbox/format";
import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Button, Card, KeyValue, Screen, ScreenHeading } from "@/ui/primitives";
import { groupDeviceLabel, identifierHex, invitationStateLabel } from "./format";
import { GroupActionFeedback } from "./group-action-feedback";
import { GroupDisclosure } from "./group-disclosure";
import { GroupInvitationManagement } from "./group-invitation-management";
import { useGroupContacts } from "./use-group-contacts";
import { useGroupInvitations } from "./use-group-invitations";
import { useGroupIdentityScope, useGroupMutation } from "./use-group-mutation";

export function GroupInvitationScreen({ invitationId }: { readonly invitationId: Uint8Array }) {
  return (
    <GroupInvitationJourney
      key={useGroupIdentityScope(identifierHex(invitationId))}
      invitationId={invitationId}
    />
  );
}

function GroupInvitationJourney({ invitationId }: { readonly invitationId: Uint8Array }) {
  const data = useGroupInvitations();
  const directory = useGroupContacts();
  const mutation = useGroupMutation("Respond", invitationId);
  const invitation = data.invitations.find(
    (entry) => identifierHex(entry.invitationId) === identifierHex(invitationId),
  );
  const saved = mutation.result?.tag === "Responded";
  const contact =
    invitation === undefined
      ? undefined
      : directory.contacts.find(
          (entry) =>
            entry.identity !== undefined &&
            identifierHex(entry.identity) ===
              identifierHex(
                invitation.outgoing ? invitation.recipientIdentity : invitation.inviterIdentity,
              ),
        );
  const respond = async (accept: boolean) => {
    await mutation.submit((clientRequestId) =>
      Bindings.GroupActionInput.Respond.new({ input: { invitationId, accept, clientRequestId } }),
    );
    await data.refresh();
  };
  return (
    <Screen>
      <ScreenHeading>{invitation?.title ?? "Group invitation"}</ScreenHeading>
      {data.failure === null ? null : <BodyText>{data.failure}</BodyText>}
      {invitation === undefined ? (
        <BodyText>
          {data.loaded ? "This invitation is not saved on this device." : "Loading invitation…"}
        </BodyText>
      ) : (
        <>
          <Badge>{invitationStateLabel(invitation)}</Badge>
          <BodyText>
            {invitation.outgoing ? "Invited: " : "From: "}
            {groupDeviceLabel(
              invitation.outgoing ? invitation.recipientIdentity : invitation.inviterIdentity,
              directory.contacts,
            )}
          </BodyText>
          {!invitation.outgoing && invitation.state === Bindings.GroupInvitationState.Offered ? (
            <BodyText>
              Accept only if you recognize and trust the owner. New members receive future messages
              only.
            </BodyText>
          ) : null}
          {invitation.waitingForDelivery &&
          (invitation.state === Bindings.GroupInvitationState.Offered ||
            invitation.state === Bindings.GroupInvitationState.Joining) ? (
            <BodyText>
              {invitation.outgoing
                ? "Joining will continue automatically when this device’s pending deliveries finish or expire."
                : "The owner is finishing pending deliveries. Your acceptance is saved; joining will continue automatically."}
            </BodyText>
          ) : invitation.state === Bindings.GroupInvitationState.Joining ? (
            <BodyText>
              {invitation.outgoing
                ? "Waiting for the invited device to finish joining. Keep prns open and both devices connected."
                : "Your acceptance is saved. Keep prns open and reconnect with the owner to finish joining."}
            </BodyText>
          ) : null}
          {invitation.state === Bindings.GroupInvitationState.Expired ? (
            <BodyText>
              {invitation.outgoing
                ? "Send a new invitation from the group if this device should still join."
                : "Ask the group owner for a new invitation."}
            </BodyText>
          ) : null}
          <GroupDisclosure label="Invitation identity details">
            <Card>
              <KeyValue label="Owner identity" value={identifierHex(invitation.inviterIdentity)} />
              <KeyValue
                label="Invited device"
                value={identifierHex(invitation.recipientIdentity)}
              />
              <KeyValue label="Expires" value={timestampLabel(invitation.expiresAtMillis)} />
              {contact === undefined ? (
                <BodyText>No matching saved contact.</BodyText>
              ) : (
                <BodyText>
                  Matches saved contact:{" "}
                  {contact.alias ?? contact.announcedName ?? "Unnamed contact"}
                </BodyText>
              )}
              <BodyText muted>
                Contact names do not verify a person. Compare the identity with someone you trust.
              </BodyText>
              {!invitation.outgoing ? (
                <BodyText>
                  The invitation is authenticated for this device. Accept only if you recognize and
                  trust its owner.
                </BodyText>
              ) : null}
            </Card>
          </GroupDisclosure>
          {invitation.state === Bindings.GroupInvitationState.Joined ? (
            <NavigationLink
              onPress={() => {
                if (saved) void mutation.acknowledge();
              }}
              href={{
                pathname: "/inbox/group/[groupId]",
                params: { groupId: identifierHex(invitation.groupId) },
              }}
            >
              Open group conversation
            </NavigationLink>
          ) : null}
          {saved && invitation.state === Bindings.GroupInvitationState.Offered ? (
            <BodyText>
              {mutation.input?.tag === "Respond" && mutation.input.inner.input.accept
                ? "Join request saved. Waiting for membership confirmation."
                : "Decline saved."}
            </BodyText>
          ) : null}
        </>
      )}
      {!saved &&
      (mutation.retrying ||
        (invitation !== undefined &&
          !invitation.outgoing &&
          invitation.state === Bindings.GroupInvitationState.Offered)) ? (
        <Card>
          <BodyText>
            {mutation.input?.tag === "Respond"
              ? `Saved decision: ${mutation.input.inner.input.accept ? "accept" : "decline"} invitation. Retrying keeps that decision unchanged.`
              : "Ask members to finish sending before joining. The owner waits for its own pending deliveries."}
          </BodyText>
          {mutation.retrying ? (
            <Button
              disabled={
                !mutation.ready ||
                mutation.pending ||
                mutation.retryExpired ||
                mutation.result !== null ||
                !mutation.available
              }
              onPress={() => void respond(true)}
            >
              {mutation.pending ? "Saving decision…" : "Retry same decision"}
            </Button>
          ) : (
            <>
              <Button
                disabled={!mutation.ready || mutation.pending || !mutation.available}
                onPress={() => void respond(true)}
              >
                Accept invitation
              </Button>
              <Button
                tone="secondary"
                disabled={!mutation.ready || mutation.pending || !mutation.available}
                onPress={() => void respond(false)}
              >
                Decline invitation
              </Button>
            </>
          )}
        </Card>
      ) : null}
      <GroupActionFeedback failure={mutation.failure} result={mutation.result} />
      <GroupInvitationManagement
        invitationId={invitationId}
        invitation={invitation}
        onSettled={data.refresh}
      />
      {!mutation.ready ? (
        <Button tone="secondary" onPress={() => void mutation.refresh()}>
          Reload saved actions
        </Button>
      ) : null}
      {mutation.retryExpired || mutation.result?.tag === "Rejected" ? (
        <Button tone="secondary" onPress={() => void mutation.beginNewActionAfterReview()}>
          I&apos;ve reviewed membership and invitations
        </Button>
      ) : null}
      {saved && invitation === undefined ? (
        <BodyText>
          {mutation.input?.tag === "Respond" && mutation.input.inner.input.accept
            ? "Your acceptance was saved. Check groups for the current membership."
            : "Decline saved."}
        </BodyText>
      ) : null}
      <Button tone="secondary" disabled={data.pending} onPress={() => void data.refresh()}>
        Refresh invitation
      </Button>
      <NavigationLink
        href="/inbox"
        direction="back"
        onPress={() => {
          if (saved) void mutation.acknowledge();
        }}
      >
        Back to Inbox
      </NavigationLink>
    </Screen>
  );
}
