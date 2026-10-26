// route-id: installation.setup
import { CatalogRouteGuard } from "@/features/placeholder-screen";
import { SetupScreen } from "@/features/setup/setup-screen";

export default function SetupRoute() {
  return (
    <CatalogRouteGuard screenId="installation.setup">
      <SetupScreen />
    </CatalogRouteGuard>
  );
}
