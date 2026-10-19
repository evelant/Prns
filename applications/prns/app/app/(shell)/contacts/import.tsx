// route-id: contacts.import

import { useLocalSearchParams } from "expo-router";
import { ContactCardImportScreen } from "@/features/contacts/contact-card-import";
import { CatalogRouteGuard } from "@/features/placeholder-screen";

export default function ImportContactCardRoute() {
  const params = useLocalSearchParams();
  return (
    <CatalogRouteGuard screenId="contacts.import">
      <ContactCardImportScreen
        {...(typeof params.card === "string" ? { initialURI: params.card } : {})}
        invalid={params.invalid === "1"}
      />
    </CatalogRouteGuard>
  );
}
