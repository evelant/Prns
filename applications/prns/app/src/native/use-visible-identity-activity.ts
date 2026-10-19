import type * as Bindings from "@prns-internal/expo";
import { useFocusEffect } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { useNamedIdentity } from "./named-identity-context";

function conversationKey(scope: Bindings.IdentityActivityScope | null) {
  if (scope === null) return null;
  const bytes = scope.tag === "Group" ? scope.inner.groupId : scope.inner.destination;
  return `${scope.tag}:${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

/** Acknowledge only an incoming-record watermark actually rendered by a visible detail page. */
export function useVisibleIdentityActivity(
  scope: Bindings.IdentityActivityScope | null,
  throughSequence: bigint | undefined,
): void {
  const identity = useNamedIdentity();
  const client = identity?.runtime;
  const ownerKey = identity?.ownerKey;
  const key = conversationKey(scope);
  const [focused, setFocused] = useState(false);
  const [foreground, setForeground] = useState(AppState.currentState === "active");
  const [revision, setRevision] = useState(0);
  const mounted = useRef(false);
  const latest = useRef({
    client,
    ownerKey,
    key,
    scope,
    focused,
    foreground,
    throughSequence,
    refresh: identity?.refreshActivity,
  });
  latest.current = {
    client,
    ownerKey,
    key,
    scope,
    focused,
    foreground,
    throughSequence,
    refresh: identity?.refreshActivity,
  };
  const pending = useRef<object | null>(null);
  const seen = useRef<{ ownerKey: string; key: string; through: bigint } | null>(null);
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    mounted.current = true;
    const listener = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => {
      mounted.current = false;
      listener.remove();
      if (retry.current !== null) clearTimeout(retry.current);
    };
  }, []);
  useFocusEffect(
    useCallback(() => {
      setFocused(true);
      return () => setFocused(false);
    }, []),
  );
  useEffect(() => {
    if (retry.current !== null) {
      clearTimeout(retry.current);
      retry.current = null;
    }
    const capturedScope = latest.current.scope;
    if (
      !focused ||
      !foreground ||
      client == null ||
      ownerKey === undefined ||
      key === null ||
      capturedScope === null ||
      throughSequence === undefined ||
      pending.current !== null
    )
      return;
    if (
      seen.current?.ownerKey === ownerKey &&
      seen.current.key === key &&
      seen.current.through >= throughSequence
    )
      return;
    const ticket = { revision };
    pending.current = ticket;
    const current = () =>
      mounted.current &&
      latest.current.ownerKey === ownerKey &&
      latest.current.key === key &&
      latest.current.client === client;
    let saved = false;
    let retryable = false;
    void client
      .markIdentityActivitySeen({ scope: capturedScope, throughSequence })
      .then((result) => {
        retryable = result.tag === "Unavailable";
        if (!current() || result.tag !== "Saved") return;
        saved = true;
        seen.current = { ownerKey, key, through: throughSequence };
        void latest.current.refresh?.();
      })
      .catch(() => {
        retryable = true;
        /* Counts remain unread until native storage confirms the watermark. */
      })
      .finally(() => {
        if (pending.current === ticket) pending.current = null;
        if (!mounted.current) return;
        if (!current() || saved || latest.current.throughSequence !== throughSequence)
          setRevision((value) => value + 1);
        else if (retryable && latest.current.focused && latest.current.foreground)
          retry.current = setTimeout(() => {
            retry.current = null;
            setRevision((value) => value + 1);
          }, 5000);
      });
    return () => {
      if (retry.current !== null) {
        clearTimeout(retry.current);
        retry.current = null;
      }
    };
  }, [client, ownerKey, key, throughSequence, focused, foreground, revision]);
}
