// route-id: contacts.index

import { useLocalSearchParams } from "expo-router";

import { ContactsScreen } from "@/features/contacts/contacts-screen";
import { CatalogRouteGuard } from "@/features/placeholder-screen";

export default function ContactsRoute() {
  const { section } = useLocalSearchParams<{ section?: string | string[] }>();
  const initialSection = section === "profile" || section === "discovered" ? section : "saved";
  return (
    <CatalogRouteGuard screenId="contacts.index">
      <ContactsScreen initialSection={initialSection} />
    </CatalogRouteGuard>
  );
}
