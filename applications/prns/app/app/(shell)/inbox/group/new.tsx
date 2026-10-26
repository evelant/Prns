// route-id: inbox.group-new

import { useLocalSearchParams } from "expo-router";

import { CreateGroupScreen } from "@/features/groups/create-group-screen";
import { NotFoundScreen } from "@/features/placeholder-screen";
import { type RawRouteParams, routeParamsAreValid, screenById } from "@/navigation/catalog";

export default function CreateGroupRoute() {
  const entry = screenById("inbox.group-new");
  const params: RawRouteParams = useLocalSearchParams();
  return routeParamsAreValid(entry, params) ? (
    <CreateGroupScreen />
  ) : (
    <NotFoundScreen backPath={entry.backPath} />
  );
}
