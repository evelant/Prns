import type { PropsWithChildren } from "react";
import { StyleSheet, View } from "react-native";

/** Retain drafts and pending actions while hiding inactive sections from navigation. */
export function SectionPanel({
  active,
  children,
}: PropsWithChildren<{ readonly active: boolean }>) {
  return (
    <View
      accessibilityElementsHidden={!active}
      importantForAccessibility={active ? "auto" : "no-hide-descendants"}
      style={active ? styles.panel : styles.hidden}
    >
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { gap: 12, minWidth: 0 },
  hidden: { display: "none" },
});
