import * as Bindings from "@prns-internal/expo";
import {
  createContext,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { useDevelopmentRuntime } from "./development-runtime-context";
import {
  clearPendingIdentityCreation,
  type PendingIdentityCreation,
  readPendingIdentityCreation,
  writePendingIdentityCreation,
} from "./pending-identity-creation";
import { runtimeProvider } from "./runtime-provider";
import { useIdentityActivity } from "./use-identity-activity";

/** A local reference includes the catalog incarnation, never just a public hash. */
export function identityReferenceKey(reference: Bindings.NamedIdentityReference): string {
  const hex = (bytes: Uint8Array) =>
    Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex(reference.incarnation)}-${reference.sequence}-${hex(reference.publicHash)}`;
}

type Catalog = Extract<Bindings.NamedIdentityCatalogOutcome, { tag: "Ready" }>["inner"]["catalog"];
export type NamedIdentity = Catalog["identities"][number];
type LifecycleIntent = {
  readonly ownerKey: string;
  readonly state: Bindings.NamedIdentityState;
};
type NamedIdentityView = {
  readonly catalog: Catalog | null;
  readonly identity: NamedIdentity | null;
  readonly ownerKey: string;
  readonly runtime: Bindings.DevelopmentRuntime | null;
  readonly canPublish: boolean;
  readonly activity: readonly Bindings.NamedIdentityActivity[] | null;
  readonly activityFailure: string | null;
  readonly refreshActivity: () => Promise<void>;
  readonly pendingCreationLabel: string | null;
  readonly creationReady: boolean;
  readonly loading: boolean;
  readonly failure: string | null;
  readonly refresh: () => Promise<void>;
  readonly create: (label: string) => Promise<boolean>;
  readonly change: (
    identity: Bindings.NamedIdentityReference,
    change: Bindings.NamedIdentityChange,
  ) => Promise<boolean>;
};

const NamedIdentityContext = createContext<NamedIdentityView | null>(null);

export function useNamedIdentity() {
  return useContext(NamedIdentityContext);
}

export function NamedIdentityProvider({
  children,
  pathname,
  requestedOwner,
  runtime: providedRuntime,
}: PropsWithChildren<{
  readonly pathname: string;
  readonly requestedOwner?: string;
  readonly runtime?: Bindings.DevelopmentRuntime | null;
}>) {
  const development = useDevelopmentRuntime();
  const generation = development.snapshot?.generationId;
  const hostRunning =
    development.phase === "ready" &&
    !development.stoppingNode &&
    development.snapshot?.runtime === Bindings.DevelopmentNodeRuntime.Running;
  const runtime =
    providedRuntime === undefined
      ? "runtime" in runtimeProvider
        ? runtimeProvider.runtime
        : null
      : providedRuntime;
  const activity = useIdentityActivity(runtime, generation);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [catalogGeneration, setCatalogGeneration] = useState<bigint | undefined>(undefined);
  const [failure, setFailure] = useState<string | null>(null);
  const [readFailure, setReadFailure] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [lifecycle, setLifecycle] = useState<LifecycleIntent | null>(null);
  const requests = useRef(0);
  const mounted = useRef(true);
  const operation = useRef(false);
  const pendingCreate = useRef<PendingIdentityCreation | null>(null);
  const [creationReady, setCreationReady] = useState(false);
  const [pendingCreationLabel, setPendingCreationLabel] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    void readPendingIdentityCreation()
      .then((intent) => {
        if (!current) return;
        pendingCreate.current = intent;
        setPendingCreationLabel(intent?.label ?? null);
        setCreationReady(true);
      })
      .catch(() => {
        if (current)
          setFailure(
            "Saved identity creation could not be loaded. Restart the app before creating another identity.",
          );
      });
    return () => {
      current = false;
    };
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      requests.current++;
    };
  }, []);
  const publish = useCallback(
    (outcome: Bindings.NamedIdentityCatalogOutcome, fromRead = false) => {
      const report = fromRead ? setReadFailure : setFailure;
      if (!mounted.current) return false;
      if (outcome.tag === "Ready") {
        setCatalog(outcome.inner.catalog);
        setCatalogGeneration(generation);
        report(null);
        return true;
      }
      report(
        outcome.tag === "Busy"
          ? "Identity storage is busy. Try again shortly."
          : outcome.inner.detail,
      );
      return false;
    },
    [generation],
  );
  const refresh = useCallback(async () => {
    if (runtime === null || operation.current) return;
    const ticket = ++requests.current;
    try {
      const outcome = await runtime.listNamedIdentities();
      if (ticket === requests.current) publish(outcome, true);
    } catch {
      if (mounted.current && ticket === requests.current)
        setReadFailure("Identities could not be loaded. Try again.");
    }
  }, [publish, runtime]);
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => {
      clearInterval(timer);
      requests.current++;
    };
  }, [refresh]);
  const mutate = useCallback(
    async (
      action: () => Promise<Bindings.NamedIdentityCatalogOutcome>,
      lifecycleIntent?: LifecycleIntent,
    ) => {
      if (operation.current) return false;
      operation.current = true;
      requests.current++;
      setLoading(true);
      if (lifecycleIntent !== undefined) setLifecycle(lifecycleIntent);
      setFailure(null);
      try {
        return publish(await action());
      } catch {
        if (mounted.current)
          setFailure("The identity change could not be confirmed. Retry the same action.");
        return false;
      } finally {
        operation.current = false;
        if (mounted.current) {
          setLoading(false);
          setLifecycle(null);
        }
      }
    },
    [publish],
  );
  const create = useCallback(
    async (label: string) => {
      if (runtime === null || !creationReady) return false;
      if (pendingCreate.current !== null && pendingCreate.current.label !== label) {
        setFailure("Finish the saved identity creation before starting another one.");
        return false;
      }
      return mutate(async () => {
        if (pendingCreate.current === null) {
          const request = await runtime.createGroupRequestId();
          if (request.tag !== "Ready") throw new Error("Request identity unavailable");
          const intent = { label, requestId: request.inner.clientRequestId };
          await writePendingIdentityCreation(intent);
          pendingCreate.current = intent;
          setPendingCreationLabel(label);
        }
        const outcome = await runtime.createNamedIdentity({
          ...pendingCreate.current,
          publishedName: undefined,
        });
        // Rejected is a native pre-admission decision; uncertain outcomes retain intent.
        if (outcome.tag === "Ready" || outcome.tag === "Rejected") {
          await clearPendingIdentityCreation();
          pendingCreate.current = null;
          setPendingCreationLabel(null);
        }
        return outcome;
      });
    },
    [mutate, runtime, creationReady],
  );
  const change = useCallback(
    (identity: Bindings.NamedIdentityReference, change: Bindings.NamedIdentityChange) => {
      if (runtime === null) return Promise.resolve(false);
      const ownerKey = identityReferenceKey(identity);
      const lifecycleIntent =
        change.tag === "Pause" || change.tag === "Resume"
          ? {
              ownerKey,
              state:
                change.tag === "Pause"
                  ? Bindings.NamedIdentityState.Paused
                  : Bindings.NamedIdentityState.Enabled,
            }
          : undefined;
      const reconcileUnconfirmedLifecycle = async () => {
        if (lifecycleIntent === undefined) return;
        // An uncertain transition must not restore stale publication readiness.
        // Keep other owners usable; a fresh native catalog restores this owner.
        if (mounted.current)
          setCatalog((current) =>
            current === null
              ? null
              : {
                  ...current,
                  identities: current.identities.map((entry) =>
                    identityReferenceKey(entry.reference) === ownerKey
                      ? { ...entry, ready: false }
                      : entry,
                  ),
                },
          );
        try {
          publish(await runtime.listNamedIdentities(), true);
        } catch {
          if (mounted.current)
            setReadFailure("The identity's current state could not be confirmed. Try again.");
        }
      };
      return mutate(async () => {
        try {
          const outcome = await runtime.changeNamedIdentity({ identity, change });
          if (outcome.tag !== "Ready") await reconcileUnconfirmedLifecycle();
          return outcome;
        } catch (error) {
          await reconcileUnconfirmedLifecycle();
          throw error;
        }
      }, lifecycleIntent);
    },
    [mutate, runtime, publish],
  );

  const visibleCatalog =
    catalog === null || lifecycle === null
      ? catalog
      : {
          ...catalog,
          identities: catalog.identities.map((entry) =>
            identityReferenceKey(entry.reference) === lifecycle.ownerKey
              ? { ...entry, state: lifecycle.state, ready: false, transitioning: true }
              : entry,
          ),
        };
  const selectedKey =
    visibleCatalog?.selected === undefined ? null : identityReferenceKey(visibleCatalog.selected);
  // Detail journeys without an owner parameter capture the current selection once.
  // Normal internal links carry owner explicitly, including the back path.
  const pinned = useRef<{ pathname: string; key: string | null }>({ pathname, key: null });
  const detail = /^\/(contacts|inbox)\/.+/.test(pathname);
  if (pinned.current.pathname !== pathname) pinned.current = { pathname, key: null };
  if (detail && pinned.current.key === null && selectedKey !== null)
    pinned.current.key = selectedKey;
  const ownerKey = requestedOwner ?? (detail ? pinned.current.key : selectedKey) ?? "no-identity";
  const identity =
    visibleCatalog?.identities.find(
      (entry) => identityReferenceKey(entry.reference) === ownerKey,
    ) ?? null;
  const reference = identity?.reference;
  // Refreshes change metadata, not immutable client ownership.
  const scoped = useRef<{
    base: Bindings.DevelopmentRuntime | null;
    key: string;
    runtime: Bindings.DevelopmentRuntime | null;
  }>({ base: null, key: "", runtime: null });
  if (
    scoped.current.base !== runtime ||
    scoped.current.key !== ownerKey ||
    (scoped.current.runtime === null && reference !== undefined)
  ) {
    scoped.current = {
      base: runtime,
      key: ownerKey,
      runtime:
        reference === undefined || runtime === null
          ? null
          : { ...runtime, ...runtime.forIdentity(reference) },
    };
  }
  const scopedRuntime = reference === undefined ? null : scoped.current.runtime;
  const value: NamedIdentityView = {
    ...activity,
    catalog: visibleCatalog,
    identity,
    ownerKey,
    runtime: scopedRuntime,
    canPublish:
      hostRunning &&
      catalogGeneration === generation &&
      identity?.state === Bindings.NamedIdentityState.Enabled &&
      !identity.transitioning &&
      identity.ready,
    pendingCreationLabel,
    creationReady,
    loading,
    failure: failure ?? readFailure,
    refresh,
    create,
    change,
  };
  return <NamedIdentityContext.Provider value={value}>{children}</NamedIdentityContext.Provider>;
}
