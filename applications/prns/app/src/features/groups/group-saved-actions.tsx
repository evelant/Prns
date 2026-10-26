import type { GroupAction } from "@prns-internal/expo";
import { useEffect, useMemo, useState } from "react";
import { AppState } from "react-native";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Card } from "@/ui/primitives";
import { identifierHex } from "./format";
import { managementActionLabel } from "./group-management-action";

export function GroupSavedActions() {
  const development = useDevelopmentRuntime();
  const { listGroupActions } = development;
  const available = development.availability.type === "available";
  const generation = development.snapshot?.generationId;
  const identity = development.snapshot?.primaryIdentity;
  const identityKey =
    identity?.tag === "Present" ? identifierHex(identity.inner.identityHash) : null;
  const localOwner = development.ownerKey;
  const owner = useMemo(
    () => ({ available, generation, identityKey, localOwner }),
    [available, generation, identityKey, localOwner],
  );
  const [saved, setSaved] = useState<{
    owner: object;
    actions: readonly GroupAction[];
    failed: boolean;
  }>({ owner, actions: [], failed: false });
  useEffect(() => {
    if (!available) return;
    let cancelled = false;
    let busy = false;
    const refresh = async () => {
      if (busy || cancelled) return;
      busy = true;
      try {
        const response = await listGroupActions();
        if (cancelled) return;
        if (response.type === "outcome" && response.outcome.tag === "Ready")
          setSaved({
            owner,
            actions: response.outcome.inner.actions.filter(
              (action) => identifierHex(action.ownerIdentity) === identityKey,
            ),
            failed: false,
          });
        else setSaved({ owner, actions: [], failed: true });
      } catch {
        if (!cancelled) setSaved({ owner, actions: [], failed: true });
      } finally {
        busy = false;
      }
    };
    void refresh();
    const interval = setInterval(() => {
      if (AppState.currentState === "active") void refresh();
    }, 3_000);
    const listener = AppState.addEventListener("change", (state) => {
      if (state === "active") void refresh();
    });
    return () => {
      cancelled = true;
      clearInterval(interval);
      listener.remove();
    };
  }, [available, identityKey, listGroupActions, owner]);
  if (saved.owner !== owner) return null;
  return (
    <>
      {saved.failed ? (
        <BodyText>
          Saved group actions could not be checked. Open the related group or invitation to retry
          safely.
        </BodyText>
      ) : null}
      {saved.actions.map((action) => {
        const input = action.input;
        const href =
          input.tag === "Create"
            ? ("/inbox/group/new" as const)
            : input.tag === "Respond" ||
                input.tag === "CancelInvitation" ||
                input.tag === "DismissInvitation"
              ? {
                  pathname: "/inbox/invitation/[invitationId]" as const,
                  params: { invitationId: identifierHex(input.inner.input.invitationId) },
                }
              : {
                  pathname:
                    input.tag === "Invite"
                      ? ("/inbox/group/[groupId]/invite" as const)
                      : ("/inbox/group/[groupId]" as const),
                  params: { groupId: identifierHex(input.inner.input.groupId) },
                };
        return (
          <Card key={identifierHex(input.inner.input.clientRequestId)}>
            <Badge tone={action.state.tag === "Completed" ? "neutral" : "warning"}>
              {action.state.tag === "Completed" ? "Saved locally" : "Needs attention"}
            </Badge>
            <BodyText>
              {action.state.tag === "Completed"
                ? "A saved group action has a result to review."
                : "A saved group action still needs confirmation. Resume it without creating a duplicate."}
            </BodyText>
            <NavigationLink href={href}>
              Review saved{" "}
              {input.tag === "Create"
                ? "group creation"
                : input.tag === "Invite"
                  ? "invitation"
                  : input.tag === "Respond"
                    ? "invitation decision"
                    : input.tag === "Send"
                      ? "message"
                      : managementActionLabel(input.tag)}
            </NavigationLink>
          </Card>
        );
      })}
    </>
  );
}
