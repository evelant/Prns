import type * as Bindings from "@prns-internal/expo";
import { identityReferenceKey, useNamedIdentity } from "@/native/named-identity-context";
import { Badge, BodyText } from "@/ui/primitives";

export function useActivityForIdentity(ownerKey: string | undefined) {
  return useNamedIdentity()?.activity?.find(
    (entry) => identityReferenceKey(entry.identity) === ownerKey,
  )?.activity;
}

export function identityActivityLabel(activity: Bindings.IdentityActivityOutcome | undefined) {
  if (activity?.tag !== "Ready") return null;
  const { summary } = activity.inner;
  const invitations = summary.actionableInvitations;
  const unread = Number(summary.unreadGroups) + Number(summary.unreadDirect);
  const parts = [];
  if (invitations > 0) parts.push(`${invitations} invitation${invitations === 1 ? "" : "s"}`);
  if (unread > 0) parts.push(`${unread} unread`);
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function identityActivityCount(activity: Bindings.IdentityActivityOutcome | undefined) {
  if (activity?.tag !== "Ready") return 0;
  const summary = activity.inner.summary;
  return (
    Number(summary.actionableInvitations) +
    Number(summary.unreadGroups) +
    Number(summary.unreadDirect)
  );
}

export function IdentityActivityLabel({ ownerKey }: { readonly ownerKey: string }) {
  const activity = useActivityForIdentity(ownerKey);
  const label = identityActivityLabel(activity);
  return (
    <>
      {label === null ? null : <Badge>{label}</Badge>}
      {activity?.tag === "Unavailable" ? <BodyText muted>Activity unavailable</BodyText> : null}
      {activity?.tag === "Ready" && activity.inner.summary.partial ? (
        <BodyText muted>Some activity is unavailable</BodyText>
      ) : null}
    </>
  );
}
