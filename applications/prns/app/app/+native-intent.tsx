import { redirectContactCardIntent } from "@/features/contacts/contact-card-links";

/** External links only open review. They never carry a trusted group or execute a mutation. */
export function redirectSystemPath({ path }: { path: string; initial: boolean }): string {
  return redirectContactCardIntent(path);
}
