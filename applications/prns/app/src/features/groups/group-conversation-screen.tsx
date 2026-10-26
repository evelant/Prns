import type { GroupTextMessage } from "@prns-internal/expo";
import * as Bindings from "@prns-internal/expo";
import { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import { timestampLabel } from "@/features/inbox/format";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { useVisibleIdentityActivity } from "@/native/use-visible-identity-activity";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import {
  ActionRow,
  Badge,
  BodyText,
  Button,
  Card,
  Screen,
  ScreenHeading,
  Subheading,
} from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { useAppPalette } from "@/ui/theme";
import { groupDeliveryLabel, groupDeviceLabel, groupUtf8Length, identifierHex } from "./format";
import { GroupActionFeedback } from "./group-action-feedback";
import { GroupDetails } from "./group-details";
import { GroupManagement } from "./group-management";
import { GroupSyncStatus } from "./group-sync-status";
import { useGroupContacts } from "./use-group-contacts";
import { useGroupData } from "./use-group-data";
import { useGroupIdentityScope, useGroupMutation } from "./use-group-mutation";

export function GroupConversationScreen({ groupId }: { readonly groupId: Uint8Array }) {
  return (
    <GroupConversationJourney
      key={useGroupIdentityScope(identifierHex(groupId))}
      groupId={groupId}
    />
  );
}

function GroupConversationJourney({ groupId }: { readonly groupId: Uint8Array }) {
  const development = useDevelopmentRuntime();
  const data = useGroupData(groupId);
  const directory = useGroupContacts();
  const visibleThrough =
    data.loaded && data.failure === null
      ? data.messages
          .filter((message) => !message.outgoing)
          .reduce<bigint | undefined>(
            (latest, message) =>
              latest === undefined || message.sequence > latest ? message.sequence : latest,
            undefined,
          )
      : undefined;
  useVisibleIdentityActivity(Bindings.IdentityActivityScope.Group.new({ groupId }), visibleThrough);
  const [details, setDetails] = useState<"group" | "delivery" | null>(null);
  const groupKey = identifierHex(groupId);
  const conversation = data.conversations.find(
    (entry) => identifierHex(entry.groupId) === groupKey,
  );
  const nodeRunning =
    development.phase === "ready" &&
    development.snapshot?.runtime === Bindings.DevelopmentNodeRuntime.Running;
  return (
    <Screen>
      <ScreenHeading
        trailing={
          <InfoDisclosure label="About this conversation">
            <BodyText>
              Keep prns open to send and receive. Receipts confirm devices received a message, not
              that someone read it.
            </BodyText>
            <BodyText>
              Maintainer alpha · Use test messages. Saved group data is not yet protected.
            </BodyText>
          </InfoDisclosure>
        }
      >
        {conversation?.title ?? "Group conversation"}
      </ScreenHeading>
      {development.availability.type !== "available" ? (
        <Card>
          <BodyText>Group messaging is available on iOS and Android.</BodyText>
        </Card>
      ) : (
        <>
          {!nodeRunning && development.canPublish !== false ? (
            <NavigationLink href="/setup">Reconnect to synchronize</NavigationLink>
          ) : null}
          {data.failure === null ? null : (
            <Card>
              <BodyText>{data.failure}</BodyText>
            </Card>
          )}
          {conversation?.synchronization.tag === "Unavailable" ? (
            <Card>
              <Badge tone="warning">Needs attention · Group synchronization unavailable</Badge>
              <BodyText muted>Open connection setup to reconnect.</BodyText>
            </Card>
          ) : null}
          {!data.loaded && data.failure === null ? (
            <BodyText muted>Loading saved messages…</BodyText>
          ) : null}
          {conversation === undefined ? (
            <Button disabled={data.pending} tone="secondary" onPress={() => void data.refresh()}>
              Refresh group
            </Button>
          ) : null}
          {data.loaded && conversation === undefined ? (
            <Card>
              <BodyText>This group is not enrolled on this device.</BodyText>
            </Card>
          ) : null}
          {conversation?.lifecycle === Bindings.GroupLifecycle.Leaving ? (
            <BodyText>
              Leaving group · Waiting for the owner to confirm removal. Saved messages remain
              available.
            </BodyText>
          ) : conversation?.lifecycle === Bindings.GroupLifecycle.Removed ? (
            <BodyText>
              You are no longer a member of this group. Saved messages remain available.
            </BodyText>
          ) : conversation?.lifecycle === Bindings.GroupLifecycle.Closed ? (
            <BodyText>The owner closed this group. Saved messages remain available.</BodyText>
          ) : null}
          <GroupComposer
            groupId={groupId}
            messages={data.messages}
            visible={conversation?.canSend === true}
            canSend={
              conversation?.canSend === true &&
              development.canPublish !== false &&
              development.phase === "ready" &&
              development.snapshot?.primaryIdentity?.tag === "Present"
            }
            onSettled={data.refresh}
          />
          {conversation === undefined ? null : (
            <>
              {conversation.canInvite && conversation.memberCount === 1 ? (
                <NavigationLink
                  href={{
                    pathname: "/inbox/group/[groupId]/invite",
                    params: { groupId: groupKey },
                  }}
                >
                  Invite a device
                </NavigationLink>
              ) : null}
              {conversation.canInvite && conversation.memberCount === 1 ? (
                <BodyText muted>Invite someone to start the conversation.</BodyText>
              ) : null}
              <ActionRow>
                <Button
                  tone="secondary"
                  accessibilityState={{ expanded: details === "group" }}
                  onPress={() => setDetails((value) => (value === "group" ? null : "group"))}
                >
                  Group details
                </Button>
                <Button
                  tone="secondary"
                  accessibilityState={{ expanded: details === "delivery" }}
                  onPress={() => setDetails((value) => (value === "delivery" ? null : "delivery"))}
                >
                  Delivery details
                </Button>
              </ActionRow>
              {details === "group" ? (
                <GroupDetails conversation={conversation} contacts={directory.contacts} />
              ) : null}
              {details === "delivery" ? (
                <Card>
                  <BodyText muted>
                    Keep prns open for synchronization. Receipt counts show devices, not people
                    reading.
                  </BodyText>
                  <GroupSyncStatus progress={conversation.progress} />
                  {conversation.synchronization.tag === "Unavailable" ? (
                    <BodyText>{conversation.synchronization.inner.detail}</BodyText>
                  ) : null}
                  <Button
                    disabled={data.pending}
                    tone="secondary"
                    onPress={() => void data.refresh()}
                  >
                    {data.pending ? "Refreshing…" : "Refresh group"}
                  </Button>
                </Card>
              ) : null}
            </>
          )}
          <GroupManagement
            groupId={groupId}
            conversation={conversation}
            contacts={directory.contacts}
            visible={details === "group"}
            onSettled={data.refresh}
          />
          {conversation === undefined ? null : (
            <>
              <Subheading>Messages</Subheading>
              {data.messages.length === 0 ? (
                <BodyText muted>No group messages yet.</BodyText>
              ) : null}
              {data.messages.map((message) => (
                <GroupMessageBubble
                  key={identifierHex(message.messageId)}
                  message={message}
                  author={
                    message.outgoing ? "You" : groupDeviceLabel(message.author, directory.contacts)
                  }
                />
              ))}
              {data.hasMore ? (
                <Button
                  disabled={data.pending}
                  tone="secondary"
                  onPress={() => void data.loadOlder()}
                >
                  Load older group messages
                </Button>
              ) : null}
            </>
          )}
        </>
      )}
      <NavigationLink href="/inbox" direction="back">
        Back to Inbox
      </NavigationLink>
    </Screen>
  );
}

function GroupComposer({
  groupId,
  messages,
  canSend,
  visible,
  onSettled,
}: {
  readonly groupId: Uint8Array;
  readonly messages: readonly GroupTextMessage[];
  readonly canSend: boolean;
  readonly visible: boolean;
  readonly onSettled: () => Promise<void>;
}) {
  const mutation = useGroupMutation("Send", groupId);
  const [text, setText] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const oversized = groupUtf8Length(text) > 2048;
  const { input, result, acknowledge, refresh } = mutation;
  useEffect(() => {
    if (input?.tag === "Send") setText(input.inner.input.text);
  }, [input]);
  useEffect(() => {
    if (result?.tag !== "Sent") return;
    setStatus("Saved locally.");
    void acknowledge().then((cleared) => {
      if (cleared) setText("");
    });
  }, [acknowledge, result]);

  const savedRequest = input === null ? null : identifierHex(input.inner.input.clientRequestId);
  const hasSavedMessage =
    savedRequest !== null &&
    messages.some(
      (message) =>
        message.outgoing &&
        message.clientRequestId !== undefined &&
        identifierHex(message.clientRequestId) === savedRequest,
    );
  const needsReconciliation = hasSavedMessage && mutation.action?.state.tag === "Pending";
  useEffect(() => {
    // Reading a result never allocates a second ID or submits a second draft.
    if (needsReconciliation) void refresh();
  }, [needsReconciliation, refresh]);

  const send = async () => {
    if (!canSend || text.trim().length === 0 || (oversized && !mutation.retrying)) return;
    setStatus(null);
    await mutation.submit((clientRequestId) =>
      Bindings.GroupActionInput.Send.new({ input: { groupId, text, clientRequestId } }),
    );
    await onSettled();
  };

  if (!visible && !mutation.retrying) return null;
  return (
    <Card>
      <TextField
        label="Group message"
        placeholder="Message"
        hideLabel
        compact
        testID="group-compose-input"
        multiline
        value={text}
        onChangeText={setText}
        editable={mutation.ready && !mutation.pending && !mutation.retrying}
      />
      {oversized && !mutation.retrying ? (
        <BodyText>Message too long. Shorten it before sending.</BodyText>
      ) : null}
      <Button
        disabled={
          !mutation.ready ||
          !canSend ||
          mutation.pending ||
          mutation.retryExpired ||
          mutation.result !== null ||
          text.trim().length === 0 ||
          (oversized && !mutation.retrying)
        }
        onPress={() => void send()}
      >
        {mutation.pending
          ? "Saving message…"
          : mutation.retrying
            ? "Retry same message"
            : "Send to group"}
      </Button>
      <GroupActionFeedback failure={mutation.failure} result={mutation.result} />
      {status === null ? null : <BodyText>{status}</BodyText>}
      {!mutation.ready ? (
        <Button tone="secondary" onPress={() => void mutation.refresh()}>
          Reload saved actions
        </Button>
      ) : null}
      {result?.tag === "Sent" && !mutation.pending ? (
        <Button
          tone="secondary"
          onPress={() =>
            void acknowledge().then((cleared) => {
              if (cleared) setText("");
            })
          }
        >
          Continue composing
        </Button>
      ) : null}
      {mutation.retryExpired || mutation.result?.tag === "Rejected" ? (
        <Button
          tone="secondary"
          onPress={() => {
            void mutation.beginNewActionAfterReview().then((cleared) => {
              if (cleared)
                setStatus("You can edit this draft and deliberately send it as a new message.");
            });
          }}
        >
          I&apos;ve reviewed saved messages
        </Button>
      ) : null}
    </Card>
  );
}

function GroupMessageBubble({
  message,
  author,
}: {
  readonly message: GroupTextMessage;
  readonly author: string;
}) {
  const palette = useAppPalette();
  return (
    <View style={[bubble.row, { alignItems: message.outgoing ? "flex-end" : "flex-start" }]}>
      <View
        style={[
          bubble.body,
          { backgroundColor: message.outgoing ? palette.selected : palette.surface },
        ]}
      >
        <Text style={[bubble.author, { color: palette.textMuted }]}>{author}</Text>
        <Text
          selectable
          testID={`group-record-${identifierHex(message.messageId)}`}
          style={[bubble.text, { color: palette.text }]}
        >
          {message.text}
        </Text>
        <Text style={[bubble.meta, { color: palette.textMuted }]}>
          {timestampLabel(message.sentAtMillis)}
        </Text>
        <Text
          testID={`group-receipt-${identifierHex(message.messageId)}`}
          style={[bubble.meta, { color: palette.textMuted }]}
        >
          {groupDeliveryLabel(message)}
        </Text>
      </View>
    </View>
  );
}
const bubble = StyleSheet.create({
  row: { width: "100%" },
  body: { borderRadius: 18, padding: 14, gap: 5, maxWidth: "92%" },
  author: { fontSize: 12, fontWeight: "600", lineHeight: 17 },
  text: { fontSize: 17, lineHeight: 24 },
  meta: { fontSize: 12, lineHeight: 17 },
});
