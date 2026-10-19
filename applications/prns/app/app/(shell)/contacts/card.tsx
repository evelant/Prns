// route-id: contacts.card

import { MyContactCardScreen } from "@/features/contacts/my-contact-card-screen";
import { CatalogRouteGuard } from "@/features/placeholder-screen";

export default function MyContactCardRoute() {
  return (
    <CatalogRouteGuard screenId="contacts.card">
      <MyContactCardScreen />
    </CatalogRouteGuard>
  );
}
