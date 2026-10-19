// route-id: identities.detail
import { useLocalSearchParams } from "expo-router";
import { IdentityDetailScreen } from "@/features/identities/identities-screen";
export default function IdentityRoute() {
  const { identityId } = useLocalSearchParams<{ identityId: string }>();
  return <IdentityDetailScreen identityId={typeof identityId === "string" ? identityId : ""} />;
}
