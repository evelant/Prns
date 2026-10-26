import type { GroupActionInput } from "@prns-internal/expo";
import { useState } from "react";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { BodyText, Button, Card, Subheading } from "@/ui/primitives";
import { identifierHex } from "./format";
import { GroupActionFeedback } from "./group-action-feedback";
import { useGroupIdentityScope, useGroupMutation } from "./use-group-mutation";

export type ManagementKind = Exclude<
  `${GroupActionInput["tag"]}`,
  "Create" | "Invite" | "Respond" | "Send"
>;

export type ManagementChoice = {
  readonly key: string;
  readonly label: string;
  readonly detail?: string;
  readonly consequence: string;
  readonly makeInput: (clientRequestId: Uint8Array) => GroupActionInput;
};

export function managementActionLabel(kind: ManagementKind): string {
  switch (kind) {
    case "RemoveMember":
      return "member removal";
    case "LeaveGroup":
      return "leaving the group";
    case "CloseGroup":
      return "group closure";
    case "DeleteLocalGroup":
      return "local group deletion";
    case "CancelInvitation":
      return "invitation cancellation";
    case "DismissInvitation":
      return "invitation dismissal";
  }
}

type GroupManagementActionProps = {
  readonly kind: ManagementKind;
  readonly scope: Uint8Array;
  readonly choices: readonly ManagementChoice[];
  readonly onSettled: () => Promise<void>;
};

export function GroupManagementAction(props: GroupManagementActionProps) {
  const development = useDevelopmentRuntime();
  const identityScope = useGroupIdentityScope(`${props.kind}:${identifierHex(props.scope)}`);
  return (
    <GroupManagementJourney
      key={`${identityScope}:${development.snapshot?.generationId ?? "unknown"}`}
      {...props}
    />
  );
}

function GroupManagementJourney({ kind, scope, choices, onSettled }: GroupManagementActionProps) {
  const mutation = useGroupMutation(kind, scope);
  const [confirmation, setConfirmation] = useState<ManagementChoice | null>(null);
  const submit = async () => {
    const makeInput = confirmation?.makeInput;
    if (mutation.input === null && makeInput === undefined) return;
    await mutation.submit((id) => {
      if (makeInput !== undefined) return makeInput(id);
      throw new Error("A saved management action must retain its input");
    });
    await onSettled();
  };
  const clear = async () => {
    const cleared = await mutation.acknowledge();
    if (cleared) setConfirmation(null);
  };
  if (
    choices.length === 0 &&
    mutation.action === null &&
    confirmation === null &&
    mutation.failure === null
  )
    return null;
  const savedInput = mutation.input;
  return (
    <Card>
      {mutation.action === null && confirmation === null
        ? choices.map((choice) => (
            <Button
              key={choice.key}
              tone="secondary"
              disabled={!mutation.ready || mutation.pending || !mutation.available}
              onPress={() => setConfirmation(choice)}
            >
              {choice.label}
            </Button>
          ))
        : null}
      {confirmation !== null && mutation.action === null ? (
        <>
          <Subheading>{confirmation.label}?</Subheading>
          {confirmation.detail === undefined ? null : <BodyText>{confirmation.detail}</BodyText>}
          <BodyText>{confirmation.consequence}</BodyText>
          <Button
            tone="destructive"
            disabled={!mutation.ready || mutation.pending || !mutation.available}
            onPress={() => void submit()}
          >
            Confirm {confirmation.label.toLowerCase()}
          </Button>
          <Button
            tone="secondary"
            disabled={mutation.pending}
            onPress={() => setConfirmation(null)}
          >
            Keep unchanged
          </Button>
        </>
      ) : null}
      {mutation.action !== null ? (
        <>
          <Subheading>Saved {managementActionLabel(kind)}</Subheading>
          {savedInput?.tag === "RemoveMember" ? (
            <BodyText>
              Device {identifierHex(savedInput.inner.input.endpointId).slice(0, 8)} · enrollment{" "}
              {savedInput.inner.input.endpointIncarnation.toString()}
            </BodyText>
          ) : null}
          {mutation.result !== null && mutation.result.tag !== "Rejected" ? (
            <BodyText>
              {kind === "LeaveGroup"
                ? "Leave request saved. Sending has stopped; the owner must receive your request to finish removal."
                : kind === "DeleteLocalGroup"
                  ? "Group history deleted from this device. Other devices keep their copies."
                  : kind === "DismissInvitation"
                    ? "Invitation removed from your list."
                    : "Saved locally. Other devices will learn about this change when they synchronize."}
            </BodyText>
          ) : null}
          {mutation.result === null && !mutation.retryExpired ? (
            <Button
              disabled={!mutation.ready || mutation.pending || !mutation.available}
              onPress={() => void submit()}
            >
              {mutation.pending ? "Saving change…" : `Retry same ${managementActionLabel(kind)}`}
            </Button>
          ) : null}
          {mutation.result !== null ? (
            <Button tone="secondary" disabled={mutation.pending} onPress={() => void clear()}>
              {mutation.result.tag === "Rejected" ? "I've reviewed the result" : "Done"}
            </Button>
          ) : null}
          {mutation.retryExpired ? (
            <Button
              tone="secondary"
              disabled={mutation.pending}
              onPress={() => {
                void mutation.beginNewActionAfterReview().then((cleared) => {
                  if (cleared) setConfirmation(null);
                });
              }}
            >
              I&apos;ve reviewed membership and invitations
            </Button>
          ) : null}
        </>
      ) : null}
      <GroupActionFeedback failure={mutation.failure} result={mutation.result} />
      {!mutation.ready ? (
        <Button tone="secondary" onPress={() => void mutation.refresh()}>
          Reload saved actions
        </Button>
      ) : null}
    </Card>
  );
}
