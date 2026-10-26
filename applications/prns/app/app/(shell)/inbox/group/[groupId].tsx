// route-id: inbox.group

import { useLocalSearchParams } from "expo-router";

import { parseGroupId } from "@/features/groups/format";
import { GroupConversationScreen } from "@/features/groups/group-conversation-screen";
import { NotFoundScreen } from "@/features/placeholder-screen";
import { type RawRouteParams, routeParamsAreValid, screenById } from "@/navigation/catalog";

export default function GroupConversationRoute() {
  const entry = screenById("inbox.group");
  const params: RawRouteParams = useLocalSearchParams();
  const groupId = typeof params.groupId === "string" ? parseGroupId(params.groupId) : null;
  if (!routeParamsAreValid(entry, params) || groupId === null) {
    return <NotFoundScreen backPath={entry.backPath} />;
  }
  return <GroupConversationScreen groupId={groupId} />;
}
