// route-id: inbox.group-invite

import { useLocalSearchParams } from "expo-router";

import { parseGroupId } from "@/features/groups/format";
import { InviteToGroupScreen } from "@/features/groups/invite-to-group-screen";
import { NotFoundScreen } from "@/features/placeholder-screen";
import { type RawRouteParams, routeParamsAreValid, screenById } from "@/navigation/catalog";

export default function InviteToGroupRoute() {
  const entry = screenById("inbox.group-invite");
  const params: RawRouteParams = useLocalSearchParams();
  const groupId = typeof params.groupId === "string" ? parseGroupId(params.groupId) : null;
  if (!routeParamsAreValid(entry, params) || groupId === null)
    return <NotFoundScreen backPath={entry.backPath} />;
  return <InviteToGroupScreen groupId={groupId} />;
}
