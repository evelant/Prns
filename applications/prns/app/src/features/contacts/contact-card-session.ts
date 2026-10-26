import { useRef } from "react";

import { useContactRuntime } from "@/native/contact-runtime-context";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { formatContactHash } from "./format";

/** A native mutation must still belong to the identity that reviewed its card. */
export function useContactCardSession() {
  const { runtime } = useContactRuntime();
  const { snapshot, phase, ownerKey } = useDevelopmentRuntime();
  const primary = snapshot?.primaryIdentity;
  const identity = primary?.tag === "Present" ? primary.inner.identityHash : null;
  const valueKey = `${ownerKey ?? "legacy"}:${identity === null ? "none" : formatContactHash(identity)}:${snapshot?.generationId}:${phase}`;
  const owner = useRef({ runtime, identity, valueKey, revision: 0, key: valueKey });
  // Fresh snapshot byte arrays do not change ownership; a replaced runtime does.
  if (owner.current.runtime !== runtime || owner.current.valueKey !== valueKey) {
    const revision = owner.current.revision + 1;
    owner.current = { runtime, identity, valueKey, revision, key: `${valueKey}:${revision}` };
  }
  return owner.current;
}
