import { screenById } from "@/navigation/catalog";
import { NavigationLink } from "@/ui/navigation-link";
import { CardStack, Screen, ScreenHeading, Subheading } from "@/ui/primitives";

const sections = [
  { title: "Messaging", entries: ["identities.index"] },
  { title: "Connectivity", entries: ["interfaces.index", "activity.index"] },
  { title: "App & support", entries: ["settings.index", "help.index", "settings.about"] },
] as const;

export function MoreScreen() {
  return (
    <Screen>
      <ScreenHeading>More</ScreenHeading>
      {sections.map((section) => (
        <CardStack key={section.title}>
          <Subheading>{section.title}</Subheading>
          {section.entries.map((id) => {
            const entry = screenById(id);
            return (
              <NavigationLink key={entry.id} href={entry.path}>
                {entry.label}
              </NavigationLink>
            );
          })}
        </CardStack>
      ))}
    </Screen>
  );
}
