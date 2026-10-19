import type { GroupConversation, GroupTextMessage } from "@prns-internal/expo";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { identifierHex, parseGroupId } from "./format";

const pageSize = 50;
const refreshIntervalMillis = 3_000;

type GroupPage = {
  readonly owner: object;
  readonly conversations: readonly GroupConversation[];
  readonly messages: readonly GroupTextMessage[];
  readonly nextBeforeSequence: bigint | undefined;
  readonly loaded: boolean;
  readonly failure: string | null;
};

export function useGroupData(groupId: Uint8Array | null) {
  const development = useDevelopmentRuntime();
  const { listGroupConversations, listGroupMessages } = development;
  const groupKey = groupId === null ? null : identifierHex(groupId);
  const available = development.availability.type === "available";
  const phase = development.phase;
  const generationId = development.snapshot?.generationId;
  const runtime = development.snapshot?.runtime;
  const identity = development.snapshot?.primaryIdentity;
  const identityKey =
    identity?.tag === "Present" ? identifierHex(identity.inner.identityHash) : null;
  const localOwner = development.ownerKey;
  const owner = useMemo(
    () => ({ groupKey, available, phase, generationId, runtime, identityKey, localOwner }),
    [groupKey, available, phase, generationId, runtime, identityKey, localOwner],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const [page, setPage] = useState<GroupPage>({
    owner,
    conversations: [],
    messages: [],
    nextBeforeSequence: undefined,
    loaded: false,
    failure: null,
  });
  const savedPage = useRef(page);
  const [pendingOwner, setPendingOwner] = useState<object | null>(null);
  const inFlight = useRef<object | null>(null);
  const request = useRef(0);
  const mounted = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      request.current += 1;
    };
  }, []);

  const load = useCallback(
    async (older: boolean) => {
      if (!mounted.current || currentOwner.current !== owner || !available) return;
      if (inFlight.current === owner) return;
      const previous = savedPage.current.owner === owner ? savedPage.current : null;
      if (older && previous?.nextBeforeSequence === undefined) return;
      const ticket = ++request.current;
      const current = () =>
        mounted.current && currentOwner.current === owner && ticket === request.current;
      const next: GroupPage = {
        owner,
        conversations: [],
        messages: [],
        nextBeforeSequence: undefined,
        loaded: true,
        failure: null,
      };
      inFlight.current = owner;
      setPendingOwner(owner);
      let invalidated = false;
      try {
        const listed = await listGroupConversations();
        if (!current()) return;
        if (listed.type === "operationFailure")
          throw new Error("Groups could not be loaded. Try again.");
        const outcome = listed.outcome;
        if (outcome.tag === "Busy") throw new Error("Group storage is busy. Try again shortly.");
        if (outcome.tag === "Unavailable")
          throw new Error("Groups are unavailable right now. Try again.");
        const conversations = outcome.tag === "Ready" ? outcome.inner.conversations : [];
        let messages: readonly GroupTextMessage[] = [];
        let nextBeforeSequence: bigint | undefined;
        const selected = groupKey === null ? null : parseGroupId(groupKey);
        if (
          selected !== null &&
          conversations.some((entry) => identifierHex(entry.groupId) === groupKey)
        ) {
          // Maintenance may delete any cached row. Even Load older replaces an
          // authoritative prefix instead of merging new pages into stale rows.
          const wanted = Math.max(
            pageSize,
            (previous?.messages.length ?? 0) + (older ? pageSize : 0),
          );
          for (let readAttempt = 0; readAttempt < 2; readAttempt += 1) {
            messages = [];
            let beforeSequence: bigint | undefined;
            let revision: bigint | undefined;
            let changed = false;
            while (messages.length < wanted) {
              const response = await listGroupMessages({
                groupId: selected,
                beforeSequence,
                limit: Math.min(pageSize, wanted - messages.length),
              });
              if (!current()) return;
              if (response.type === "operationFailure" || response.outcome.tag !== "Ready") {
                throw new Error("Group messages could not be loaded. Try again.");
              }
              const fetched = response.outcome.inner;
              if (revision !== undefined && revision !== fetched.revision) {
                invalidated = true;
                changed = true;
                break;
              }
              revision = fetched.revision;
              messages = [...messages, ...fetched.messages];
              nextBeforeSequence = fetched.nextBeforeSequence;
              if (nextBeforeSequence === undefined) break;
              if (
                fetched.messages.length === 0 ||
                (beforeSequence !== undefined && nextBeforeSequence >= beforeSequence)
              ) {
                throw new Error("Group history could not be continued. Refresh and try again.");
              }
              beforeSequence = nextBeforeSequence;
            }
            if (!changed) break;
            if (readAttempt === 1)
              throw new Error("Group history changed while loading. Refresh to continue.");
          }
          const seen = new Set<string>();
          messages = messages.filter((message) => {
            const key = identifierHex(message.messageId);
            if (seen.has(key)) return false;
            seen.add(key);
            return true;
          });
        }
        const loaded = { ...next, conversations, messages, nextBeforeSequence };
        savedPage.current = loaded;
        setPage(loaded);
      } catch (failure) {
        if (!current()) return;
        const failed = {
          ...(!invalidated && previous !== null ? previous : next),
          owner,
          loaded: !invalidated && (previous?.loaded ?? false),
          failure:
            failure instanceof Error ? failure.message : "Group messages could not be loaded.",
        };
        savedPage.current = failed;
        setPage(failed);
      } finally {
        if (current()) {
          inFlight.current = null;
          setPendingOwner(null);
        }
      }
    },
    [available, groupKey, listGroupConversations, listGroupMessages, owner],
  );

  const refresh = useCallback(() => load(false), [load]);
  const loadOlder = useCallback(() => load(true), [load]);
  useEffect(() => {
    if (!available) return;
    void refresh();
    const interval = setInterval(() => {
      if (AppState.currentState === "active") void refresh();
    }, refreshIntervalMillis);
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void refresh();
    });
    return () => {
      clearInterval(interval);
      subscription.remove();
    };
  }, [available, refresh]);

  const valid = available && page.owner === owner;
  return {
    conversations: valid ? page.conversations : [],
    messages: valid ? page.messages : [],
    loaded: valid && page.loaded,
    hasMore: valid && page.nextBeforeSequence !== undefined,
    failure: valid ? page.failure : null,
    pending: pendingOwner === owner,
    refresh,
    loadOlder,
  };
}
