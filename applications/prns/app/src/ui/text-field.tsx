import { useCallback, useContext, useRef, useState } from "react";
import { StyleSheet, Text, TextInput, type TextInputProps, View } from "react-native";
import { FieldKeyboardContext } from "./android-keyboard-screen";
import { radius, space, useAppPalette } from "./theme";

type TextFieldProps = TextInputProps & {
  readonly label: string;
  readonly hideLabel?: boolean;
  readonly compact?: boolean;
};

export function TextField({
  accessibilityLabel,
  editable = true,
  label,
  hideLabel = false,
  compact = false,
  multiline = false,
  onBlur,
  onFocus,
  onSelectionChange,
  onContentSizeChange,
  placeholderTextColor,
  style,
  ...props
}: TextFieldProps) {
  const palette = useAppPalette();
  const [focused, setFocused] = useState(false);
  const keyboard = useContext(FieldKeyboardContext);
  const input = useRef<TextInput | null>(null);
  const blur = keyboard?.blur;
  const inputRef = useCallback(
    (next: TextInput | null) => {
      if (next === null) blur?.(input.current);
      input.current = next;
    },
    [blur],
  );

  return (
    <View style={styles.field}>
      {hideLabel ? null : <Text style={[styles.label, { color: palette.text }]}>{label}</Text>}
      <TextInput
        {...props}
        ref={inputRef}
        accessibilityLabel={accessibilityLabel ?? label}
        editable={editable}
        multiline={multiline}
        onBlur={(event) => {
          setFocused(false);
          keyboard?.blur(input.current);
          onBlur?.(event);
        }}
        onFocus={(event) => {
          setFocused(true);
          keyboard?.focus(input.current);
          onFocus?.(event);
        }}
        onSelectionChange={(event) => {
          keyboard?.reveal(input.current);
          onSelectionChange?.(event);
        }}
        onContentSizeChange={(event) => {
          keyboard?.reveal(input.current);
          onContentSizeChange?.(event);
        }}
        placeholderTextColor={placeholderTextColor ?? palette.textMuted}
        style={[
          styles.input,
          multiline ? styles.multiline : null,
          compact ? { minHeight: 48 } : null,
          { backgroundColor: palette.background, color: palette.text },
          style,
          multiline && keyboard?.maximumInputHeight !== undefined
            ? {
                minHeight: Math.min(compact ? 48 : 128, keyboard.maximumInputHeight),
                maxHeight: keyboard.maximumInputHeight,
              }
            : null,
          {
            borderColor: focused && editable ? palette.focus : palette.border,
            opacity: editable ? 1 : 0.6,
          },
        ]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  field: { gap: space.xs, minWidth: 0, paddingVertical: 2 },
  label: { fontSize: 14, fontWeight: "600", lineHeight: 20 },
  input: {
    borderRadius: radius.sm,
    borderWidth: 1,
    fontSize: 16,
    minHeight: 48,
    minWidth: 0,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  multiline: { minHeight: 128, textAlignVertical: "top" },
});
