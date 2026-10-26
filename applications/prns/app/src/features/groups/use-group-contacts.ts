import type { Contact } from "@prns-internal/expo";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";

import { useContactRuntime } from "@/native/contact-runtime-context";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { identifierHex } from "./format";

/** Contact names are local labels, never authority for enrollment. */
export function useGroupContacts() {
  const { runtime } = useContactRuntime();
  const development = useDevelopmentRuntime();
  const identity = development.snapshot?.primaryIdentity;
  const identityKey =
    identity?.tag === "Present" ? identifierHex(identity.inner.identityHash) : null;
  const generation = development.snapshot?.generationId;
  const localOwner = development.ownerKey;
  const owner = useMemo(
    () => ({ runtime, identityKey, generation, localOwner }),
    [runtime, identityKey, generation, localOwner],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const mounted = useRef(false);
  const ticket = useRef(0);
  const active = useRef<object | null>(null);
  const [pendingOwner, setPendingOwner] = useState<object | null>(null);
  const [page, setPage] = useState<{
    readonly owner: object;
    readonly contacts: readonly Contact[];
    readonly failure: string | null;
    readonly loaded: boolean;
  }>({ owner, contacts: [], failure: null, loaded: false });
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      ticket.current += 1;
    };
  }, []);
  const refresh = useCallback(async () => {
    if (
      !mounted.current ||
      runtime === null ||
      currentOwner.current !== owner ||
      active.current === owner
    )
      return;
    const request = ++ticket.current;
    active.current = owner;
    setPendingOwner(owner);
    const current = () =>
      mounted.current && currentOwner.current === owner && ticket.current === request;
    try {
      const result = await runtime.listContacts();
      if (!current()) return;
      if (result.tag !== "Listed") throw new Error("Contacts unavailable");
      setPage({ owner, contacts: result.inner.contacts, failure: null, loaded: true });
    } catch {
      if (current())
        setPage((previous) => ({
          owner,
          contacts: previous.owner === owner ? previous.contacts : [],
          loaded: previous.owner === owner && previous.loaded,
          failure: "Saved contacts could not be loaded. Try reloading them.",
        }));
    } finally {
      if (current()) {
        active.current = null;
        setPendingOwner(null);
      }
    }
  }, [owner, runtime]);
  useEffect(() => {
    void refresh();
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void refresh();
    });
    return () => subscription.remove();
  }, [refresh]);
  const valid = runtime !== null && page.owner === owner;
  return {
    contacts: valid ? page.contacts : [],
    failure: valid ? page.failure : null,
    loaded: valid && page.loaded,
    pending: pendingOwner === owner,
    refresh,
  };
}
