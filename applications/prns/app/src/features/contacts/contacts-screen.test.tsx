import type { DevelopmentNodeSnapshot, DevelopmentRuntime } from "@prns-internal/expo";
import * as Bindings from "@prns-internal/expo";
import { act, fireEvent, fireEventAsync, render, waitFor } from "@testing-library/react-native";
import { destinationHash, identityHash, interfaceId } from "personal-rns/contract";
import type { ReactNode } from "react";
import { Share } from "react-native";
import {
  AddContactScreen,
  ContactDetailScreen,
  ContactsScreen,
} from "@/features/contacts/contacts-screen";
import { ContactRuntimeProvider } from "@/native/contact-runtime-context";
import type { RuntimeCommandResult } from "@/native/development-runtime-context";

const mockReplace = jest.fn();
jest.mock("expo-router", () => ({
  Link: ({ children }: { readonly children: ReactNode }) => children,
  useRouter: () => ({ replace: mockReplace }),
}));
const destination = destinationHash(Uint8Array.from({ length: 16 }, (_, index) => index));
const identity = identityHash(Uint8Array.from({ length: 16 }, (_, index) => index + 16));
const mockProfile = { displayName: "prns", destination: new Uint8Array(16).fill(0xaa) };
const mockReadProfile = jest.fn<
  Promise<RuntimeCommandResult<Bindings.LocalMessagingProfileOutcome>>,
  []
>();
const mockSetName = jest.fn<
  Promise<RuntimeCommandResult<Bindings.LocalMessagingProfileOutcome>>,
  [string]
>();
const mockListPeers = jest.fn<Promise<RuntimeCommandResult<Bindings.LxmfPeerListOutcome>>, []>();
const mockAnnounce = jest.fn<Promise<RuntimeCommandResult<Bindings.AnnounceLxmfOutcome>>, []>();
const mockSaveDiscovered = jest.fn<
  Promise<RuntimeCommandResult<Bindings.ContactMutationOutcome>>,
  [Bindings.ContactDestinationInput]
>();
const mockClearDiscovery = jest.fn<
  Promise<RuntimeCommandResult<Bindings.LxmfDiscoveryClearOutcome>>,
  []
>();
const mockNodeRuntime = Bindings.DevelopmentNodeRuntime;
let mockRunning = false;
let mockRevision = 0n;
let mockFontScale = 1;
let mockWidth = 390;
let mockLocalHost: DevelopmentNodeSnapshot["localHost"] = snapshot().localHost;
let mockPhysicalPeers: DevelopmentNodeSnapshot["bluetooth"]["peers"] = [];
jest.mock("react-native/Libraries/Utilities/useWindowDimensions", () => ({
  __esModule: true,
  default: () => ({ width: mockWidth, height: 844, scale: 3, fontScale: mockFontScale }),
}));
const discovered: Bindings.LxmfPeerSummary = {
  destination,
  identity,
  displayName: "Announced Alice",
  requiredStampCost: undefined,
  sourceInterface: new Uint8Array(8).fill(0xbb),
  hops: 2,
  isPathResponse: false,
  lastObservedAgeMillis: 120_000n,
};
jest.mock("@/native/development-runtime-context", () => ({
  useDevelopmentRuntime: () => ({
    availability: { type: "available", platform: "ios" },
    phase: "ready",
    snapshot: {
      generationId: 1n,
      revision: mockRevision,
      localHost: mockLocalHost,
      bluetooth: { peers: mockPhysicalPeers },
      runtime: mockRunning ? mockNodeRuntime.Running : mockNodeRuntime.Stopped,
    },
    readMessagingProfile: mockReadProfile,
    setMessagingName: mockSetName,
    listLxmfPeers: mockListPeers,
    announceLxmf: mockAnnounce,
    saveDiscoveredContact: mockSaveDiscovered,
    clearLxmfDiscovery: mockClearDiscovery,
  }),
}));
beforeEach(() => {
  jest.clearAllMocks();
  mockRunning = false;
  mockRevision = 0n;
  mockFontScale = 1;
  mockWidth = 390;
  mockLocalHost = snapshot().localHost;
  mockPhysicalPeers = [];
  mockReadProfile.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.LocalMessagingProfileOutcome.Ready.new({ profile: mockProfile }),
  });
  mockSetName.mockImplementation(async (displayName) => ({
    type: "outcome",
    outcome: Bindings.LocalMessagingProfileOutcome.Ready.new({
      profile: { ...mockProfile, displayName: displayName.trim() },
    }),
  }));
  mockListPeers.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.LxmfPeerListOutcome.Listed.new({ peers: [discovered] }),
  });
  mockAnnounce.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.AnnounceLxmfOutcome.Requested,
  });
  mockSaveDiscovered.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.ContactMutationOutcome.Saved.new({
      contact: {
        destination,
        identity,
        alias: undefined,
        announcedName: "Announced Alice",
        isMessaging: true,
        pinned: false,
      },
    }),
  });
  mockClearDiscovery.mockResolvedValue({
    type: "outcome",
    outcome: Bindings.LxmfDiscoveryClearOutcome.Cleared,
  });
});
function snapshot(): DevelopmentNodeSnapshot {
  return {
    contractFingerprint: "test",
    network: {
      state: Bindings.LocalNetworkState.Stopped.new(),
      routes: [],
      announces: [],
      activityRevision: 0n,
      droppedAnnounceCount: 0n,
    },
    revision: 0n,
    runtime: Bindings.DevelopmentNodeRuntime.Stopped,
    primaryIdentity: Bindings.PrimaryIdentityState.Missing.new(),
    localHost: Bindings.LocalHostState.Stopped.new({
      lastStartFailure: undefined,
    }),
    bluetooth: {
      desiredEnabled: undefined,
      state: Bindings.LocalBluetoothState.Stopped.new(),
      peers: [],
    },
    lxmf: {
      state: Bindings.LxmfHealthState.Stopped,
      inboundOverflowCount: 0n,
      mailboxRevision: 0n,
      projectionRevision: 0n,
    },
    controllerIdentityFingerprint: undefined,
    pairing: Bindings.RemoteControlPairingState.Searching.new(),
    pairingCandidates: [],
    pairedTargets: [],
    lastAnnouncement: undefined,
    lastRemoteChange: undefined,
    generationId: 0n,
    activeOperation: undefined,
    failure: undefined,
  };
}
function fakeRuntime(overrides: Partial<DevelopmentRuntime> = {}): DevelopmentRuntime {
  return {
    waitLxmfChange: async () => undefined,
    listNamedIdentityActivity: async () =>
      Bindings.NamedIdentityActivityListOutcome.Ready.new({ activities: [] }),
    readIdentityActivity: async () =>
      Bindings.IdentityActivityOutcome.Ready.new({
        summary: {
          actionableInvitations: 0,
          unreadGroups: 0,
          unreadDirect: 0,
          conversations: [],
          partial: false,
        },
      }),
    markIdentityActivitySeen: async () => Bindings.IdentityActivitySeenOutcome.Saved.new(),
    listNamedIdentities: async () => Bindings.NamedIdentityCatalogOutcome.Busy.new(),
    createNamedIdentity: async () => Bindings.NamedIdentityCatalogOutcome.Busy.new(),
    changeNamedIdentity: async () => Bindings.NamedIdentityCatalogOutcome.Busy.new(),
    forIdentity: () => {
      throw new Error("No named identity fixture");
    },
    exportContactCard: async () => Bindings.ContactCardExportOutcome.Busy.new(),
    previewContactCard: async () => Bindings.ContactCardPreviewOutcome.UnsupportedService.new(),
    importContactCard: async () => Bindings.ContactCardImportOutcome.Busy.new(),
    listGroupActions: async () => Bindings.GroupActionListOutcome.Ready.new({ actions: [] }),
    prepareGroupAction: async () => Bindings.GroupActionOutcome.Busy.new(),
    executeGroupAction: async () => Bindings.GroupActionOutcome.Busy.new(),
    acknowledgeGroupAction: async () => Bindings.GroupActionAcknowledgeOutcome.Busy.new(),
    discardExpiredGroupAction: async () => Bindings.GroupActionAcknowledgeOutcome.Busy.new(),
    getGroupAnnounceSettings: async () =>
      Bindings.GroupAnnounceSettingsOutcome.Ready.new({ settings: { intervalMinutes: 60 } }),
    setGroupAnnounceSettings: async () => Bindings.GroupAnnounceSettingsOutcome.Busy.new(),
    announceGroups: async () => Bindings.GroupAnnounceOutcome.Busy.new(),
    createGroup: async () => Bindings.CreateGroupOutcome.Busy.new(),
    inviteToGroup: async () => Bindings.InviteToGroupOutcome.Busy.new(),
    listGroupInvitations: async () =>
      Bindings.GroupInvitationListOutcome.Ready.new({ invitations: [] }),
    respondGroupInvitation: async () => Bindings.RespondGroupInvitationOutcome.Busy.new(),
    listGroupConversations: async () => Bindings.GroupConversationListOutcome.NotProvisioned.new(),
    listGroupMessages: async () => Bindings.GroupMessageListOutcome.NotProvisioned.new(),
    createGroupRequestId: async () =>
      Bindings.GroupRequestIdOutcome.Ready.new({ clientRequestId: new Uint8Array(32) }),
    sendGroupText: async () => Bindings.GroupTextSendOutcome.NotProvisioned.new(),
    attachHost: async () => undefined,
    requestLocalNetworkPermission: async () => "notRequired",
    readConnectionSettings: async () => {
      throw new Error("No connection settings fixture");
    },
    updateConnectionSettings: async () => {
      throw new Error("No connection settings fixture");
    },
    readConnectionSnapshot: async () => {
      throw new Error("No connection snapshot fixture");
    },
    retryConnection: async () => {
      throw new Error("No connection snapshot fixture");
    },
    readBluetoothDiscoveryGroups: async () =>
      Bindings.LocalBluetoothDiscoveryGroupsOutcome.Busy.new(),
    setBluetoothDiscoveryGroups: async () =>
      Bindings.LocalBluetoothDiscoveryGroupsOutcome.Busy.new(),
    readMessagingProfile: async () =>
      Bindings.LocalMessagingProfileOutcome.Ready.new({ profile: mockProfile }),
    setMessagingName: async (displayName) =>
      Bindings.LocalMessagingProfileOutcome.Ready.new({ profile: { ...mockProfile, displayName } }),
    saveDiscoveredContact: async () => Bindings.ContactMutationOutcome.NotObserved.new(),
    clearLxmfDiscovery: async () => Bindings.LxmfDiscoveryClearOutcome.Cleared,
    readBluetoothSettings: async () =>
      Bindings.LocalBluetoothSettingsOutcome.Ready.new({ enabled: true }),
    setBluetoothEnabled: async (enabled: boolean) =>
      Bindings.LocalBluetoothSettingsOutcome.Ready.new({ enabled }),
    inspectDevelopmentIdentity: async () => Bindings.PrimaryIdentityState.Missing.new(),
    previewIdentityImport: async () => Bindings.IdentityImportPreviewOutcome.InvalidLength.new(),
    createGeneratedIdentity: async () => Bindings.IdentityCreationOutcome.AlreadyExists.new(),
    createImportedIdentity: async () => Bindings.IdentityCreationOutcome.AlreadyExists.new(),
    startDevelopmentNode: async () =>
      Bindings.DevelopmentNodeStartOutcome.Started.new({
        snapshot: snapshot(),
      }),
    readDevelopmentNodeSnapshot: async () => snapshot(),
    clearNetworkActivity: async () => Bindings.ClearNetworkActivityOutcome.LocalNodeStopped.new(),
    initiateRemoteControlPairing: async () =>
      Bindings.RemoteControlPairingCommandOutcome.Busy.new(),
    approveRemoteControlPairing: async () => Bindings.RemoteControlPairingCommandOutcome.Busy.new(),
    rejectRemoteControlPairing: async () => Bindings.RemoteControlPairingCommandOutcome.Busy.new(),
    describeRemoteControlTarget: async () => Bindings.RemoteControlDescribeOutcome.Busy.new(),
    announceRemoteControlTarget: async () => Bindings.RemoteControlAnnounceOutcome.Busy.new(),
    readRemoteNode: async () => Bindings.ReadRemoteNodeOutcome.Busy.new(),
    changeRemoteNode: async () => Bindings.ChangeRemoteNodeOutcome.Busy.new(),
    startRemoteWifiTrial: async () => Bindings.RemoteWifiCommandOutcome.Busy.new(),
    inspectRemoteWifiTrial: async () => Bindings.RemoteWifiCommandOutcome.Busy.new(),
    finishRemoteWifiTrial: async () => Bindings.RemoteWifiCommandOutcome.Busy.new(),
    saveObservedDestination: async () => Bindings.ContactMutationOutcome.NotObserved.new(),
    createManualContact: async () => Bindings.ContactMutationOutcome.NotFound.new(),
    setContactAlias: async () => Bindings.ContactMutationOutcome.NotFound.new(),
    setContactPinned: async () => Bindings.ContactMutationOutcome.NotFound.new(),
    deleteContact: async () => Bindings.ContactMutationOutcome.NotFound.new(),
    getContact: async () => Bindings.ContactLookupOutcome.NotFound.new(),
    listContacts: async () =>
      Bindings.ContactListOutcome.Listed.new({
        contacts: [],
      }),
    listLxmfPeers: async () =>
      Bindings.LxmfPeerListOutcome.Listed.new({
        peers: [],
      }),
    listLxmfConversations: async () => Bindings.LxmfMessageListOutcome.Listed.new({ messages: [] }),
    listLxmfMessages: async () =>
      Bindings.LxmfMessageListOutcome.Listed.new({
        messages: [],
      }),
    retryLxmfMessage: async () => Bindings.RetryLxmfMessageOutcome.NotFound.new(),
    cancelLxmfMessage: async () => Bindings.CancelLxmfMessageOutcome.NotFound.new(),
    announceLxmf: async () => Bindings.AnnounceLxmfOutcome.Requested,
    measureLxmfText: async () =>
      Bindings.MeasureLxmfTextOutcome.Measured.new({
        wireBytes: 113,
        remainingBytes: 318,
      }),
    sendDirectText: async () =>
      Bindings.SendDirectTextOutcome.Accepted.new({
        localRecordId: 1n,
      }),
    stopDevelopmentNode: async () => Bindings.DevelopmentNodeStopOutcome.AlreadyStopped.new(),
    resetDevelopmentData: async () => Bindings.DevelopmentNodeStopOutcome.AlreadyStopped.new(),
    ...overrides,
  };
}
function withRuntime(runtime: DevelopmentRuntime, screen: ReactNode) {
  return render(
    <ContactRuntimeProvider
      provider={{ availability: { type: "available", platform: "ios" }, runtime }}
    >
      {screen}
    </ContactRuntimeProvider>,
  );
}

test("a manual contact save may finish for its old owner without navigating the replacement journey", async () => {
  let finish!: (result: Bindings.ContactMutationOutcome) => void;
  const createManualContact = jest.fn(
    () =>
      new Promise<Bindings.ContactMutationOutcome>((resolve) => {
        finish = resolve;
      }),
  );
  const personal = fakeRuntime({ createManualContact });
  const workCreate = jest.fn(async () => Bindings.ContactMutationOutcome.NotFound.new());
  const work = fakeRuntime({ createManualContact: workCreate });
  const journey = (runtime: DevelopmentRuntime, owner: string) => (
    <ContactRuntimeProvider
      provider={{ availability: { type: "available", platform: "ios" }, runtime }}
    >
      <AddContactScreen key={owner} />
    </ContactRuntimeProvider>
  );
  const view = render(journey(personal, "personal"));
  fireEvent.changeText(view.getByLabelText("Destination"), "000102030405060708090a0b0c0d0e0f");
  fireEvent.press(view.getByRole("button", { name: "Save contact" }));
  expect(createManualContact).toHaveBeenCalledTimes(1);
  view.rerender(journey(work, "work"));
  await act(async () =>
    finish(
      Bindings.ContactMutationOutcome.Saved.new({
        contact: {
          destination,
          identity,
          alias: "Personal Alice",
          announcedName: undefined,
          isMessaging: true,
          pinned: false,
        },
      }),
    ),
  );
  expect(mockReplace).not.toHaveBeenCalled();
  expect(workCreate).not.toHaveBeenCalled();
  expect(view.getByLabelText("Destination").props.value).toBe("");
  expect(view.getByRole("button", { name: "Save contact" })).toBeEnabled();
});

test("a deleted old-owner contact cannot navigate away from the new owner's detail", async () => {
  let finish!: (result: Bindings.ContactMutationOutcome) => void;
  const contact = {
    destination,
    identity,
    alias: "Personal Alice",
    announcedName: undefined,
    isMessaging: true,
    pinned: false,
  };
  const deleteContact = jest.fn(
    () =>
      new Promise<Bindings.ContactMutationOutcome>((resolve) => {
        finish = resolve;
      }),
  );
  const personal = fakeRuntime({
    getContact: async () => Bindings.ContactLookupOutcome.Found.new({ contact }),
    deleteContact,
  });
  const work = fakeRuntime({
    getContact: async () =>
      Bindings.ContactLookupOutcome.Found.new({ contact: { ...contact, alias: "Work Alice" } }),
  });
  const journey = (runtime: DevelopmentRuntime, owner: string) => (
    <ContactRuntimeProvider
      provider={{ availability: { type: "available", platform: "ios" }, runtime }}
    >
      <ContactDetailScreen key={owner} destination={destination} />
    </ContactRuntimeProvider>
  );
  const view = render(journey(personal, "personal"));
  await view.findByText("Personal Alice");
  expect(view.queryByText("000102030405060708090a0b0c0d0e0f")).toBeNull();
  expect(view.queryByText("101112131415161718191a1b1c1d1e1f")).toBeNull();
  fireEvent.press(view.getByRole("button", { name: "Contact details" }));
  expect(view.getByText("000102030405060708090a0b0c0d0e0f")).toBeTruthy();
  expect(view.getByText("101112131415161718191a1b1c1d1e1f")).toBeTruthy();
  expect(deleteContact).not.toHaveBeenCalled();
  fireEvent.press(view.getByRole("button", { name: "Delete contact" }));
  expect(deleteContact).toHaveBeenCalledWith(destination);
  view.rerender(journey(work, "work"));
  await view.findByText("Work Alice");
  expect(view.queryByText("000102030405060708090a0b0c0d0e0f")).toBeNull();
  expect(view.getByRole("button", { name: "Contact details" })).toBeTruthy();
  await act(async () => finish(Bindings.ContactMutationOutcome.Deleted.new()));
  expect(mockReplace).not.toHaveBeenCalled();
  expect(view.getByText("Work Alice")).toBeTruthy();
});

test("a late alias update cannot replace a different captured contact runtime's local view", async () => {
  let finish!: (result: Bindings.ContactMutationOutcome) => void;
  const contact = {
    destination,
    identity,
    alias: "Personal Alice",
    announcedName: undefined,
    isMessaging: true,
    pinned: false,
  };
  const setContactAlias = jest.fn(
    () =>
      new Promise<Bindings.ContactMutationOutcome>((resolve) => {
        finish = resolve;
      }),
  );
  const personal = fakeRuntime({
    getContact: async () => Bindings.ContactLookupOutcome.Found.new({ contact }),
    setContactAlias,
  });
  const work = fakeRuntime({
    getContact: async () =>
      Bindings.ContactLookupOutcome.Found.new({ contact: { ...contact, alias: "Work Alice" } }),
  });
  const journey = (runtime: DevelopmentRuntime) => (
    <ContactRuntimeProvider
      provider={{ availability: { type: "available", platform: "ios" }, runtime }}
    >
      <ContactDetailScreen destination={destination} />
    </ContactRuntimeProvider>
  );
  const view = render(journey(personal));
  await view.findByText("Personal Alice");
  fireEvent.changeText(view.getByLabelText("Name (optional)"), "Personal updated");
  fireEvent.press(view.getByRole("button", { name: "Save name" }));
  view.rerender(journey(work));
  await view.findByText("Work Alice");
  await act(async () =>
    finish(
      Bindings.ContactMutationOutcome.Updated.new({
        contact: { ...contact, alias: "Personal updated" },
      }),
    ),
  );
  expect(view.getByText("Work Alice")).toBeTruthy();
  expect(view.getByLabelText("Name (optional)").props.value).toBe("Work Alice");
  expect(view.queryByText("Personal updated")).toBeNull();
  expect(mockReplace).not.toHaveBeenCalled();
});

describe("contact screens", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });
  it("renders persisted contacts returned by the direct native facade", async () => {
    const listContacts = jest.fn(async () =>
      Bindings.ContactListOutcome.Listed.new({
        contacts: [
          {
            destination,
            identity,
            alias: "Alice",
            announcedName: undefined,
            isMessaging: true,
            pinned: true,
          },
        ],
      }),
    );
    const view = withRuntime(fakeRuntime({ listContacts }), <ContactsScreen />);
    await waitFor(() => expect(view.getByText("Alice")).toBeTruthy());
    expect(view.queryByText("000102030405060708090a0b0c0d0e0f")).toBeNull();
    expect(view.getByText("Pinned")).toBeTruthy();
    expect(view.getByRole("link", { name: "Open contact Alice" })).toBeTruthy();
    expect(view.queryByText("Identity")).toBeNull();
    expect(view.queryByText("101112131415161718191a1b1c1d1e1f")).toBeNull();
    const labels = JSON.stringify(view.toJSON());
    expect(labels.indexOf("Add contact")).toBeLessThan(labels.indexOf("Alice"));
    expect(listContacts).toHaveBeenCalledTimes(1);
  });
  it("shows an unnamed contact's shortened destination once", async () => {
    const listContacts = jest.fn(async () =>
      Bindings.ContactListOutcome.Listed.new({
        contacts: [
          {
            destination,
            identity,
            alias: undefined,
            announcedName: undefined,
            isMessaging: false,
            pinned: false,
          },
        ],
      }),
    );
    const view = withRuntime(fakeRuntime({ listContacts }), <ContactsScreen />);
    expect(await view.findByText("00010203…0d0e0f")).toBeTruthy();
    expect(view.getAllByText("00010203…0d0e0f")).toHaveLength(1);
    expect(view.queryByText("000102030405060708090a0b0c0d0e0f")).toBeNull();
    expect(view.getByRole("button", { name: "Saved" }).props.accessibilityState.selected).toBe(
      true,
    );
    expect(view.getByRole("link", { name: "Open contact 00010203…0d0e0f" })).toBeTruthy();
  });
  it("distinguishes contacts with matching private or announced names", async () => {
    const otherDestination = destinationHash(new Uint8Array(16).fill(0xab));
    const listContacts = jest.fn(async () =>
      Bindings.ContactListOutcome.Listed.new({
        contacts: [
          {
            destination,
            identity,
            alias: "Alice",
            announcedName: undefined,
            isMessaging: true,
            pinned: false,
          },
          {
            destination: otherDestination,
            identity: undefined,
            alias: undefined,
            announcedName: "alice",
            isMessaging: true,
            pinned: false,
          },
        ],
      }),
    );
    const view = withRuntime(fakeRuntime({ listContacts }), <ContactsScreen />);
    await view.findByText("Alice");
    expect(view.getByText("00010203…0d0e0f")).toBeTruthy();
    expect(view.getByText("abababab…ababab")).toBeTruthy();
    expect(view.getByRole("link", { name: "Open contact Alice, 00010203…0d0e0f" })).toBeTruthy();
    expect(view.getByRole("link", { name: "Open contact alice, abababab…ababab" })).toBeTruthy();
  });
  it("does not expose native contact-list failure details", async () => {
    const listContacts = jest.fn(async () =>
      Bindings.ContactListOutcome.DevelopmentUnavailable.new({
        detail: "E290 upstream RemoteControl signed availability database owner stopped",
      }),
    );
    const view = withRuntime(fakeRuntime({ listContacts }), <ContactsScreen />);
    expect(await view.findByText("Contacts could not be loaded. Try again.")).toBeTruthy();
    expect(
      view.queryAllByText(/E290|signed availability|upstream RemoteControl|database owner/iu),
    ).toHaveLength(0);
    expect(listContacts).toHaveBeenCalledTimes(1);
  });
  it("passes explicit optional manual fields and routes by destination hash", async () => {
    const createManualContact = jest.fn(async () =>
      Bindings.ContactMutationOutcome.Saved.new({
        contact: {
          destination,
          identity: undefined,
          alias: "Alice",
          announcedName: undefined,
          isMessaging: true,
          pinned: false,
        },
      }),
    );
    const view = withRuntime(fakeRuntime({ createManualContact }), <AddContactScreen />);
    fireEvent.changeText(view.getByLabelText("Destination"), "000102030405060708090a0b0c0d0e0f");
    fireEvent.changeText(view.getByLabelText("Name (optional)"), " Alice ");
    fireEvent.press(view.getByRole("button", { name: "Save contact" }));
    await waitFor(() =>
      expect(createManualContact).toHaveBeenCalledWith(destination, undefined, " Alice "),
    );
    expect(mockReplace).toHaveBeenCalledWith({
      pathname: "/contacts/[destination]",
      params: { destination: "000102030405060708090a0b0c0d0e0f" },
    });
  });
  it("loads a destination-addressed detail and reports pin rejection honestly", async () => {
    const getContact = jest.fn(async () =>
      Bindings.ContactLookupOutcome.Found.new({
        contact: {
          destination,
          identity: undefined,
          alias: "Manual",
          announcedName: undefined,
          isMessaging: false,
          pinned: false,
        },
      }),
    );
    const setContactPinned = jest.fn(async () =>
      Bindings.ContactMutationOutcome.MissingIdentity.new(),
    );
    const view = withRuntime(
      fakeRuntime({ getContact, setContactPinned }),
      <ContactDetailScreen destination={destination} />,
    );
    await waitFor(() => expect(view.getByText("Manual")).toBeTruthy());
    fireEvent.press(view.getByRole("button", { name: "Pin contact" }));
    await waitFor(() =>
      expect(
        view.getByText("Add or discover an identity before pinning this contact."),
      ).toBeTruthy(),
    );
    expect(setContactPinned).toHaveBeenCalledWith(destination, true);
  });
  it("does not simulate contacts on an unsupported platform", () => {
    const view = render(
      <ContactRuntimeProvider
        provider={{
          availability: { type: "unavailable", platform: "web", reason: "notImplemented" },
        }}
      >
        <ContactsScreen />
      </ContactRuntimeProvider>,
    );
    expect(view.getByText("Contacts unavailable")).toBeTruthy();
    expect(view.getByText("Contacts are not available on web yet.")).toBeTruthy();
  });
});

test.each([
  [
    Bindings.NativeStoragePreparationOutcome.DevelopmentResetRequired.new({
      reason: "private database layout detail",
    }),
    "App reset required",
  ],
  [
    Bindings.NativeStoragePreparationOutcome.Unavailable.new({
      detail: "private database lock detail",
    }),
    "Storage unavailable",
  ],
] as const)("preserves bootstrap recovery and retry guidance %#", async (outcome, title) => {
  let ready = false;
  const listContacts = jest.fn(async () => {
    if (!ready) throw new Bindings.NativeStoragePreparationError(outcome);
    return Bindings.ContactListOutcome.Listed.new({ contacts: [] });
  });
  const view = withRuntime(fakeRuntime({ listContacts }), <ContactsScreen />);
  expect(await view.findByText(title)).toBeTruthy();
  if (outcome.tag === "DevelopmentResetRequired")
    expect(view.getByText("Open recovery")).toBeTruthy();
  expect(view.queryByText(/private database/)).toBeNull();
  expect(view.queryByText("Loading saved contacts…")).toBeNull();
  ready = true;
  fireEvent.press(view.getByRole("button", { name: "Refresh contacts" }));
  expect(await view.findByText("No saved contacts")).toBeTruthy();
  expect(view.queryByText(title)).toBeNull();
});

describe("messaging discovery and local profile", () => {
  test("keeps the profile editable while stopped and never announces on load or save", async () => {
    const view = withRuntime(fakeRuntime(), <ContactsScreen />);
    expect(await view.findByText("prns")).toBeTruthy();
    expect(view.queryByLabelText("Messaging name")).toBeNull();
    expect(view.getByRole("button", { name: "Announce yourself" })).toBeDisabled();
    fireEvent.press(view.getByRole("button", { name: "Edit messaging name" }));
    fireEvent.changeText(view.getByLabelText("Messaging name"), "  Trail friend  ");
    fireEvent.press(view.getByRole("button", { name: "Save messaging name" }));
    expect(await view.findByText("Trail friend")).toBeTruthy();
    expect(mockSetName).toHaveBeenCalledWith("  Trail friend  ");
    expect(view.queryByLabelText("Messaging name")).toBeNull();
    expect(mockAnnounce).not.toHaveBeenCalled();
    expect(mockListPeers).not.toHaveBeenCalled();
  });

  test("shares the native LXMF address separately from network announcing", async () => {
    const share = jest.spyOn(Share, "share").mockResolvedValue({ action: Share.sharedAction });
    const view = withRuntime(fakeRuntime(), <ContactsScreen />);
    await view.findByText("prns");
    fireEvent.press(view.getByRole("button", { name: "My address" }));
    expect(view.getByText("aa".repeat(16))).toBeTruthy();
    fireEvent.press(view.getByRole("button", { name: "Share address" }));
    await waitFor(() => expect(share).toHaveBeenCalledWith({ message: "aa".repeat(16) }));
    expect(mockAnnounce).not.toHaveBeenCalled();
    share.mockRestore();
  });

  test.each([
    [
      Bindings.AnnounceLxmfOutcome.Requested,
      "Announcement requested. Other devices may hear it on connected networks.",
    ],
    [
      Bindings.AnnounceLxmfOutcome.NoUsableConnection,
      "No usable connection. Open Connections to connect, then announce again.",
    ],
    [Bindings.AnnounceLxmfOutcome.Busy, "Another messaging action is in progress. Try again."],
  ])("announces only on explicit action with honest outcome %#", async (outcome, expected) => {
    mockRunning = true;
    mockAnnounce.mockResolvedValue({ type: "outcome", outcome });
    const runtime = fakeRuntime();
    const view = withRuntime(runtime, <ContactsScreen />);
    await view.findByText("prns");
    mockRevision += 1n;
    view.rerender(
      <ContactRuntimeProvider
        provider={{ availability: { type: "available", platform: "ios" }, runtime }}
      >
        <ContactsScreen />
      </ContactRuntimeProvider>,
    );
    expect(mockAnnounce).not.toHaveBeenCalled();
    fireEvent.press(view.getByRole("button", { name: "Announce yourself" }));
    expect(await view.findByText(expected)).toBeTruthy();
    expect(mockAnnounce).toHaveBeenCalledTimes(1);
    expect(view.queryByText("Messaging address shared.")).toBeNull();
  });

  test("shows discovery age and saves through the authenticated native command", async () => {
    mockRunning = true;
    let saved = false;
    const runtime = fakeRuntime({
      listContacts: async () =>
        Bindings.ContactListOutcome.Listed.new({
          contacts: saved
            ? [
                {
                  destination,
                  identity,
                  alias: "Private Alice",
                  announcedName: "Announced Alice",
                  isMessaging: true,
                  pinned: true,
                },
              ]
            : [],
        }),
    });
    mockSaveDiscovered.mockImplementation(async () => {
      saved = true;
      return {
        type: "outcome",
        outcome: Bindings.ContactMutationOutcome.Saved.new({
          contact: {
            destination,
            identity,
            alias: "Private Alice",
            announcedName: "Announced Alice",
            isMessaging: true,
            pinned: true,
          },
        }),
      };
    });
    const view = withRuntime(runtime, <ContactsScreen />);
    await view.findByText("prns");
    fireEvent.press(view.getByRole("button", { name: "Discovered" }));
    expect(await view.findByText("Announced Alice")).toBeTruthy();
    expect(view.getByText(/Heard 2 minutes ago/)).toBeTruthy();
    fireEvent.press(view.getByRole("button", { name: "Save contact Announced Alice" }));
    expect(await view.findByText("Private Alice")).toBeTruthy();
    expect(mockSaveDiscovered).toHaveBeenCalledWith({ destination });
    expect(view.getByRole("link", { name: "Message Private Alice" })).toBeTruthy();
    expect(view.getByText("Saved contact")).toBeTruthy();
    fireEvent.press(view.getByRole("button", { name: "Saved" }));
    expect(view.getByText("Private Alice")).toBeTruthy();
    expect(view.getByText("Pinned")).toBeTruthy();
  });

  test("upgrades a manually saved address through authenticated discovery without replacing its private name", async () => {
    mockRunning = true;
    let contact: Bindings.Contact = {
      destination,
      identity: undefined,
      alias: "Private Alice",
      announcedName: undefined,
      isMessaging: false,
      pinned: false,
    };
    const runtime = fakeRuntime({
      listContacts: async () => Bindings.ContactListOutcome.Listed.new({ contacts: [contact] }),
      getContact: async () => Bindings.ContactLookupOutcome.Found.new({ contact }),
    });
    mockSaveDiscovered.mockImplementation(async () => {
      contact = {
        ...contact,
        identity,
        announcedName: discovered.displayName,
        isMessaging: true,
      };
      return {
        type: "outcome",
        outcome: Bindings.ContactMutationOutcome.Updated.new({ contact }),
      };
    });
    const view = withRuntime(runtime, <ContactsScreen />);
    await view.findByText("Private Alice");
    fireEvent.press(view.getByRole("button", { name: "Discovered" }));
    expect(view.getByRole("link", { name: "Open contact" })).toBeTruthy();
    // Flush both the save command and its contact-list refresh before checking the new UI.
    await fireEventAsync.press(
      await view.findByRole("button", { name: "Save messaging contact Private Alice" }),
    );
    await waitFor(() =>
      expect(
        view.queryByRole("button", { name: "Save messaging contact Private Alice" }),
      ).toBeNull(),
    );
    expect(mockSaveDiscovered).toHaveBeenCalledWith({ destination });
    expect(contact.alias).toBe("Private Alice");
    expect(contact.pinned).toBe(false);
    expect(view.getByRole("link", { name: "Message Private Alice" })).toBeTruthy();
    view.unmount();

    const detail = withRuntime(runtime, <ContactDetailScreen destination={destination} />);
    expect(await detail.findByRole("link", { name: "Message" })).toBeTruthy();
    expect(detail.getByDisplayValue("Private Alice")).toBeTruthy();
  });

  test("preserves identity conflict feedback and leaves a discovered contact unsaved", async () => {
    mockRunning = true;
    mockSaveDiscovered.mockResolvedValue({
      type: "outcome",
      outcome: Bindings.ContactMutationOutcome.IdentityConflict.new({
        existing: identity,
        attempted: new Uint8Array(16).fill(0xee),
      }),
    });
    const view = withRuntime(fakeRuntime(), <ContactsScreen />);
    await view.findByText("prns");
    fireEvent.press(view.getByRole("button", { name: "Discovered" }));
    fireEvent.press(await view.findByRole("button", { name: "Save contact Announced Alice" }));
    expect(
      await view.findByText("The saved identity differs from the verified network identity."),
    ).toBeTruthy();
    expect(view.queryByText("Contact saved.")).toBeNull();
  });

  test("clears only discovery, with saved contacts still available", async () => {
    mockRunning = true;
    const runtime = fakeRuntime({
      listContacts: async () =>
        Bindings.ContactListOutcome.Listed.new({
          contacts: [
            {
              destination,
              identity,
              alias: "Private Alice",
              announcedName: undefined,
              isMessaging: true,
              pinned: false,
            },
          ],
        }),
    });
    const view = withRuntime(runtime, <ContactsScreen />);
    await view.findByText("Private Alice");
    fireEvent.press(view.getByRole("button", { name: "Discovered" }));
    await view.findByText("Saved contact");
    mockListPeers.mockResolvedValue({
      type: "outcome",
      outcome: Bindings.LxmfPeerListOutcome.Listed.new({ peers: [] }),
    });
    fireEvent.press(view.getByRole("button", { name: "Clear discovered contacts" }));
    expect(await view.findByText("No discovered contacts")).toBeTruthy();
    expect(mockClearDiscovery).toHaveBeenCalledTimes(1);
    fireEvent.press(view.getByRole("button", { name: "Saved" }));
    expect(view.getByText("Private Alice")).toBeTruthy();
  });

  test("does not display a delayed discovery result from a stopped generation", async () => {
    mockRunning = true;
    let finish: ((result: RuntimeCommandResult<Bindings.LxmfPeerListOutcome>) => void) | undefined;
    mockListPeers.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const runtime = fakeRuntime();
    const view = withRuntime(runtime, <ContactsScreen />);
    await view.findByText("prns");
    fireEvent.press(view.getByRole("button", { name: "Discovered" }));
    mockRunning = false;
    view.rerender(
      <ContactRuntimeProvider
        provider={{ availability: { type: "available", platform: "ios" }, runtime }}
      >
        <ContactsScreen />
      </ContactRuntimeProvider>,
    );
    await act(async () =>
      finish?.({
        type: "outcome",
        outcome: Bindings.LxmfPeerListOutcome.Listed.new({ peers: [discovered] }),
      }),
    );
    expect(view.queryByText("Announced Alice")).toBeNull();
  });

  test.each([
    [
      Bindings.LocalMessagingProfileOutcome.InvalidInput.new({
        detail: "private validation detail",
      }),
      "Choose a shorter name without line breaks or special control characters.",
    ],
    [
      Bindings.LocalMessagingProfileOutcome.Unavailable.new({ detail: "private disk detail" }),
      "The messaging name could not be confirmed. Refresh to check it, then try again.",
    ],
    [
      Bindings.LocalMessagingProfileOutcome.SavedButNotApplied.new({
        profile: { ...mockProfile, displayName: "New name" },
        detail: "private runtime detail",
      }),
      "Name saved. It could not be applied to the running node yet. Try saving it again before announcing.",
    ],
  ])(
    "handles profile validation and uncertain application honestly %#",
    async (outcome, expected) => {
      mockSetName.mockResolvedValue({ type: "outcome", outcome });
      const view = withRuntime(fakeRuntime(), <ContactsScreen />);
      await view.findByText("prns");
      fireEvent.press(view.getByRole("button", { name: "Edit messaging name" }));
      fireEvent.changeText(view.getByLabelText("Messaging name"), "New name");
      fireEvent.press(view.getByRole("button", { name: "Save messaging name" }));
      expect(await view.findByText(expected)).toBeTruthy();
      expect(view.queryByText(/private .* detail/)).toBeNull();
      expect(mockAnnounce).not.toHaveBeenCalled();
    },
  );

  test("offers Message only for a native-recognized saved messaging destination", async () => {
    const runtime = fakeRuntime({
      getContact: async () =>
        Bindings.ContactLookupOutcome.Found.new({
          contact: {
            destination,
            identity,
            alias: undefined,
            announcedName: "Remembered Alice",
            isMessaging: true,
            pinned: false,
          },
        }),
    });
    const view = withRuntime(runtime, <ContactDetailScreen destination={destination} />);
    expect(await view.findByText("Remembered Alice")).toBeTruthy();
    expect(view.getByRole("link", { name: "Message" })).toBeTruthy();
    view.unmount();
    const other = withRuntime(
      fakeRuntime({
        getContact: async () =>
          Bindings.ContactLookupOutcome.Found.new({
            contact: {
              destination,
              identity,
              alias: "Board",
              announcedName: undefined,
              isMessaging: false,
              pinned: false,
            },
          }),
      }),
      <ContactDetailScreen destination={destination} />,
    );
    await other.findByText("Board");
    expect(other.queryByRole("link", { name: "Message" })).toBeNull();
  });
});

test.each([1.5, 2])(
  "keeps profile editing and exact discovery provenance accessible at %sx text",
  async (fontScale) => {
    mockRunning = true;
    mockFontScale = fontScale;
    mockWidth = 320;
    const view = withRuntime(fakeRuntime(), <ContactsScreen />);
    await view.findByText("prns");
    fireEvent.press(view.getByRole("button", { name: "Edit messaging name" }));
    fireEvent.changeText(view.getByLabelText("Messaging name"), "An accessible messaging name");
    fireEvent.press(view.getByRole("button", { name: "Save messaging name" }));
    expect(await view.findByText("An accessible messaging name")).toBeTruthy();
    fireEvent.press(view.getByRole("button", { name: "Discovered" }));
    expect(await view.findByText("Announced Alice")).toBeTruthy();
    expect(view.queryByText("Connection ID")).toBeNull();
    fireEvent.press(
      view.getByRole("button", { name: "Show discovery details for Announced Alice" }),
    );
    expect(view.getByText("Hop count")).toBeTruthy();
    expect(view.getByText("2")).toBeTruthy();
    expect(view.getByText("bb".repeat(8))).toBeTruthy();
    expect(view.getByText("101112131415161718191a1b1c1d1e1f")).toBeTruthy();
    expect(view.getByText("Announce")).toBeTruthy();
    expect(view.getByText("Connection no longer listed")).toBeTruthy();
    expect(view.queryByText("Automatic Bluetooth")).toBeNull();
    expect(view.getByRole("link", { name: "Message Announced Alice" })).toBeTruthy();
    expect(view.getByRole("button", { name: "Save contact Announced Alice" })).toBeEnabled();
  },
);

test("prevents announcing the previous name while a profile save is pending", async () => {
  mockRunning = true;
  let finish:
    | ((result: RuntimeCommandResult<Bindings.LocalMessagingProfileOutcome>) => void)
    | undefined;
  mockSetName.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = withRuntime(fakeRuntime(), <ContactsScreen />);
  await view.findByText("prns");
  fireEvent.press(view.getByRole("button", { name: "Edit messaging name" }));
  fireEvent.changeText(view.getByLabelText("Messaging name"), "New name");
  fireEvent.press(view.getByRole("button", { name: "Save messaging name" }));
  expect(view.getByRole("button", { name: "Announce yourself" })).toBeDisabled();
  fireEvent.press(view.getByRole("button", { name: "Announce yourself" }));
  expect(mockAnnounce).not.toHaveBeenCalled();
  await act(async () =>
    finish?.({
      type: "outcome",
      outcome: Bindings.LocalMessagingProfileOutcome.Ready.new({
        profile: { ...mockProfile, displayName: "New name" },
      }),
    }),
  );
  expect(view.getByRole("button", { name: "Announce yourself" })).toBeEnabled();
  fireEvent.press(view.getByRole("button", { name: "Announce yourself" }));
  await waitFor(() => expect(mockAnnounce).toHaveBeenCalledTimes(1));
});

test("resolves recorded Bluetooth ingress by exact physical peer ID despite a folded host inventory", async () => {
  mockRunning = true;
  mockPhysicalPeers = [
    {
      interfaceId: discovered.sourceInterface,
      name: "Connected peer",
      connected: true,
      rxBytes: 0n,
      txBytes: 0n,
      details: undefined,
      rssiDbm: undefined,
    },
  ];
  mockLocalHost = Bindings.LocalHostState.Running.new({
    host: {
      revision: 1n,
      backend: {
        backend: "Native",
        capabilities: ["Bluetooth"],
        interfaceKinds: ["AutomaticBluetoothLe"],
      },
      interfaces: [
        {
          interfaceId: interfaceId(new Uint8Array(8).fill(0xcc)),
          name: "Bluetooth supervisor",
          kind: "AutomaticBluetoothLe",
          health: "Connected",
          rxBytes: 0n,
          txBytes: 0n,
          routeCount: 0,
          linkCount: 0,
          transportedLinkCount: 0,
        },
      ],
      routes: [],
      activeLinkCount: 0,
      destinationIdentities: [],
      runtime: {
        running: true,
        uptimeMillis: 1,
        interfaceCount: 1,
        onlineInterfaceCount: 1,
        routeCount: 0,
        linkCount: 0,
        transportedLinkCount: 0,
        rxBytes: 0n,
        txBytes: 0n,
        rxBps: 0,
        txBps: 0,
      },
      persistence: { persistent: true, restored: false },
    },
  });
  const view = withRuntime(fakeRuntime(), <ContactsScreen />);
  await view.findByText("prns");
  fireEvent.press(view.getByRole("button", { name: "Discovered" }));
  fireEvent.press(
    await view.findByRole("button", { name: "Show discovery details for Announced Alice" }),
  );
  expect(view.getByText("Connected peer")).toBeTruthy();
  expect(view.getByText("bb".repeat(8))).toBeTruthy();
  expect(view.queryByText("Connection no longer listed")).toBeNull();
  expect(view.queryByText("Bluetooth supervisor")).toBeNull();
  expect(view.getByText(/not the route a future message will take/)).toBeTruthy();
});
