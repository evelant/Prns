// route-id: inbox.invitation

import { useLocalSearchParams } from "expo-router";

import { parseInvitationId } from "@/features/groups/format";
import { GroupInvitationScreen } from "@/features/groups/group-invitation-screen";
import { NotFoundScreen } from "@/features/placeholder-screen";
import { type RawRouteParams, routeParamsAreValid, screenById } from "@/navigation/catalog";

export default function GroupInvitationRoute() {
  const entry = screenById("inbox.invitation");
  const params: RawRouteParams = useLocalSearchParams();
  const invitationId =
    typeof params.invitationId === "string" ? parseInvitationId(params.invitationId) : null;
  if (!routeParamsAreValid(entry, params) || invitationId === null)
    return <NotFoundScreen backPath={entry.backPath} />;
  return <GroupInvitationScreen invitationId={invitationId} />;
}
