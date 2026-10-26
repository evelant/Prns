import * as Bindings from "@prns-internal/expo";
import { identifierHex } from "@/features/groups/format";
import type { RuntimeCommandResult } from "@/native/development-runtime-context";

type Operations = {
  create?: (
    input: Bindings.CreateGroupInput,
  ) => Promise<RuntimeCommandResult<Bindings.CreateGroupOutcome>>;
  invite?: (
    input: Bindings.InviteToGroupInput,
  ) => Promise<RuntimeCommandResult<Bindings.InviteToGroupOutcome>>;
  respond?: (
    input: Bindings.RespondGroupInvitationInput,
  ) => Promise<RuntimeCommandResult<Bindings.RespondGroupInvitationOutcome>>;
  send?: (
    input: Bindings.SendGroupTextInput,
  ) => Promise<RuntimeCommandResult<Bindings.GroupTextSendOutcome>>;
};

/** Durable native API double: survives React unmounts; clear explicitly between tests. */
export function createGroupActionTestStore(operations: Operations) {
  const actions = new Map<string, Bindings.GroupAction>();
  const ready = (
    action: Bindings.GroupAction,
  ): RuntimeCommandResult<Bindings.GroupActionOutcome> => ({
    type: "outcome",
    outcome: Bindings.GroupActionOutcome.Ready.new({ action }),
  });
  const listGroupActions = jest.fn(
    async (): Promise<RuntimeCommandResult<Bindings.GroupActionListOutcome>> => ({
      type: "outcome",
      outcome: Bindings.GroupActionListOutcome.Ready.new({ actions: [...actions.values()] }),
    }),
  );
  const prepareGroupAction = jest.fn(async (prepared: Bindings.PrepareGroupActionInput) => {
    const input = prepared.input;
    const id = identifierHex(input.inner.input.clientRequestId);
    const action = actions.get(id) ?? {
      ownerIdentity: prepared.ownerIdentity,
      input,
      state: Bindings.GroupActionState.Pending.new(),
    };
    actions.set(id, action);
    return ready(action);
  });
  const executeGroupAction = jest.fn(
    async (
      reference: Bindings.GroupActionReference,
    ): Promise<RuntimeCommandResult<Bindings.GroupActionOutcome>> => {
      const key = identifierHex(reference.clientRequestId);
      const action = actions.get(key);
      if (action === undefined) throw new Error("Unprepared action");
      if (action.state.tag !== "Pending") return ready(action);
      const input = action.input;
      const response =
        input.tag === "Create"
          ? await operations.create?.(input.inner.input)
          : input.tag === "Invite"
            ? await operations.invite?.(input.inner.input)
            : input.tag === "Respond"
              ? await operations.respond?.(input.inner.input)
              : await operations.send?.(input.inner.input);
      if (response === undefined) throw new Error("Operation missing");
      if (response.type !== "outcome") return response;
      const outcome = response.outcome;
      if (outcome.tag === "Busy" || outcome.tag === "StorageFull")
        return { type: "outcome", outcome: Bindings.GroupActionOutcome[outcome.tag].new() };
      if (outcome.tag === "Unavailable")
        return {
          type: "outcome",
          outcome: Bindings.GroupActionOutcome.Unavailable.new(outcome.inner),
        };
      let state: Bindings.GroupActionState;
      if (outcome.tag === "RetryExpired") state = Bindings.GroupActionState.RetryExpired.new();
      else {
        const result =
          outcome.tag === "Accepted"
            ? "groupId" in outcome.inner
              ? Bindings.GroupActionResult.Created.new({ groupId: outcome.inner.groupId })
              : Bindings.GroupActionResult.Sent.new({ messageId: outcome.inner.messageId })
            : outcome.tag === "Queued"
              ? Bindings.GroupActionResult.Invited.new({ invitationId: outcome.inner.invitationId })
              : outcome.tag === "Saved"
                ? Bindings.GroupActionResult.Responded.new()
                : Bindings.GroupActionResult.Rejected.new({
                    reason:
                      outcome.tag === "InvalidInput"
                        ? outcome.inner.reason
                        : Bindings.GroupRejectionReason.Unknown,
                    detail:
                      outcome.tag === "InvalidInput" ? outcome.inner.detail : "Unavailable group",
                  });
        state = Bindings.GroupActionState.Completed.new({ result });
      }
      const completed = { ...action, state };
      actions.set(key, completed);
      return ready(completed);
    },
  );
  const acknowledgeGroupAction = jest.fn(
    async (
      reference: Bindings.GroupActionReference,
    ): Promise<RuntimeCommandResult<Bindings.GroupActionAcknowledgeOutcome>> => {
      const clientRequestId = reference.clientRequestId;
      actions.delete(identifierHex(clientRequestId));
      return { type: "outcome", outcome: Bindings.GroupActionAcknowledgeOutcome.Removed.new() };
    },
  );
  const discardExpiredGroupAction = jest.fn(async (reference: Bindings.GroupActionReference) =>
    acknowledgeGroupAction(reference),
  );
  return {
    actions,
    listGroupActions,
    prepareGroupAction,
    executeGroupAction,
    acknowledgeGroupAction,
    discardExpiredGroupAction,
  };
}
