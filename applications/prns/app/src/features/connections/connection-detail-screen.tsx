import { useLocalSearchParams } from "expo-router";

import { NavigationLink } from "@/ui/navigation-link";
import { Screen, ScreenHeading } from "@/ui/primitives";
import { IpConnections } from "./ip-connections";

export function ConnectionDetailScreen({ editing = false }: { readonly editing?: boolean }) {
  const params = useLocalSearchParams<{ interfaceId?: string | string[] }>();
  const profileId = typeof params.interfaceId === "string" ? params.interfaceId : undefined;
  return (
    <Screen>
      <ScreenHeading>{editing ? "Edit TCP peer" : "Connection details"}</ScreenHeading>
      <IpConnections
        {...(profileId === undefined ? {} : { profileId })}
        initiallyEditing={editing}
      />
      <NavigationLink href="/more/interfaces" direction="back">
        Back to Connections
      </NavigationLink>
    </Screen>
  );
}
