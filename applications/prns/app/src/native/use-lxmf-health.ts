import * as Bindings from "@prns-internal/expo";
import { useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";

type Page = {
  readonly owner: object;
  readonly health: Bindings.LxmfHealth | null;
  readonly revision: number;
};

/** Observe the captured identity's service without acquiring or starting a node. */
export function useLxmfHealth(
  runtime: Pick<Bindings.DevelopmentRuntime, "waitLxmfChange"> | null,
  ownerKey: string,
  generation: bigint | undefined,
  enabled: boolean,
) {
  const owner = useMemo(
    () => ({ runtime, ownerKey, generation, enabled }),
    [runtime, ownerKey, generation, enabled],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const [page, setPage] = useState<Page>({ owner, health: null, revision: 0 });

  useEffect(() => {
    if (runtime === null || !enabled) return;
    let retired = false;
    let foreground = AppState.currentState !== "background" && AppState.currentState !== "inactive";
    let cursor: bigint | undefined;
    let controller: AbortController | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;
    const current = () => !retired && foreground && currentOwner.current === owner;
    const publish = (health: Bindings.LxmfHealth | null) => {
      setPage((previous) => {
        if (!current()) return previous;
        if (health === null && previous.owner === owner && previous.health === null)
          return previous;
        return {
          owner,
          health,
          // A new foreground attachment can have the same counter as a retired
          // service. It still refreshes durable rows without a radio update.
          revision: (previous.owner === owner ? previous.revision : 0) + 1,
        };
      });
    };
    const observe = async (): Promise<void> => {
      if (!current() || controller !== null) return;
      const pending = new AbortController();
      controller = pending;
      const after = cursor;
      let backoff = true;
      try {
        const health = await runtime.waitLxmfChange(after, pending.signal);
        if (!current() || controller !== pending || pending.signal.aborted) return;
        if (health === undefined || health.state === Bindings.LxmfHealthState.Stopped) {
          cursor = undefined;
          publish(null);
        } else {
          cursor = health.projectionRevision;
          publish(health);
          backoff = after === cursor;
        }
      } catch {
        if (!current() || controller !== pending || pending.signal.aborted) return;
        cursor = undefined;
        publish(null);
      } finally {
        if (controller === pending) controller = null;
      }
      if (!current() || pending.signal.aborted) return;
      if (backoff) {
        retry = setTimeout(() => {
          retry = null;
          void observe();
        }, 1000);
      } else {
        void observe();
      }
    };
    const cancel = () => {
      controller?.abort();
      controller = null;
      if (retry !== null) clearTimeout(retry);
      retry = null;
      cursor = undefined;
    };
    const listener = AppState.addEventListener("change", (state) => {
      const active = state === "active";
      if (active === foreground) return;
      foreground = active;
      cancel();
      if (active) void observe();
    });
    void observe();
    return () => {
      retired = true;
      cancel();
      listener.remove();
    };
  }, [owner, runtime, enabled]);

  return page.owner === owner ? page : { owner, health: null, revision: 0 };
}
