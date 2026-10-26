import { Fragment, type PropsWithChildren } from "react";

import { useNamedIdentity } from "./named-identity-context";

/** Reset owner-specific screens without remounting the navigator above them. */
export function MessagingJourney({ children }: PropsWithChildren) {
  const identity = useNamedIdentity();
  const key = identity === null ? "legacy" : `${identity.ownerKey}:${identity.identity?.state}`;
  return <Fragment key={key}>{children}</Fragment>;
}
