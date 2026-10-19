import type * as Bindings from "@prns-internal/expo";
import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";

/** A global projection only. Reading or choosing an identity never marks messages seen. */
export function useIdentityActivity(
  runtime: Bindings.DevelopmentRuntime | null,
  generation: bigint | undefined,
) {
  const owner = useRef({ runtime, generation });
  if (owner.current.runtime !== runtime || owner.current.generation !== generation)
    owner.current = { runtime, generation };
  const currentOwner = owner.current;
  const inFlight = useRef<{ owner: typeof currentOwner; ticket: number } | null>(null);
  const request = useRef(0);
  const mounted = useRef(false);
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [page, setPage] = useState<{
    owner: typeof currentOwner;
    entries: readonly Bindings.NamedIdentityActivity[] | null;
    failure: string | null;
  }>({ owner: currentOwner, entries: null, failure: null });
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      request.current++;
    };
  }, []);
  const refresh = useCallback(
    async (allowRetry = true): Promise<void> => {
      if (runtime === null || inFlight.current?.owner === currentOwner) return;
      if (retry.current !== null) {
        clearTimeout(retry.current);
        retry.current = null;
      }
      const ticket = ++request.current;
      inFlight.current = { owner: currentOwner, ticket };
      const current = () =>
        mounted.current && owner.current === currentOwner && request.current === ticket;
      let contended = false;
      try {
        const result = await runtime.listNamedIdentityActivity();
        if (!current()) return;
        contended =
          result.tag === "Busy" ||
          (result.tag === "Ready" &&
            result.inner.activities.some((entry) => entry.activity.tag === "Unavailable"));
        if (result.tag === "Ready")
          setPage({ owner: currentOwner, entries: result.inner.activities, failure: null });
        else
          setPage({
            owner: currentOwner,
            entries: null,
            failure:
              result.tag === "Busy" ? "Activity is busy. Try again shortly." : result.inner.detail,
          });
      } catch {
        if (current())
          setPage({ owner: currentOwner, entries: null, failure: "Activity could not be loaded." });
      } finally {
        if (inFlight.current?.ticket === ticket) inFlight.current = null;
        // Inbox and activity both poll every three seconds. One offset read avoids
        // staying synchronized with the same owner's short local-storage job.
        if (allowRetry && contended && current() && AppState.currentState === "active")
          retry.current = setTimeout(() => {
            retry.current = null;
            if (current() && AppState.currentState === "active") void refresh(false);
          }, 400);
      }
    },
    [currentOwner, runtime],
  );
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => {
      if (AppState.currentState === "active") void refresh();
    }, 3000);
    const listener = AppState.addEventListener("change", (state) => {
      if (state === "active") void refresh();
      else if (retry.current !== null) {
        clearTimeout(retry.current);
        retry.current = null;
      }
    });
    return () => {
      clearInterval(timer);
      listener.remove();
      if (retry.current !== null) {
        clearTimeout(retry.current);
        retry.current = null;
      }
      request.current++;
      inFlight.current = null;
    };
  }, [refresh]);
  return {
    activity: page.owner === currentOwner ? page.entries : null,
    activityFailure: page.owner === currentOwner ? page.failure : null,
    refreshActivity: refresh,
  };
}
