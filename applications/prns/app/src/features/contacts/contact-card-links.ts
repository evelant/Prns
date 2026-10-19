/** Draft portable URI owned by prns-contact-card, independent of this app's scheme. */
export const CONTACT_CARD_URI_PREFIX = "rns-contact://v1/";
export const MAX_CONTACT_CARD_URI_LENGTH = 385;

/** Admission only. Rust verifies the canonical encoding, signature and service binding. */
export function isContactCardUriShape(value: string): boolean {
  return (
    value.length <= MAX_CONTACT_CARD_URI_LENGTH &&
    value.startsWith(CONTACT_CARD_URI_PREFIX) &&
    /^[A-Za-z0-9_-]+$/u.test(value.slice(CONTACT_CARD_URI_PREFIX.length))
  );
}

export function redirectContactCardIntent(path: string): string {
  if (!/^rns-contact:/iu.test(path)) return path;
  return isContactCardUriShape(path)
    ? `/contacts/import?card=${encodeURIComponent(path)}`
    : "/contacts/import?invalid=1";
}
