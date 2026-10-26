import { NamedIdentityState } from "@prns-internal/expo";
import { Link, usePathname } from "expo-router";
import { StyleSheet, Text, View } from "react-native";
import { useNamedIdentity } from "@/native/named-identity-context";
import { space, useAppPalette } from "@/ui/theme";

export function IdentityBanner() {
  const scope = useNamedIdentity();
  const path = usePathname();
  const palette = useAppPalette();
  if (scope === null || scope.canPublish || !/^\/(inbox|contacts)(?:\/|$)/.test(path)) return null;
  const identity = scope.identity;
  return (
    <View
      style={[styles.container, { backgroundColor: palette.surface, borderColor: palette.border }]}
    >
      {identity !== null && !scope.canPublish ? (
        <Text testID="identity-paused-banner" style={{ color: palette.textMuted }}>
          {identity.transitioning
            ? identity.state === NamedIdentityState.Paused
              ? "Pausing…"
              : "Starting…"
            : identity.state === NamedIdentityState.Paused
              ? "Paused"
              : "This identity is not ready to send."}
        </Text>
      ) : null}
      <Link href="/more/identities" accessibilityLabel="Manage messaging identities">
        <Text style={{ color: palette.accent }}>
          {identity === null ? "Choose an identity" : "Manage identities"}
        </Text>
      </Link>
    </View>
  );
}
const styles = StyleSheet.create({
  container: {
    borderBottomWidth: 1,
    gap: space.xs,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
  },
});
