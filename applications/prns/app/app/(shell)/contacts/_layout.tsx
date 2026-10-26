import { ContactsScreen } from "@/features/contacts/contacts-screen";
import { MessagingJourney } from "@/native/messaging-journey";
import { MasterDetailLayout } from "@/ui/master-detail-layout";

export default function ContactsLayout() {
  return (
    <MasterDetailLayout
      emptyDescription="Choose a contact from the list to view or edit it."
      emptyTitle="Select a contact"
      master={
        <MessagingJourney>
          <ContactsScreen />
        </MessagingJourney>
      }
      rootPath="/contacts"
      sectionLabel="Contacts"
    />
  );
}
