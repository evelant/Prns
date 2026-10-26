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
      <NavigationLink
        href={{ pathname: "/more/interfaces", params: { section: "ip" } }}
        direction="back"
      >
        Back to Interfaces
      </NavigationLink>
    </Screen>
  );
}
