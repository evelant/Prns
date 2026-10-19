import { type PropsWithChildren, useState } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import { useAppPalette } from "@/ui/theme";

/** Presentation only: opening details never starts protocol work. */
export function Disclosure({
  label,
  accessibilityLabel = label,
  children,
}: PropsWithChildren<{ readonly label: string; readonly accessibilityLabel?: string }>) {
  const [expanded, setExpanded] = useState(false);
  const palette = useAppPalette();
  return (
    <>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={
          expanded ? `Hide ${accessibilityLabel.toLowerCase()}` : accessibilityLabel
        }
        accessibilityState={{ expanded }}
        onPress={() => setExpanded((value) => !value)}
        style={styles.trigger}
      >
        <Text style={[styles.label, { color: palette.textMuted }]}>{label}</Text>
        <Text accessible={false} style={{ color: palette.textMuted }}>
          {expanded ? "⌃" : "⌄"}
        </Text>
      </Pressable>
      {expanded ? children : null}
    </>
  );
}
const styles = StyleSheet.create({
  trigger: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    minHeight: 44,
    gap: 12,
  },
  label: { flex: 1, fontSize: 14, fontWeight: "500", lineHeight: 20 },
});
