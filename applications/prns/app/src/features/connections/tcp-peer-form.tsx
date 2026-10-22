import { useState } from "react";

import { ActionRow, BodyText, Button } from "@/ui/primitives";
import { TextField } from "@/ui/text-field";

export type TcpPeerDraft = {
  readonly label: string;
  readonly host: string;
  readonly port: number;
};

/** Convenience validation only. Native settings validation remains authoritative. */
export function parseTcpPeerDraft(
  label: string,
  host: string,
  port: string,
): TcpPeerDraft | string {
  const address = host.trim();
  const name = label.trim();
  if (!name || /\p{Cc}/u.test(name)) return "Enter a peer name without control characters.";
  const nameBytes = Array.from(name).reduce((length, character) => {
    const code = character.codePointAt(0) ?? 0;
    return length + (code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4);
  }, 0);
  if (nameBytes > 64) return "This peer name is too long. Shorten it and try again.";
  if (!address || /[\s/]/u.test(address) || address.includes("://"))
    return "Enter a hostname or IP address without a URL or port.";
  if (!/^\d+$/u.test(port) || Number(port) < 1 || Number(port) > 65_535)
    return "Enter a port from 1 to 65535.";
  return { label: name, host: address, port: Number(port) };
}

export function TcpPeerForm({
  initial,
  busy,
  canSave = true,
  onSave,
  onCancel,
}: {
  readonly initial?: TcpPeerDraft;
  readonly busy: boolean;
  readonly canSave?: boolean;
  readonly onSave: (draft: TcpPeerDraft) => Promise<boolean>;
  readonly onCancel: () => void;
}) {
  const [label, setLabel] = useState(initial?.label ?? "");
  const [host, setHost] = useState(initial?.host ?? "");
  const [port, setPort] = useState(initial?.port.toString() ?? "4242");
  const [failure, setFailure] = useState<string | null>(null);
  const save = async () => {
    if (busy || !canSave) return;
    const draft = parseTcpPeerDraft(label, host, port);
    if (typeof draft === "string") {
      setFailure(draft);
      return;
    }
    setFailure(null);
    await onSave(draft);
  };
  return (
    <>
      <TextField
        label="Peer name"
        testID="tcp-peer-name"
        value={label}
        onChangeText={setLabel}
        editable={!busy}
        maxLength={64}
      />
      <TextField
        label="Hostname or IP address"
        testID="tcp-peer-host"
        value={host}
        onChangeText={setHost}
        editable={!busy}
        autoCapitalize="none"
        autoCorrect={false}
        maxLength={253}
        placeholder="node.example.org"
      />
      <TextField
        label="Port"
        testID="tcp-peer-port"
        value={port}
        onChangeText={setPort}
        editable={!busy}
        keyboardType="number-pad"
        maxLength={5}
      />
      <BodyText muted>
        You can save a peer while it is offline. Enabled peers reconnect automatically.
      </BodyText>
      {failure === null ? null : <BodyText>{failure}</BodyText>}
      <ActionRow>
        <Button disabled={busy || !canSave} onPress={() => void save()}>
          {busy ? "Saving…" : "Save peer"}
        </Button>
        <Button disabled={busy} tone="secondary" onPress={onCancel}>
          Cancel
        </Button>
      </ActionRow>
    </>
  );
}
