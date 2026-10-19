import type { GroupInvitation } from "@prns-internal/expo";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { identifierHex } from "./format";

export function useGroupInvitations() {
  const development = useDevelopmentRuntime();
  const { listGroupInvitations } = development;
  const available = development.availability.type === "available";
  const generation = development.snapshot?.generationId;
  const phase = development.phase;
  const runtime = development.snapshot?.runtime;
  const identity = development.snapshot?.primaryIdentity;
  const identityKey =
    identity?.tag === "Present" ? identifierHex(identity.inner.identityHash) : null;
  const localOwner = development.ownerKey;
  const owner = useMemo(
    () => ({ available, generation, phase, runtime, identityKey, localOwner }),
    [available, generation, phase, runtime, identityKey, localOwner],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const request = useRef(0);
  const active = useRef<object | null>(null);
  const mounted = useRef(false);
  const [page, setPage] = useState<{
    readonly owner: object;
    readonly invitations: readonly GroupInvitation[];
    readonly failure: string | null;
    readonly loaded: boolean;
  }>({ owner, invitations: [], failure: null, loaded: false });
  const [pendingOwner, setPendingOwner] = useState<object | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      request.current += 1;
    };
  }, []);
  const refresh = useCallback(async () => {
    if (
      !mounted.current ||
      !available ||
      currentOwner.current !== owner ||
      active.current === owner
    )
      return;
    const ticket = ++request.current;
    active.current = owner;
    setPendingOwner(owner);
    const current = () =>
      mounted.current && currentOwner.current === owner && request.current === ticket;
    try {
      const response = await listGroupInvitations();
      if (!current()) return;
      if (response.type !== "outcome")
        throw new Error("Invitations could not be loaded. Try again.");
      switch (response.outcome.tag) {
        case "Ready":
          setPage({
            owner,
            invitations: response.outcome.inner.invitations,
            loaded: true,
            failure: null,
          });
          break;
        case "Busy":
          throw new Error("Invitation storage is busy. Try again shortly.");
        case "Unavailable":
          throw new Error(response.outcome.inner.detail);
      }
    } catch (error) {
      if (current())
        setPage((previous) => ({
          owner,
          invitations: previous.owner === owner ? previous.invitations : [],
          loaded: previous.owner === owner && previous.loaded,
          failure:
            error instanceof Error ? error.message : "Invitations could not be loaded. Try again.",
        }));
    } finally {
      if (current()) {
        active.current = null;
        setPendingOwner(null);
      }
    }
  }, [available, listGroupInvitations, owner]);
  useEffect(() => {
    void refresh();
    const interval = setInterval(() => {
      if (AppState.currentState === "active") void refresh();
    }, 3_000);
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void refresh();
    });
    return () => {
      clearInterval(interval);
      subscription.remove();
    };
  }, [refresh]);
  const valid = available && page.owner === owner;
  return {
    invitations: valid ? page.invitations : [],
    loaded: valid && page.loaded,
    failure: valid ? page.failure : null,
    pending: pendingOwner === owner,
    refresh,
  };
}
