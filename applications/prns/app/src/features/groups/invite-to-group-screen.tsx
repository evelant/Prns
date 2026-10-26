import * as Bindings from "@prns-internal/expo";
import { useEffect, useState } from "react";

import { ContactCardImporter } from "@/features/contacts/contact-card-import";
import { parseIdentityHash } from "@/features/contacts/format";
import { NavigationLink } from "@/ui/navigation-link";
import {
  BodyText,
  Button,
  Card,
  KeyValue,
  Screen,
  ScreenHeading,
  Subheading,
} from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { groupDeviceLabel, identifierHex, invitationStateLabel } from "./format";
import { GroupActionFeedback } from "./group-action-feedback";
import { GroupDisclosure } from "./group-disclosure";
import { useGroupContacts } from "./use-group-contacts";
import { useGroupData } from "./use-group-data";
import { useGroupInvitations } from "./use-group-invitations";
import { useGroupIdentityScope, useGroupMutation } from "./use-group-mutation";

export function InviteToGroupScreen({ groupId }: { readonly groupId: Uint8Array }) {
  return (
    <InviteToGroupJourney key={useGroupIdentityScope(identifierHex(groupId))} groupId={groupId} />
  );
}

function InviteToGroupJourney({ groupId }: { readonly groupId: Uint8Array }) {
  const data = useGroupData(null);
  const directory = useGroupContacts();
  const invitations = useGroupInvitations();
  const groupKey = identifierHex(groupId);
  const conversation = data.conversations.find(
    (entry) => identifierHex(entry.groupId) === groupKey,
  );
  const mutation = useGroupMutation("Invite", groupId);
  const [identityText, setIdentityText] = useState("");
  const [search, setSearch] = useState("");
  const [choosing, setChoosing] = useState(true);
  const [importingCard, setImportingCard] = useState(false);
  const [selectedLabel, setSelectedLabel] = useState<string | null>(null);
  useEffect(() => {
    if (mutation.input?.tag === "Invite") {
      setIdentityText(identifierHex(mutation.input.inner.input.recipientIdentity));
      setChoosing(false);
    }
  }, [mutation.input]);
  const recipientIdentity = parseIdentityHash(identityText);
  const recipientKey = recipientIdentity === null ? null : identifierHex(recipientIdentity);
  const locked = !mutation.ready || mutation.pending || mutation.retrying;
  const invitation = mutation.result;
  const member = (key: string) =>
    conversation?.members.find((entry) => identifierHex(entry.identity) === key);
  const pendingInvitation = (key: string) =>
    invitations.invitations.find(
      (entry) =>
        entry.outgoing &&
        identifierHex(entry.groupId) === groupKey &&
        identifierHex(entry.recipientIdentity) === key &&
        (entry.state === Bindings.GroupInvitationState.Offered ||
          entry.state === Bindings.GroupInvitationState.Joining),
    );
  const selectedMember = recipientKey === null ? undefined : member(recipientKey);
  const selectedInvitation = recipientKey === null ? undefined : pendingInvitation(recipientKey);
  const duplicate = selectedMember !== undefined || selectedInvitation !== undefined;
  const seen = new Set<string>();
  const query = search.trim().toLowerCase();
  const contacts = directory.contacts.filter((contact) => {
    const key = contact.identity === undefined ? null : identifierHex(contact.identity);
    if (
      query &&
      ![contact.alias, contact.announcedName, key].some((value) =>
        value?.toLowerCase().includes(query),
      )
    )
      return false;
    if (key !== null) {
      if (seen.has(key)) return false;
      seen.add(key);
    }
    return true;
  });
  return (
    <Screen>
      <ScreenHeading>Invite a device</ScreenHeading>
      <BodyText>{conversation?.title ?? "Group invitation"}</BodyText>
      {invitation?.tag === "Invited" ? (
        <Card>
          <BodyText>
            Invitation saved. The device still needs to accept and finish joining.
          </BodyText>
          <NavigationLink
            onPress={() => void mutation.acknowledge()}
            href={{
              pathname: "/inbox/invitation/[invitationId]",
              params: { invitationId: identifierHex(invitation.inner.invitationId) },
            }}
          >
            View invitation progress
          </NavigationLink>
          <Button
            tone="secondary"
            disabled={mutation.pending}
            onPress={() =>
              void mutation.acknowledge().then((cleared) => {
                if (cleared) {
                  setIdentityText("");
                  setSelectedLabel(null);
                  setChoosing(true);
                  setSearch("");
                }
              })
            }
          >
            Invite another device
          </Button>
        </Card>
      ) : conversation?.canInvite || mutation.retrying ? (
        <>
          <BodyText muted>
            Choose a device you trust. New members receive future messages only.
          </BodyText>
          {recipientIdentity === null ? null : (
            <Card>
              <Subheading>
                {selectedLabel ?? groupDeviceLabel(recipientIdentity, directory.contacts)}
              </Subheading>
              <KeyValue label="Selected device identity" value={identifierHex(recipientIdentity)} />
              {selectedMember === undefined ? null : (
                <BodyText>
                  {selectedMember.isLocal
                    ? "This is your device. It is already a member."
                    : "This device is already a member."}
                </BodyText>
              )}
              {selectedInvitation === undefined ? null : (
                <>
                  <BodyText>{invitationStateLabel(selectedInvitation)}</BodyText>
                  <NavigationLink
                    href={{
                      pathname: "/inbox/invitation/[invitationId]",
                      params: { invitationId: identifierHex(selectedInvitation.invitationId) },
                    }}
                  >
                    View existing invitation
                  </NavigationLink>
                </>
              )}
              <Button
                tone="secondary"
                disabled={locked}
                onPress={() => setChoosing((value) => !value)}
              >
                {choosing ? "Hide device choices" : "Choose another device"}
              </Button>
            </Card>
          )}
          {importingCard && !locked ? (
            <ContactCardImporter
              onCancel={() => setImportingCard(false)}
              onImported={(card, contact) => {
                setIdentityText(identifierHex(card.identity));
                setSelectedLabel(
                  contact.alias?.trim() ||
                    contact.announcedName?.trim() ||
                    card.displayName ||
                    "Scanned contact",
                );
                setChoosing(false);
                setImportingCard(false);
                void directory.refresh();
              }}
            />
          ) : null}
          {choosing && !mutation.retrying && !importingCard ? (
            <>
              <Button disabled={locked} onPress={() => setImportingCard(true)}>
                Scan or paste contact
              </Button>
              <Subheading>Saved contacts</Subheading>
              <TextField
                label="Search contacts"
                value={search}
                onChangeText={setSearch}
                editable={!locked}
                autoCorrect={false}
              />
              {directory.failure === null ? null : <BodyText>{directory.failure}</BodyText>}
              {!directory.loaded && directory.failure === null ? (
                <BodyText muted>Loading saved contacts…</BodyText>
              ) : null}
              {directory.loaded && contacts.length === 0 ? (
                <BodyText muted>
                  {query
                    ? "No matching contacts."
                    : "No saved contacts. Add one in Contacts or enter a device identity."}
                </BodyText>
              ) : null}
              {contacts.map((contact) => {
                const key = contact.identity === undefined ? null : identifierHex(contact.identity);
                const enrolled = key === null ? undefined : member(key);
                const pending = key === null ? undefined : pendingInvitation(key);
                const name =
                  contact.alias?.trim() ||
                  contact.announcedName?.trim() ||
                  (contact.identity === undefined
                    ? "Saved contact"
                    : groupDeviceLabel(contact.identity, directory.contacts));
                return (
                  <Card key={identifierHex(contact.destination)}>
                    <Subheading>{name}</Subheading>
                    {key === null ? (
                      <BodyText muted>
                        Device identity is not known. Update this contact when its device is
                        reachable, or ask for its identity.
                      </BodyText>
                    ) : (
                      <>
                        {enrolled === undefined ? null : (
                          <BodyText muted>
                            {enrolled.isLocal
                              ? "Your device · Already a member"
                              : "Already a member"}
                          </BodyText>
                        )}
                        {pending === undefined || enrolled !== undefined ? null : (
                          <BodyText muted>{invitationStateLabel(pending)}</BodyText>
                        )}
                        <Button
                          tone="secondary"
                          disabled={locked || enrolled !== undefined}
                          onPress={() => {
                            setIdentityText(key);
                            setSelectedLabel(name);
                            setChoosing(false);
                          }}
                        >
                          Choose {name}
                        </Button>
                      </>
                    )}
                  </Card>
                );
              })}
              <Button
                tone="secondary"
                disabled={directory.pending}
                onPress={() => void directory.refresh()}
              >
                Reload contacts
              </Button>
              <NavigationLink href="/contacts">Open Contacts</NavigationLink>
              <GroupDisclosure label="Enter device identity">
                <TextField
                  label="Device identity"
                  value={identityText}
                  onChangeText={(value) => {
                    setIdentityText(value);
                    setSelectedLabel(null);
                  }}
                  editable={!locked}
                  autoCapitalize="none"
                  autoCorrect={false}
                  placeholder="32-character identity fingerprint"
                  maxLength={64}
                />
                <BodyText muted>Use the identity fingerprint, not the messaging address.</BodyText>
                {identityText.length > 0 && recipientIdentity === null ? (
                  <BodyText>Enter a 32-character hexadecimal identity.</BodyText>
                ) : null}
              </GroupDisclosure>
            </>
          ) : null}
          <BodyText>
            Joining waits for this device’s pending deliveries. Ask other members to finish sending
            before anyone joins.
          </BodyText>
          <Button
            disabled={
              importingCard ||
              !mutation.available ||
              !mutation.ready ||
              mutation.result !== null ||
              mutation.pending ||
              mutation.retryExpired ||
              recipientIdentity === null ||
              (duplicate && !mutation.retrying)
            }
            onPress={() => {
              if (!importingCard && recipientIdentity !== null && (!duplicate || mutation.retrying))
                void mutation.submit((clientRequestId) =>
                  Bindings.GroupActionInput.Invite.new({
                    input: { groupId, recipientIdentity, clientRequestId },
                  }),
                );
            }}
          >
            {mutation.pending
              ? "Saving invitation…"
              : mutation.retrying
                ? "Retry same invitation"
                : "Send invitation"}
          </Button>
          <GroupActionFeedback failure={mutation.failure} result={mutation.result} />
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
          <GroupDisclosure label="About invitations">
            <BodyText>
              Keep the owner and invited device connected with prns open to finish joining. Only the
              owner can invite devices in this alpha.
            </BodyText>
            <BodyText>
              Pending deliveries on other devices can still be interrupted by a membership change.
              Names are local labels; compare the selected identity through a trusted channel.
            </BodyText>
          </GroupDisclosure>
        </>
      ) : (
        <Card>
          <BodyText>
            {data.loaded
              ? conversation === undefined
                ? "This group is not saved on this device. Refresh groups or ask its owner for an invitation."
                : conversation.ownerIdentity === undefined
                  ? "Invitations are unavailable for this older group."
                  : "Only the group owner can invite devices. Ask the owner to send an invitation."
              : (data.failure ?? "Loading group…")}
          </BodyText>
          <Button tone="secondary" disabled={data.pending} onPress={() => void data.refresh()}>
            Refresh group
          </Button>
        </Card>
      )}
      <NavigationLink
        href={{ pathname: "/inbox/group/[groupId]", params: { groupId: groupKey } }}
        direction="back"
        onPress={() => {
          if (invitation?.tag === "Invited") void mutation.acknowledge();
        }}
      >
        Back to group
      </NavigationLink>
    </Screen>
  );
}
