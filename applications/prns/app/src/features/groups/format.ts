import {
  type Contact,
  type GroupInvitation,
  GroupInvitationState,
  type GroupTextMessage,
} from "@prns-internal/expo";

/** Names are local address-book labels, never membership or person verification. */
export function groupDeviceLabel(identity: Uint8Array, contacts: readonly Contact[]): string {
  const key = identifierHex(identity);
  const contact = contacts.find(
    (entry) => entry.identity !== undefined && identifierHex(entry.identity) === key,
  );
  return contact?.alias?.trim() || contact?.announcedName?.trim() || `Device ${key.slice(0, 8)}`;
}

/** Match native UTF-8 admission limits without relying on a platform TextEncoder. */
export function groupUtf8Length(value: string): number {
  let bytes = 0;
  for (const character of value) {
    const point = character.codePointAt(0) ?? 0;
    bytes += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
  }
  return bytes;
}

export function identifierHex(groupId: Uint8Array): string {
  return Array.from(groupId, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function parseGroupId(value: string): Uint8Array | null {
  return parseIdentifier(value);
}

export function parseInvitationId(value: string): Uint8Array | null {
  return parseIdentifier(value);
}

function parseIdentifier(value: string): Uint8Array | null {
  if (!/^[0-9a-fA-F]{64}$/u.test(value)) return null;
  return Uint8Array.from({ length: 32 }, (_, index) =>
    Number.parseInt(value.slice(index * 2, index * 2 + 2), 16),
  );
}

export function groupDeliveryLabel(message: GroupTextMessage): string {
  if (!message.outgoing) return "Received on this device";
  if (message.membershipChanged)
    return "Needs attention · Delivery interrupted by membership change";
  if (message.recipientCount === 0) return "Saved locally";
  if (message.receivedCount === 0)
    return `Saved locally · Waiting for devices · 0 of ${message.recipientCount} devices received`;
  return message.receivedCount < message.recipientCount
    ? `Waiting for devices · Received by ${message.receivedCount} of ${message.recipientCount} devices`
    : `Received by ${message.receivedCount} of ${message.recipientCount} devices`;
}

export function invitationStateLabel(invitation: GroupInvitation): string {
  if (
    invitation.waitingForDelivery &&
    (invitation.state === GroupInvitationState.Offered ||
      invitation.state === GroupInvitationState.Joining)
  )
    return "Waiting for current deliveries";
  switch (invitation.state) {
    case GroupInvitationState.Offered:
      return invitation.outgoing ? "Waiting for a response" : "Invitation received";
    case GroupInvitationState.Joining:
      return "Joining · waiting for membership confirmation";
    case GroupInvitationState.Joined:
      return invitation.outgoing ? "Device joined" : "Joined";
    case GroupInvitationState.Declined:
      return "Invitation declined";
    case GroupInvitationState.Expired:
      return "Invitation expired";
  }
}
