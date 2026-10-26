import * as Bindings from "@prns-internal/expo";
import { useEffect, useState } from "react";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import { BodyText, Button, Card, Screen, ScreenHeading } from "@/ui/primitives";
import { TextField } from "@/ui/text-field";
import { groupUtf8Length, identifierHex } from "./format";
import { GroupActionFeedback } from "./group-action-feedback";
import { useGroupIdentityScope, useGroupMutation } from "./use-group-mutation";

export function CreateGroupScreen() {
  return <CreateGroupJourney key={useGroupIdentityScope("create")} />;
}

function CreateGroupJourney() {
  const mutation = useGroupMutation("Create");
  const [title, setTitle] = useState("");
  useEffect(() => {
    if (mutation.input?.tag === "Create") setTitle(mutation.input.inner.input.title);
  }, [mutation.input]);
  const created = mutation.result;
  const oversized = groupUtf8Length(title.trim()) > 128;
  return (
    <Screen>
      <ScreenHeading
        trailing={
          <InfoDisclosure label="About groups">
            <BodyText>
              Create a group, then invite your contacts. You can create it offline.
            </BodyText>
            <BodyText>
              Maintainer alpha · Use test messages. Saved group data is not yet protected.
            </BodyText>
          </InfoDisclosure>
        }
      >
        New group
      </ScreenHeading>
      {!mutation.available ? (
        <BodyText>Group creation is available on iOS and Android.</BodyText>
      ) : created?.tag === "Created" ? (
        <Card>
          <BodyText>Group created. You are its owner and its only member.</BodyText>
          <NavigationLink
            onPress={() => void mutation.acknowledge()}
            href={{
              pathname: "/inbox/group/[groupId]/invite",
              params: { groupId: identifierHex(created.inner.groupId) },
            }}
          >
            Invite a device
          </NavigationLink>
          <NavigationLink
            onPress={() => void mutation.acknowledge()}
            href={{
              pathname: "/inbox/group/[groupId]",
              params: { groupId: identifierHex(created.inner.groupId) },
            }}
          >
            Open group conversation
          </NavigationLink>
          <Button
            tone="secondary"
            disabled={mutation.pending}
            onPress={() =>
              void mutation.acknowledge().then((cleared) => {
                if (cleared) setTitle("");
              })
            }
          >
            Create another group
          </Button>
        </Card>
      ) : (
        <Card>
          <TextField
            label="Group name"
            value={title}
            onChangeText={setTitle}
            editable={mutation.ready && !mutation.pending && !mutation.retrying}
            maxLength={128}
          />
          {oversized && !mutation.retrying ? (
            <BodyText>
              Shorten the group name. The limit is 128 bytes; some characters use more than one.
            </BodyText>
          ) : null}
          <Button
            disabled={
              !mutation.ready ||
              mutation.pending ||
              mutation.retryExpired ||
              mutation.result !== null ||
              title.trim().length === 0 ||
              (oversized && !mutation.retrying)
            }
            onPress={() =>
              void mutation.submit((clientRequestId) =>
                Bindings.GroupActionInput.Create.new({
                  input: { title: title.trim(), clientRequestId },
                }),
              )
            }
          >
            {mutation.pending
              ? "Creating group…"
              : mutation.retrying
                ? "Retry same creation"
                : "Create group"}
          </Button>
          <GroupActionFeedback failure={mutation.failure} result={mutation.result} />
          {!mutation.ready ? (
            <Button tone="secondary" onPress={() => void mutation.refresh()}>
              Reload saved actions
            </Button>
          ) : null}
          {mutation.retryExpired || mutation.result?.tag === "Rejected" ? (
            <Button tone="secondary" onPress={() => void mutation.beginNewActionAfterReview()}>
              I&apos;ve reviewed saved groups
            </Button>
          ) : null}
        </Card>
      )}
      <NavigationLink
        href="/inbox"
        direction="back"
        onPress={() => {
          if (created?.tag === "Created") void mutation.acknowledge();
        }}
      >
        Back to Inbox
      </NavigationLink>
    </Screen>
  );
}
