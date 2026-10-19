import type * as Bindings from "@prns-internal/native-bindings";
import {
  bindingModule,
  HOST_CONTRACT_FINGERPRINT,
  NATIVE_CONTRACT_FINGERPRINT,
} from "@prns-internal/native-bindings";
import type * as NativeBindings from "@prns-internal/native-bindings/native";
import { type FfiConverter, UniffiInternalError } from "@ubjs/core";
import type { PrnsAppNativeModule } from "./native";

export type * from "@prns-internal/native-bindings";

export class NativeContractMismatchError extends Error {
  constructor(
    readonly expected: string,
    readonly actual: string,
  ) {
    super(`Native contract mismatch: expected ${expected}, received ${actual}`);
    this.name = "NativeContractMismatchError";
  }
}

/** Preparation errors retain the generated outcome for recovery decisions. */
export class NativeStoragePreparationError extends Error {
  constructor(readonly outcome: Bindings.NativeStoragePreparationOutcome) {
    super(`Native storage preparation: ${outcome.tag}`);
    this.name = "NativeStoragePreparationError";
  }
}

type IdentityApi = Pick<
  Bindings.NamedIdentityClient,
  | "readMessagingProfile"
  | "setMessagingName"
  | "saveObservedDestination"
  | "createManualContact"
  | "setContactAlias"
  | "setContactPinned"
  | "deleteContact"
  | "getContact"
  | "listContacts"
  | "listGroupActions"
  | "prepareGroupAction"
  | "executeGroupAction"
  | "acknowledgeGroupAction"
  | "discardExpiredGroupAction"
  | "getGroupAnnounceSettings"
  | "setGroupAnnounceSettings"
  | "announceGroups"
  | "createGroup"
  | "inviteToGroup"
  | "listGroupInvitations"
  | "respondGroupInvitation"
  | "listGroupConversations"
  | "listGroupMessages"
  | "sendGroupText"
  | "listLxmfPeers"
  | "saveDiscoveredContact"
  | "clearLxmfDiscovery"
  | "listLxmfMessages"
  | "listLxmfConversations"
  | "retryLxmfMessage"
  | "cancelLxmfMessage"
  | "announceLxmf"
  | "measureLxmfText"
  | "sendDirectText"
>;

type BindingLoader = () => Promise<typeof NativeBindings>;
const loadNativeBindings: BindingLoader = () => import("@prns-internal/native-bindings/native");
const codecs = bindingModule.converters;
const asyncOptions = (signal: AbortSignal | undefined) =>
  signal === undefined ? undefined : { signal };

// RN's AbortSignal polyfill has .aborted and listeners, but no throwIfAborted.
function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted === true) throw new UniffiInternalError.AbortError();
}

/**
 * Domain names and platform admission around the generated functions. Values
 * pass through unchanged; the native supervisor owns the process and storage.
 */
export function createDevelopmentRuntime(
  getNativeModule: () => PrnsAppNativeModule,
  loadBindings: BindingLoader = loadNativeBindings,
) {
  let verifiedBindings: Promise<typeof NativeBindings> | undefined;
  let storagePreparation: Promise<void> | undefined;

  const bindings = async (signal?: AbortSignal) => {
    throwIfAborted(signal);
    // Require the platform capability before installing the native player.
    getNativeModule();
    if (verifiedBindings === undefined) {
      verifiedBindings = loadBindings()
        .then((api) => {
          const contract = api.bindingContract();
          for (const [expected, actual] of [
            [NATIVE_CONTRACT_FINGERPRINT, contract.app],
            [HOST_CONTRACT_FINGERPRINT, contract.host],
          ] as const) {
            if (expected !== actual) throw new NativeContractMismatchError(expected, actual);
          }
          return api;
        })
        .catch((cause: unknown) => {
          verifiedBindings = undefined;
          throw cause;
        });
    }
    const api = await verifiedBindings;
    throwIfAborted(signal);
    return api;
  };
  const decode = async <T>(
    operation: (native: PrnsAppNativeModule) => Promise<number[]>,
    codec: FfiConverter<Uint8Array, T>,
  ): Promise<T> => {
    await bindings();
    return codec.lift(Uint8Array.from(await operation(getNativeModule())));
  };
  const prepareStorage = async () => {
    if (storagePreparation === undefined) {
      storagePreparation = decode(
        (native) => native.prepareStorage(),
        codecs.FfiConverterTypeNativeStoragePreparationOutcome,
      )
        .then((outcome) => {
          if (outcome.tag !== "Prepared") throw new NativeStoragePreparationError(outcome);
        })
        .catch((cause: unknown) => {
          storagePreparation = undefined;
          throw cause;
        });
    }
    await storagePreparation;
  };
  const call = async <T>(
    operation: (api: typeof Bindings) => Promise<T> | T,
    signal?: AbortSignal,
    preparation?: "storage" | "outbound" | "storageAndOutbound",
  ): Promise<T> => {
    const api = await bindings(signal);
    if (preparation === "storage" || preparation === "storageAndOutbound") {
      await prepareStorage();
      throwIfAborted(signal);
    }
    if (preparation === "outbound" || preparation === "storageAndOutbound") {
      await getNativeModule().prepareOutbound();
    }
    throwIfAborted(signal);
    return operation(api);
  };

  // Each client owns one immutable catalog reference. Neither opening a client
  // nor invoking a method consults the UI's current default identity.
  const identityRuntime = (reference?: Bindings.NamedIdentityReference) => {
    let client: Promise<Bindings.NamedIdentityClientLike> | undefined;
    const namedCall = <T>(operation: (api: Bindings.NamedIdentityClientLike) => Promise<T>) =>
      call(
        async (api) => {
          if (reference === undefined)
            throw new Error("Activity requires an explicit named identity.");
          client ??= Promise.resolve(api.openNamedIdentity(reference));
          return operation(await client);
        },
        undefined,
        "storage",
      );
    const domainCall = <T>(
      operation: (api: IdentityApi) => Promise<T> | T,
      signal?: AbortSignal,
      preparation?: "storage" | "outbound" | "storageAndOutbound",
    ): Promise<T> =>
      call(
        async (api) => {
          if (reference === undefined) return operation(api);
          client ??= Promise.resolve(api.openNamedIdentity(reference));
          return operation(await client);
        },
        signal,
        preparation,
      );
    return {
      readIdentityActivity: () => namedCall((api) => api.readIdentityActivity()),
      markIdentityActivitySeen: (input: Bindings.MarkIdentityActivitySeenInput) =>
        namedCall((api) => api.markIdentityActivitySeen(input)),
      readMessagingProfile: () =>
        domainCall((api) => api.readMessagingProfile(), undefined, "storage"),
      setMessagingName: (name: string) =>
        domainCall((api) => api.setMessagingName(name), undefined, "storage"),
      saveObservedDestination: (destination: Uint8Array, signal?: AbortSignal) =>
        domainCall(
          (api) => api.saveObservedDestination({ destination }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      createManualContact: (
        destination: Uint8Array,
        identity?: Uint8Array,
        alias?: string,
        signal?: AbortSignal,
      ) =>
        domainCall(
          (api) => api.createManualContact({ destination, identity, alias }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      setContactAlias: (destination: Uint8Array, alias?: string, signal?: AbortSignal) =>
        domainCall(
          (api) => api.setContactAlias({ destination, alias }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      setContactPinned: (destination: Uint8Array, pinned: boolean, signal?: AbortSignal) =>
        domainCall(
          (api) => api.setContactPinned({ destination, pinned }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      deleteContact: (destination: Uint8Array, signal?: AbortSignal) =>
        domainCall(
          (api) => api.deleteContact({ destination }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      getContact: (destination: Uint8Array, signal?: AbortSignal) =>
        domainCall(
          (api) => api.getContact({ destination }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      listContacts: (signal?: AbortSignal) =>
        domainCall((api) => api.listContacts(asyncOptions(signal)), signal, "storage"),
      listGroupActions: (signal?: AbortSignal) =>
        domainCall((api) => api.listGroupActions(asyncOptions(signal)), signal, "storage"),
      prepareGroupAction: (input: Bindings.PrepareGroupActionInput, signal?: AbortSignal) =>
        domainCall((api) => api.prepareGroupAction(input, asyncOptions(signal)), signal, "storage"),
      executeGroupAction: (reference: Bindings.GroupActionReference, signal?: AbortSignal) =>
        domainCall(
          (api) => api.executeGroupAction(reference, asyncOptions(signal)),
          signal,
          "storage",
        ),
      acknowledgeGroupAction: (reference: Bindings.GroupActionReference, signal?: AbortSignal) =>
        domainCall(
          (api) => api.acknowledgeGroupAction(reference, asyncOptions(signal)),
          signal,
          "storage",
        ),
      discardExpiredGroupAction: (reference: Bindings.GroupActionReference, signal?: AbortSignal) =>
        domainCall(
          (api) => api.discardExpiredGroupAction(reference, asyncOptions(signal)),
          signal,
          "storage",
        ),
      getGroupAnnounceSettings: (signal?: AbortSignal) =>
        domainCall((api) => api.getGroupAnnounceSettings(asyncOptions(signal)), signal, "storage"),
      setGroupAnnounceSettings: (input: Bindings.GroupAnnounceSettings, signal?: AbortSignal) =>
        domainCall(
          (api) => api.setGroupAnnounceSettings(input, asyncOptions(signal)),
          signal,
          "storage",
        ),
      announceGroups: (signal?: AbortSignal) =>
        domainCall((api) => api.announceGroups(asyncOptions(signal)), signal, "outbound"),
      createGroupRequestId: (signal?: AbortSignal) =>
        call((api) => api.createGroupRequestId(asyncOptions(signal)), signal),
      createGroup: (input: Bindings.CreateGroupInput, signal?: AbortSignal) =>
        domainCall((api) => api.createGroup(input, asyncOptions(signal)), signal, "storage"),
      inviteToGroup: (input: Bindings.InviteToGroupInput, signal?: AbortSignal) =>
        domainCall((api) => api.inviteToGroup(input, asyncOptions(signal)), signal, "storage"),
      listGroupInvitations: (signal?: AbortSignal) =>
        domainCall((api) => api.listGroupInvitations(asyncOptions(signal)), signal, "storage"),
      respondGroupInvitation: (input: Bindings.RespondGroupInvitationInput, signal?: AbortSignal) =>
        domainCall(
          (api) => api.respondGroupInvitation(input, asyncOptions(signal)),
          signal,
          "storage",
        ),
      listGroupConversations: (signal?: AbortSignal) =>
        domainCall((api) => api.listGroupConversations(asyncOptions(signal)), signal, "storage"),
      listGroupMessages: (input: Bindings.ListGroupMessagesInput, signal?: AbortSignal) =>
        domainCall((api) => api.listGroupMessages(input, asyncOptions(signal)), signal, "storage"),
      sendGroupText: (input: Bindings.SendGroupTextInput, signal?: AbortSignal) =>
        domainCall((api) => api.sendGroupText(input, asyncOptions(signal)), signal, "storage"),
      listLxmfPeers: (signal?: AbortSignal) =>
        domainCall((api) => api.listLxmfPeers(asyncOptions(signal)), signal),
      saveDiscoveredContact: (input: Bindings.ContactDestinationInput, signal?: AbortSignal) =>
        domainCall(
          (api) => api.saveDiscoveredContact(input, asyncOptions(signal)),
          signal,
          "storage",
        ),
      clearLxmfDiscovery: (signal?: AbortSignal) =>
        domainCall((api) => api.clearLxmfDiscovery(asyncOptions(signal)), signal),
      listLxmfMessages: (input: Bindings.ListLxmfMessagesInput, signal?: AbortSignal) =>
        domainCall((api) => api.listLxmfMessages(input, asyncOptions(signal)), signal, "storage"),
      listLxmfConversations: (input: Bindings.ListLxmfConversationsInput, signal?: AbortSignal) =>
        domainCall(
          (api) => api.listLxmfConversations(input, asyncOptions(signal)),
          signal,
          "storage",
        ),
      retryLxmfMessage: (localRecordId: bigint, signal?: AbortSignal) =>
        domainCall(
          (api) => api.retryLxmfMessage({ localRecordId }, asyncOptions(signal)),
          signal,
          "storageAndOutbound",
        ),
      cancelLxmfMessage: (localRecordId: bigint, signal?: AbortSignal) =>
        domainCall(
          (api) => api.cancelLxmfMessage({ localRecordId }, asyncOptions(signal)),
          signal,
          "storage",
        ),
      announceLxmf: (signal?: AbortSignal) =>
        domainCall((api) => api.announceLxmf(asyncOptions(signal)), signal, "outbound"),
      measureLxmfText: (input: Bindings.MeasureLxmfTextInput, signal?: AbortSignal) =>
        domainCall((api) => api.measureLxmfText(input, asyncOptions(signal)), signal),
      sendDirectText: (input: Bindings.SendDirectTextInput, signal?: AbortSignal) =>
        domainCall((api) => api.sendDirectText(input, asyncOptions(signal)), signal, "outbound"),
    };
  };

  return {
    ...identityRuntime(),
    forIdentity: (reference: Bindings.NamedIdentityReference) =>
      identityRuntime({
        incarnation: Uint8Array.from(reference.incarnation),
        sequence: reference.sequence,
        publicHash: Uint8Array.from(reference.publicHash),
      }),
    listNamedIdentities: () => call((api) => api.listNamedIdentities(), undefined, "storage"),
    listNamedIdentityActivity: () =>
      call((api) => api.listNamedIdentityActivity(), undefined, "storage"),
    createNamedIdentity: (input: Bindings.CreateNamedIdentityInput) =>
      call((api) => api.createNamedIdentity(input), undefined, "storage"),
    changeNamedIdentity: (input: Bindings.ChangeNamedIdentityInput) =>
      call((api) => api.changeNamedIdentity(input), undefined, "storage"),
    inspectDevelopmentIdentity: () =>
      decode((native) => native.inspectIdentity(), codecs.FfiConverterTypePrimaryIdentityState),
    previewIdentityImport: (identity: Uint8Array) =>
      call((api) => api.previewIdentityImport(identity)),
    createGeneratedIdentity: () =>
      decode(
        (native) => native.createGeneratedIdentity(),
        codecs.FfiConverterTypeIdentityCreationOutcome,
      ),
    createImportedIdentity: (identity: Uint8Array) =>
      decode(
        (native) => native.createImportedIdentity(Array.from(identity)),
        codecs.FfiConverterTypeIdentityCreationOutcome,
      ),
    startDevelopmentNode: async (input: Bindings.DevelopmentNodeStartInput) => {
      await bindings();
      const bytes = codecs.FfiConverterTypeDevelopmentNodeStartInput.lower(
        input,
        (size) => new Uint8Array(size),
      );
      return decode(
        (native) => native.start(Array.from(bytes)),
        codecs.FfiConverterTypeDevelopmentNodeStartOutcome,
      );
    },
    readDevelopmentNodeSnapshot: (signal?: AbortSignal) =>
      call((api) => api.readSnapshot(asyncOptions(signal)), signal),
    clearNetworkActivity: (input: Bindings.ClearNetworkActivityInput, signal?: AbortSignal) =>
      call((api) => api.clearNetworkActivity(input, asyncOptions(signal)), signal),
    readBluetoothSettings: () => call((api) => api.readBluetoothSettings(), undefined, "storage"),
    setBluetoothEnabled: (enabled: boolean) =>
      call((api) => api.setBluetoothEnabled(enabled), undefined, "storage"),
    attachHost: async () => {
      const api = await bindings();
      const host = api.sharedHost();
      if (host === undefined) return undefined;
      return api.borrowHost(host);
    },
    initiateRemoteControlPairing: (
      input: Bindings.InitiateRemoteControlPairingInput,
      signal?: AbortSignal,
    ) => call((api) => api.initiatePairing(input, asyncOptions(signal)), signal, "outbound"),
    approveRemoteControlPairing: (
      input: Bindings.RemoteControlPairingDecisionInput,
      signal?: AbortSignal,
    ) => call((api) => api.approvePairing(input, asyncOptions(signal)), signal),
    rejectRemoteControlPairing: (
      input: Bindings.RemoteControlPairingDecisionInput,
      signal?: AbortSignal,
    ) => call((api) => api.rejectPairing(input, asyncOptions(signal)), signal),
    describeRemoteControlTarget: (
      input: Bindings.DescribeRemoteControlTargetInput,
      signal?: AbortSignal,
    ) => call((api) => api.describeTarget(input, asyncOptions(signal)), signal, "outbound"),
    announceRemoteControlTarget: (
      input: Bindings.AnnounceRemoteControlTargetInput,
      signal?: AbortSignal,
    ) => call((api) => api.announceTarget(input, asyncOptions(signal)), signal, "outbound"),
    readRemoteNode: (input: Bindings.ReadRemoteNodeInput, signal?: AbortSignal) =>
      call((api) => api.readRemoteNode(input, asyncOptions(signal)), signal, "outbound"),
    changeRemoteNode: (input: Bindings.ChangeRemoteNodeInput, signal?: AbortSignal) =>
      call((api) => api.changeRemoteNode(input, asyncOptions(signal)), signal, "outbound"),
    startRemoteWifiTrial: (input: Bindings.StartRemoteWifiTrialInput, signal?: AbortSignal) =>
      call((api) => api.startRemoteWifiTrial(input, asyncOptions(signal)), signal, "outbound"),
    inspectRemoteWifiTrial: (input: Bindings.InspectRemoteWifiTrialInput, signal?: AbortSignal) =>
      call((api) => api.inspectRemoteWifiTrial(input, asyncOptions(signal)), signal, "outbound"),
    finishRemoteWifiTrial: (input: Bindings.FinishRemoteWifiTrialInput, signal?: AbortSignal) =>
      call((api) => api.finishRemoteWifiTrial(input, asyncOptions(signal)), signal, "outbound"),
    stopDevelopmentNode: () =>
      decode((native) => native.stop(), codecs.FfiConverterTypeDevelopmentNodeStopOutcome),
    resetDevelopmentData: async () => {
      const outcome = await decode(
        (native) => native.reset(),
        codecs.FfiConverterTypeDevelopmentNodeStopOutcome,
      );
      storagePreparation = undefined;
      return outcome;
    },
  };
}

export type DevelopmentRuntime = ReturnType<typeof createDevelopmentRuntime>;
