import { type Href, useRouter } from "expo-router";
import { useMemo } from "react";
import { useNamedIdentity } from "./named-identity-context";

/** Internal message/contact navigation preserves the initiating local identity. */
export function identityHref(href: Href, owner: string | undefined): Href {
  if (owner === undefined || owner === "no-identity") return href;
  const path = typeof href === "string" ? href : href.pathname;
  if (!/^\/(contacts|inbox)(?:\/|\?|$)/.test(path)) return href;
  if (typeof href === "string") {
    if (/(?:\?|&)owner=/.test(href)) return href;
    return `${href}${href.includes("?") ? "&" : "?"}owner=${encodeURIComponent(owner)}` as Href;
  }
  return { ...href, params: { ...href.params, owner: href.params?.owner ?? owner } } as Href;
}

export function useIdentityRouter() {
  const router = useRouter();
  const owner = useNamedIdentity()?.ownerKey;
  return useMemo(
    () => ({
      ...router,
      push: (href: Href) => router.push(identityHref(href, owner)),
      replace: (href: Href) => router.replace(identityHref(href, owner)),
    }),
    [router, owner],
  );
}
