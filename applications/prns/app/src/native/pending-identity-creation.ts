import AsyncStorage from "@react-native-async-storage/async-storage";

const key = "prns.pending-identity-creation.v1";
export type PendingIdentityCreation = { readonly label: string; readonly requestId: Uint8Array };

/** Persist only UI intent; keys and exactly-once creation remain owned by Rust. */
export async function readPendingIdentityCreation(): Promise<PendingIdentityCreation | null> {
  const stored = await AsyncStorage.getItem(key);
  if (stored === null) return null;
  const value: unknown = JSON.parse(stored);
  if (
    typeof value !== "object" ||
    value === null ||
    !("label" in value) ||
    !("requestId" in value) ||
    typeof value.label !== "string" ||
    value.label.length === 0 ||
    value.label.length > 64 ||
    typeof value.requestId !== "string" ||
    !/^[a-f0-9]{64}$/.test(value.requestId)
  )
    throw new Error("Saved identity creation needs attention");
  return {
    label: value.label,
    requestId: Uint8Array.from(value.requestId.match(/../g) ?? [], (byte) =>
      Number.parseInt(byte, 16),
    ),
  };
}
export async function writePendingIdentityCreation(intent: PendingIdentityCreation): Promise<void> {
  await AsyncStorage.setItem(
    key,
    JSON.stringify({
      label: intent.label,
      requestId: Array.from(intent.requestId, (byte) => byte.toString(16).padStart(2, "0")).join(
        "",
      ),
    }),
  );
}
export async function clearPendingIdentityCreation(): Promise<void> {
  await AsyncStorage.removeItem(key);
}
