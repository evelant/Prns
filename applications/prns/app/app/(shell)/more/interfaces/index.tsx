// route-id: interfaces.index

import { useLocalSearchParams } from "expo-router";

import { ConnectionsScreen } from "@/features/connections/connections-screen";
import { CatalogRouteGuard } from "@/features/placeholder-screen";

export default function InterfacesRoute() {
  const { section } = useLocalSearchParams<{ section?: string | string[] }>();
  const initialSection = section === "ip" ? "ip" : "bluetooth";
  return (
    <CatalogRouteGuard screenId="interfaces.index">
      <ConnectionsScreen key={initialSection} initialSection={initialSection} />
    </CatalogRouteGuard>
  );
}
