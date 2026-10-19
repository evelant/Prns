import { NamedIdentityState } from "@prns-internal/expo";
import { useState } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import {
  identityReferenceKey,
  type NamedIdentity,
  useNamedIdentity,
} from "@/native/named-identity-context";
import { BodyText, Button, Subheading } from "@/ui/primitives";
import { radius, space, useAppPalette } from "@/ui/theme";
import { identityActivityCount, identityActivityLabel } from "./identity-activity";

/** Choosing a local owner never changes its activation or the app's default itself. */
export function IdentityPicker({
  selectedKey,
  onSelect,
  label = "Identity",
  disabled = false,
}: {
  readonly selectedKey: string | undefined;
  readonly onSelect: (identity: NamedIdentity) => void;
  readonly label?: string;
  readonly disabled?: boolean;
}) {
  const identities = useNamedIdentity();
  const palette = useAppPalette();
  const [open, setOpen] = useState(false);
  const choices = identities?.catalog?.identities.filter(
    (identity) => identity.state !== NamedIdentityState.Retired,
  );
  const selected = choices?.find(
    (identity) => identityReferenceKey(identity.reference) === selectedKey,
  );
  const activityLabel = (key: string | undefined) =>
    identityActivityLabel(
      identities?.activity?.find((entry) => identityReferenceKey(entry.identity) === key)?.activity,
    );
  const selectedActivity = activityLabel(selectedKey);
  const elsewhere =
    identities?.activity
      ?.filter((entry) => identityReferenceKey(entry.identity) !== selectedKey)
      .reduce((total, entry) => total + identityActivityCount(entry.activity), 0) ?? 0;
  const total =
    identities?.activity?.reduce((sum, entry) => sum + identityActivityCount(entry.activity), 0) ??
    0;
  const incomplete =
    identities?.activityFailure != null ||
    identities?.activity?.some(
      (entry) => entry.activity.tag === "Unavailable" || entry.activity.inner.summary.partial,
    ) === true;
  return (
    <>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${label}: ${selected?.label ?? "Choose an identity"}${selectedActivity ? `, ${selectedActivity}` : ""}${elsewhere > 0 ? `, ${elsewhere} updates in other identities` : ""}${incomplete ? ", activity incomplete" : ""}`}
        accessibilityState={{ disabled, expanded: open }}
        disabled={disabled}
        onPress={() => setOpen(true)}
        style={[styles.trigger, { backgroundColor: palette.surfaceRaised }]}
      >
        <Text style={[styles.triggerText, { color: palette.text }]}>
          {selected?.label ?? "Choose an identity"}
        </Text>
        {total === 0 ? (
          incomplete ? (
            <Text style={{ color: palette.textMuted }}>…</Text>
          ) : null
        ) : (
          <Text style={{ color: palette.accent }}>
            {total}
            {incomplete ? "+" : ""} new
          </Text>
        )}
        <Text
          accessibilityElementsHidden
          importantForAccessibility="no"
          style={{ color: palette.textMuted }}
        >
          ⌄
        </Text>
      </Pressable>
      <Modal transparent visible={open} animationType="fade" onRequestClose={() => setOpen(false)}>
        <View style={styles.backdrop}>
          <View
            accessibilityViewIsModal
            style={[
              styles.sheet,
              { backgroundColor: palette.surface, borderColor: palette.border },
            ]}
          >
            <Subheading>Choose an identity</Subheading>
            {identities?.activityFailure == null ? null : (
              <BodyText muted>Activity unavailable</BodyText>
            )}
            <ScrollView contentContainerStyle={styles.options} keyboardShouldPersistTaps="handled">
              {choices === undefined ? <BodyText>Loading identities…</BodyText> : null}
              {choices?.map((identity) => {
                const key = identityReferenceKey(identity.reference);
                const chosen = key === selectedKey;
                const activity = identities?.activity?.find(
                  (entry) => identityReferenceKey(entry.identity) === key,
                )?.activity;
                const state = identity.transitioning
                  ? identity.state === NamedIdentityState.Paused
                    ? "Pausing…"
                    : "Starting…"
                  : identity.state === NamedIdentityState.Paused
                    ? "Paused"
                    : identity.ready
                      ? "Active"
                      : "Starting";
                return (
                  <Pressable
                    key={key}
                    accessibilityRole="radio"
                    accessibilityLabel={`${identity.label}, ${state}`}
                    accessibilityState={{ checked: chosen }}
                    onPress={() => {
                      setOpen(false);
                      onSelect(identity);
                    }}
                    style={[
                      styles.option,
                      { backgroundColor: chosen ? palette.selected : palette.surfaceRaised },
                    ]}
                  >
                    <Text style={[styles.triggerText, { color: palette.text }]}>
                      {identity.label}
                    </Text>
                    <Text style={{ color: palette.textMuted }}>
                      {state}
                      {chosen ? " · Selected" : ""}
                    </Text>
                    {activityLabel(key) === null ? null : (
                      <Text style={{ color: palette.text }}>{activityLabel(key)}</Text>
                    )}
                    {activity?.tag === "Unavailable" ? (
                      <Text style={{ color: palette.textMuted }}>Activity unavailable</Text>
                    ) : null}
                    {activity?.tag === "Ready" && activity.inner.summary.partial ? (
                      <Text style={{ color: palette.textMuted }}>
                        More activity may be available
                      </Text>
                    ) : null}
                  </Pressable>
                );
              })}
            </ScrollView>
            <Button tone="secondary" onPress={() => setOpen(false)}>
              Cancel
            </Button>
          </View>
        </View>
      </Modal>
    </>
  );
}

const styles = StyleSheet.create({
  trigger: {
    minHeight: 44,
    borderRadius: radius.pill,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    flexDirection: "row",
    alignItems: "center",
    gap: space.sm,
    alignSelf: "flex-start",
    flexShrink: 1,
  },
  triggerText: { fontSize: 16, fontWeight: "600", flexShrink: 1 },
  backdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.4)",
    padding: space.lg,
    alignItems: "center",
    justifyContent: "center",
  },
  sheet: {
    width: "100%",
    maxWidth: 480,
    maxHeight: "85%",
    borderRadius: radius.md,
    borderWidth: 1,
    padding: space.md,
    gap: space.md,
  },
  options: { gap: space.sm },
  option: { padding: space.md, borderRadius: radius.sm, gap: space.xs },
});
