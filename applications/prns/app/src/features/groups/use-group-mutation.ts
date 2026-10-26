import * as Bindings from "@prns-internal/expo";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { identifierHex } from "./format";
import { groupRejectionMessage } from "./group-action-feedback";

type Kind = `${Bindings.GroupActionInput["tag"]}`;

function isLocalAction(kind: Kind) {
  return kind === "DeleteLocalGroup" || kind === "DismissInvitation";
}

export function groupActionMatches(input: Bindings.GroupActionInput, kind: Kind, scope?: string) {
  if (input.tag !== kind) return false;
  switch (input.tag) {
    case "Create":
      return true;
    case "Invite":
    case "Send":
    case "RemoveMember":
    case "LeaveGroup":
    case "CloseGroup":
    case "DeleteLocalGroup":
      return identifierHex(input.inner.input.groupId) === scope;
    case "Respond":
    case "CancelInvitation":
    case "DismissInvitation":
      return identifierHex(input.inner.input.invitationId) === scope;
  }
}

/** Re-mount editable drafts when identity ownership changes, but not on reconnect. */
export function useGroupIdentityScope(scope: string) {
  const development = useDevelopmentRuntime();
  const identity = development.snapshot?.primaryIdentity;
  return `${development.ownerKey ?? "legacy"}:${identity?.tag === "Present" ? identifierHex(identity.inner.identityHash) : "no-identity"}:${scope}`;
}

function actionFailure(action: Bindings.GroupAction): string | null {
  if (action.state.tag === "RetryExpired")
    return "This retry has expired. The original action may already have completed. Review saved groups, invitations and messages before starting another action.";
  if (action.state.tag === "Completed")
    return action.state.inner.result.tag === "Rejected"
      ? groupRejectionMessage(action.state.inner.result.inner.reason)
      : null;
  return "Saved locally. Confirmation is still needed. Retry this same action to check its saved result.";
}

/** Native storage owns immutable input before dispatch, including across UI/process loss. */
export function useGroupMutation(kind: Kind, scope?: Uint8Array) {
  const development = useDevelopmentRuntime();
  const {
    listGroupActions,
    prepareGroupAction,
    executeGroupAction,
    acknowledgeGroupAction,
    discardExpiredGroupAction,
    createGroupRequestId,
  } = development;
  const available = development.availability.type === "available";
  const canMutate = development.canPublish !== false || isLocalAction(kind);
  const generation = development.snapshot?.generationId;
  const phase = development.phase;
  const runtime = development.snapshot?.runtime;
  const identity = development.snapshot?.primaryIdentity;
  const identityKey =
    identity?.tag === "Present" ? identifierHex(identity.inner.identityHash) : undefined;
  const scopeKey = scope === undefined ? undefined : identifierHex(scope);
  const localOwner = development.ownerKey;
  const owner = useMemo(
    () => ({ available, generation, phase, runtime, identityKey, kind, scopeKey, localOwner }),
    [available, generation, phase, runtime, identityKey, kind, scopeKey, localOwner],
  );
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const mounted = useRef(false);
  const active = useRef<object | null>(null);
  const selected = useRef<Bindings.GroupAction | null>(null);
  const readTicket = useRef(0);
  const [state, setState] = useState<{
    owner: object;
    action: Bindings.GroupAction | null;
    ready: boolean;
    failure: string | null;
  }>({ owner, action: null, ready: false, failure: null });
  const [pendingOwner, setPendingOwner] = useState<object | null>(null);
  const current = useCallback(() => mounted.current && currentOwner.current === owner, [owner]);
  const publish = useCallback(
    (
      action: Bindings.GroupAction | null,
      failure = action === null ? null : actionFailure(action),
    ) => {
      // A newer mutation or authoritative read supersedes every older read.
      readTicket.current += 1;
      selected.current = action;
      setState({ owner, action, ready: true, failure });
    },
    [owner],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      readTicket.current += 1;
    };
  }, []);

  const refresh = useCallback(
    async (emptyFailure: string | null = null) => {
      if (!available || !current()) return;
      const ticket = ++readTicket.current;
      try {
        const response = await listGroupActions();
        if (!current() || ticket !== readTicket.current) return;
        if (response.type !== "outcome" || response.outcome.tag !== "Ready") {
          setState({
            owner,
            action: selected.current,
            ready: false,
            failure:
              "Saved actions could not be checked. Reload them before starting another action.",
          });
          return;
        }
        const action = response.outcome.inner.actions.find(
          (entry) =>
            (identityKey === undefined || identifierHex(entry.ownerIdentity) === identityKey) &&
            groupActionMatches(entry.input, kind, scopeKey),
        );
        publish(action ?? null, action === undefined ? emptyFailure : actionFailure(action));
      } catch {
        if (current() && ticket === readTicket.current)
          setState({
            owner,
            action: selected.current,
            ready: false,
            failure:
              "Saved actions could not be checked. Reload them before starting another action.",
          });
      }
    },
    [available, current, identityKey, kind, listGroupActions, owner, publish, scopeKey],
  );

  useEffect(() => {
    selected.current = null;
    active.current = null;
    setPendingOwner(null);
    setState({ owner, action: null, ready: false, failure: null });
    void refresh();
  }, [owner, refresh]);

  const handle = (outcome: Bindings.GroupActionOutcome) => {
    if (outcome.tag === "Ready") {
      publish(outcome.inner.action);
      return outcome.inner.action;
    }
    const failure =
      outcome.tag === "StorageFull"
        ? "Group storage is full. Keep this action unchanged and retry when space is available."
        : outcome.tag === "Busy"
          ? "Group storage is busy. Retry this same action shortly."
          : outcome.inner.detail;
    setState({ owner, action: selected.current, ready: true, failure });
    return null;
  };

  const submit = async (makeInput: (id: Uint8Array) => Bindings.GroupActionInput) => {
    if (
      !available ||
      !canMutate ||
      !current() ||
      active.current !== null ||
      state.owner !== owner ||
      !state.ready
    )
      return;
    if (
      selected.current?.state.tag === "RetryExpired" ||
      selected.current?.state.tag === "Completed"
    )
      return;
    const ticket = {};
    active.current = ticket;
    readTicket.current += 1;
    setPendingOwner(owner);
    const valid = () => current() && active.current === ticket;
    try {
      let action = selected.current;
      if (action === null) {
        if (identity?.tag !== "Present") {
          setState({
            owner,
            action: null,
            ready: true,
            failure:
              "This device identity is not ready. Wait for it to load before starting an action.",
          });
          return;
        }
        const created = await createGroupRequestId();
        if (!valid()) return;
        if (created.type !== "outcome" || created.outcome.tag !== "Ready") {
          setState({
            owner,
            action: null,
            ready: true,
            failure: "This action could not be prepared. Try again.",
          });
          return;
        }
        action = {
          ownerIdentity: identity.inner.identityHash,
          input: makeInput(created.outcome.inner.clientRequestId),
          state: Bindings.GroupActionState.Pending.new(),
        };
        if (development.canPublish === false && !isLocalAction(action.input.tag)) return;
        publish(action, "Preparing this action…");
      }
      if (development.canPublish === false && !isLocalAction(action.input.tag)) return;
      const prepared = await prepareGroupAction({
        ownerIdentity: action.ownerIdentity,
        input: action.input,
      });
      if (!valid()) return;
      if (prepared.type !== "outcome") throw new Error("Prepare interrupted");
      if (prepared.outcome.tag === "InvalidInput") {
        // A validation failure did not admit this input. Re-read before unlocking:
        // it can also mean another UI already prepared an action for this scope.
        await refresh(prepared.outcome.inner.detail);
        return;
      }
      action = handle(prepared.outcome);
      if (action === null || action.state.tag !== "Pending") return;
      const response = await executeGroupAction({
        ownerIdentity: action.ownerIdentity,
        clientRequestId: action.input.inner.input.clientRequestId,
      });
      if (!valid()) return;
      if (response.type !== "outcome") throw new Error("Confirmation interrupted");
      handle(response.outcome);
    } catch {
      if (valid())
        setState({
          owner,
          action: selected.current,
          ready: true,
          failure:
            "Confirmation was interrupted. Retry this same action to avoid creating a duplicate.",
        });
    } finally {
      if (valid()) {
        active.current = null;
        setPendingOwner(null);
      }
    }
  };

  const clear = useCallback(
    async (reviewed: boolean) => {
      if (!current() || active.current !== null) return false;
      const action = selected.current;
      if (
        action === null ||
        (action.state.tag !== "Completed" && !(reviewed && action.state.tag === "RetryExpired"))
      )
        return false;
      const ticket = {};
      active.current = ticket;
      readTicket.current += 1;
      setPendingOwner(owner);
      try {
        const operation =
          action.state.tag === "RetryExpired" ? discardExpiredGroupAction : acknowledgeGroupAction;
        const response = await operation({
          ownerIdentity: action.ownerIdentity,
          clientRequestId: action.input.inner.input.clientRequestId,
        });
        if (!current() || active.current !== ticket) return false;
        if (response.type === "outcome" && response.outcome.tag === "Removed") {
          publish(null);
          return true;
        }
        setState({
          owner,
          action,
          ready: true,
          failure: "The saved result could not be cleared. Retry after reviewing it.",
        });
        return false;
      } catch {
        if (current())
          setState({
            owner,
            action,
            ready: true,
            failure: "The saved result could not be cleared. Retry after reviewing it.",
          });
        return false;
      } finally {
        if (current() && active.current === ticket) {
          active.current = null;
          setPendingOwner(null);
        }
      }
    },
    [acknowledgeGroupAction, current, discardExpiredGroupAction, owner, publish],
  );

  const valid = state.owner === owner;
  const action = valid ? state.action : null;
  return {
    submit,
    refresh,
    available,
    action,
    input: action?.input ?? null,
    result: action?.state.tag === "Completed" ? action.state.inner.result : null,
    pending: pendingOwner === owner,
    ready: valid && state.ready && canMutate,
    retrying: action !== null,
    retryExpired: action?.state.tag === "RetryExpired",
    failure: valid ? state.failure : null,
    acknowledge: useCallback(() => clear(false), [clear]),
    beginNewActionAfterReview: useCallback(() => clear(true), [clear]),
  };
}
