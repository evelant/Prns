import { createContext, type PropsWithChildren, useState } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { radius, space, useAppPalette } from "./theme";

export const CloseInfoDisclosure = createContext<(() => void) | null>(null);

/** Optional explanation. Important action results stay on the calling screen. */
export function InfoDisclosure({ label, children }: PropsWithChildren<{ readonly label: string }>) {
  const [open, setOpen] = useState(false);
  const palette = useAppPalette();
  return (
    <>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={label}
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen(true)}
        style={styles.trigger}
      >
        <View style={[styles.icon, { borderColor: palette.textMuted }]}>
          <Text accessible={false} style={[styles.iconText, { color: palette.textMuted }]}>
            i
          </Text>
        </View>
      </Pressable>
      <Modal visible={open} transparent animationType="fade" onRequestClose={() => setOpen(false)}>
        <View style={styles.overlay}>
          <Pressable
            accessible={false}
            onPress={() => setOpen(false)}
            style={StyleSheet.absoluteFill}
          />
          <SafeAreaView
            edges={["bottom", "left", "right"]}
            style={[styles.sheet, { backgroundColor: palette.surface }]}
            accessibilityViewIsModal
          >
            <View style={styles.header}>
              <Text accessibilityRole="header" style={[styles.title, { color: palette.text }]}>
                {label}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Close ${label}`}
                onPress={() => setOpen(false)}
                style={styles.trigger}
              >
                <Text accessible={false} style={[styles.close, { color: palette.textMuted }]}>
                  ×
                </Text>
              </Pressable>
            </View>
            <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
              <CloseInfoDisclosure.Provider value={() => setOpen(false)}>
                {children}
              </CloseInfoDisclosure.Provider>
            </ScrollView>
          </SafeAreaView>
        </View>
      </Modal>
    </>
  );
}

const styles = StyleSheet.create({
  trigger: { minWidth: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  icon: {
    width: 20,
    height: 20,
    borderRadius: 10,
    borderWidth: 1.5,
    alignItems: "center",
    justifyContent: "center",
  },
  iconText: { fontSize: 14, fontWeight: "600", lineHeight: 17 },
  overlay: { flex: 1, justifyContent: "flex-end", backgroundColor: "rgba(0,0,0,0.35)" },
  sheet: { maxHeight: "75%", borderTopLeftRadius: radius.md, borderTopRightRadius: radius.md },
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: space.lg,
    paddingRight: space.sm,
    paddingTop: space.sm,
  },
  title: { flex: 1, fontSize: 20, lineHeight: 28, fontWeight: "600" },
  close: { fontSize: 28, lineHeight: 32 },
  content: { paddingHorizontal: space.lg, paddingBottom: space.lg, gap: space.md },
});
