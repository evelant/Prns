import { MAX_CONTACT_CARD_URI_LENGTH } from "../features/contacts/contact-card-links.ts";

export type ScaffoldAvailability = "implementedScaffold" | "notYetImplemented";
export type NavigationSection = "installation" | "inbox" | "contacts" | "nodes" | "more";

type ScalarParam = {
  readonly kind: "scalar";
  readonly name: string;
  readonly required: boolean;
  readonly format?: "destinationHash" | "groupId" | "invitationId" | "contactCardDraft";
};

type EnumParam = {
  readonly kind: "enum";
  readonly name: string;
  readonly required: boolean;
  readonly values: readonly string[];
};

export type RouteParamRule = ScalarParam | EnumParam;

export type ScreenCatalogEntry = {
  readonly id: string;
  readonly path: string;
  readonly routeFiles: readonly string[];
  readonly label: string;
  readonly section: NavigationSection;
  readonly phonePlacement: string;
  readonly widePlacement: string;
  readonly deepLink: "external-navigation" | "app-issued" | "none";
  readonly availability: ScaffoldAvailability;
  readonly summary: string;
  readonly limitation: string;
  readonly backPath: string;
  readonly params: readonly RouteParamRule[];
  readonly phoneRootOrder?: number;
  readonly wideRootOrder?: number;
};

const noParams: readonly RouteParamRule[] = [];

export const screenCatalog = [
  {
    id: "installation.onboarding",
    path: "/onboarding/:step?",
    routeFiles: ["app/onboarding/index.tsx", "app/onboarding/[step].tsx"],
    label: "Onboarding",
    section: "installation",
    phonePlacement: "Full-screen gate",
    widePlacement: "Full-window gate",
    deepLink: "none",
    availability: "implementedScaffold",
    summary: "Set up the identity this device will use on Reticulum.",
    limitation: "Identity export, backup, and recovery are not available in this preview.",
    backPath: "/onboarding/welcome",
    params: [
      {
        kind: "enum",
        name: "step",
        required: false,
        values: [
          "welcome",
          "create",
          "import",
          "retention",
          "provision",
          "interfaces",
          "pairing",
          "complete",
        ],
      },
    ],
  },
  {
    id: "installation.setup",
    path: "/setup",
    routeFiles: ["app/(shell)/setup.tsx"],
    label: "Connection setup",
    section: "installation",
    phonePlacement: "Guided setup",
    widePlacement: "Guided setup",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Connect nearby and choose how you appear to contacts.",
    limitation: "Nearby connections need Bluetooth access on iOS or Android.",
    backPath: "/inbox",
    params: noParams,
  },
  {
    id: "installation.recovery",
    path: "/recovery",
    routeFiles: ["app/recovery.tsx"],
    label: "Recovery",
    section: "installation",
    phonePlacement: "Full-screen gate",
    widePlacement: "Full-window gate",
    deepLink: "none",
    availability: "implementedScaffold",
    summary: "Check your identity or reset this app's data.",
    limitation: "Identity repair, export, and backup are not available in this preview.",
    backPath: "/onboarding/welcome",
    params: [{ kind: "scalar", name: "installationId", required: false }],
  },
  {
    id: "inbox.index",
    path: "/inbox",
    routeFiles: ["app/(shell)/inbox/index.tsx"],
    label: "Inbox",
    section: "inbox",
    phonePlacement: "Inbox tab root",
    widePlacement: "Inbox rail root",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Read and manage messages saved on this device.",
    limitation: "Messaging is available on iOS and Android. Background delivery is not guaranteed.",
    backPath: "/inbox",
    params: noParams,
    phoneRootOrder: 0,
    wideRootOrder: 0,
  },
  {
    id: "inbox.conversation",
    path: "/inbox/conversation/:destination",
    routeFiles: ["app/(shell)/inbox/conversation/[destination].tsx"],
    label: "Conversation",
    section: "inbox",
    phonePlacement: "Push from Inbox",
    widePlacement: "Inbox detail pane",
    deepLink: "app-issued",
    availability: "implementedScaffold",
    summary: "Read messages with one contact and retry or cancel queued messages.",
    limitation: "Failed messages must be retried manually.",
    backPath: "/inbox",
    params: [{ kind: "scalar", name: "destination", required: true, format: "destinationHash" }],
  },
  {
    id: "inbox.group",
    path: "/inbox/group/:groupId",
    routeFiles: ["app/(shell)/inbox/group/[groupId].tsx"],
    label: "Group conversation",
    section: "inbox",
    phonePlacement: "Push from Inbox",
    widePlacement: "Inbox detail pane",
    deepLink: "app-issued",
    availability: "implementedScaffold",
    summary: "Send group text, invite devices, and view device receipts.",
    limitation: "Test messages only. Saved group data is not yet protected on the phone.",
    backPath: "/inbox",
    params: [{ kind: "scalar", name: "groupId", required: true, format: "groupId" }],
  },
  {
    id: "inbox.group-new",
    path: "/inbox/group/new",
    routeFiles: ["app/(shell)/inbox/group/new.tsx"],
    label: "New group",
    section: "inbox",
    phonePlacement: "Push from Inbox",
    widePlacement: "Inbox detail pane",
    deepLink: "app-issued",
    availability: "implementedScaffold",
    summary: "Create a group on this device and invite other devices.",
    limitation: "Group creation is available on iOS and Android.",
    backPath: "/inbox",
    params: noParams,
  },
  {
    id: "inbox.group-invite",
    path: "/inbox/group/:groupId/invite",
    routeFiles: ["app/(shell)/inbox/group/[groupId]/invite.tsx"],
    label: "Invite a device",
    section: "inbox",
    phonePlacement: "Push from group conversation",
    widePlacement: "Inbox detail pane",
    deepLink: "app-issued",
    availability: "implementedScaffold",
    summary: "Invite a device using its chosen messaging identity.",
    limitation: "Only the owner can invite devices. Earlier history is not shared.",
    backPath: "/inbox",
    params: [{ kind: "scalar", name: "groupId", required: true, format: "groupId" }],
  },
  {
    id: "inbox.invitation",
    path: "/inbox/invitation/:invitationId",
    routeFiles: ["app/(shell)/inbox/invitation/[invitationId].tsx"],
    label: "Group invitation",
    section: "inbox",
    phonePlacement: "Push from Inbox",
    widePlacement: "Inbox detail pane",
    deepLink: "app-issued",
    availability: "implementedScaffold",
    summary: "Review an authenticated invitation and accept or decline it.",
    limitation: "Joining waits for the owner and invited device to connect.",
    backPath: "/inbox",
    params: [{ kind: "scalar", name: "invitationId", required: true, format: "invitationId" }],
  },
  {
    id: "inbox.message",
    path: "/inbox/message/:messageId",
    routeFiles: ["app/(shell)/inbox/message/[messageId].tsx"],
    label: "Message",
    section: "inbox",
    phonePlacement: "Push or sheet from conversation",
    widePlacement: "Inspector detail pane",
    deepLink: "app-issued",
    availability: "notYetImplemented",
    summary: "View message details and delivery status.",
    limitation: "Message details are not available yet.",
    backPath: "/inbox",
    params: [{ kind: "scalar", name: "messageId", required: true }],
  },
  {
    id: "inbox.compose",
    path: "/inbox/compose",
    routeFiles: ["app/(shell)/inbox/compose.tsx"],
    label: "Compose",
    section: "inbox",
    phonePlacement: "Full-screen composer",
    widePlacement: "Composer detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Choose a saved or discovered contact and write a direct message.",
    limitation: "Large messages are not supported yet.",
    backPath: "/inbox",
    params: [{ kind: "scalar", name: "destination", required: false, format: "destinationHash" }],
  },
  {
    id: "contacts.index",
    path: "/contacts",
    routeFiles: ["app/(shell)/contacts/index.tsx"],
    label: "Contacts",
    section: "contacts",
    phonePlacement: "Contacts tab root",
    widePlacement: "Contacts rail root",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Browse saved and discovered messaging contacts, and announce yourself.",
    limitation: "Contacts are available on iOS and Android.",
    backPath: "/contacts",
    params: [
      {
        kind: "enum",
        name: "section",
        required: false,
        values: ["saved", "discovered", "profile"],
      },
    ],
    phoneRootOrder: 1,
    wideRootOrder: 1,
  },
  {
    id: "contacts.entry",
    path: "/contacts/:destination",
    routeFiles: ["app/(shell)/contacts/[destination].tsx"],
    label: "Contact",
    section: "contacts",
    phonePlacement: "Push from Contacts",
    widePlacement: "Contacts detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "View or edit a saved contact.",
    limitation: "Saved contacts do not update automatically from the network.",
    backPath: "/contacts",
    params: [{ kind: "scalar", name: "destination", required: true, format: "destinationHash" }],
  },
  {
    id: "contacts.add",
    path: "/contacts/add",
    routeFiles: ["app/(shell)/contacts/add.tsx"],
    label: "Add contact",
    section: "contacts",
    phonePlacement: "Modal or full screen",
    widePlacement: "Dialog or detail pane",
    deepLink: "none",
    availability: "implementedScaffold",
    summary: "Save a destination with an optional identity and name.",
    limitation: "Signed cards can also be scanned or pasted from Contacts.",
    backPath: "/contacts",
    params: noParams,
  },
  {
    id: "contacts.card",
    path: "/contacts/card",
    routeFiles: ["app/(shell)/contacts/card.tsx"],
    label: "My contact card",
    section: "contacts",
    phonePlacement: "Push from Contacts",
    widePlacement: "Contacts detail pane",
    deepLink: "app-issued",
    availability: "implementedScaffold",
    summary: "Share your signed public contact card as a QR code or portable link.",
    limitation: "Contact cards bind an identity to a service; confirm who shared a card with you.",
    backPath: "/contacts",
    params: noParams,
  },
  {
    id: "contacts.import",
    path: "/contacts/import",
    routeFiles: ["app/(shell)/contacts/import.tsx"],
    label: "Add a contact card",
    section: "contacts",
    phonePlacement: "Push from Contacts",
    widePlacement: "Contacts detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Scan or paste a signed contact card and review it before saving.",
    limitation: "Saving a contact never grants group membership.",
    backPath: "/contacts",
    params: [
      { kind: "scalar", name: "card", required: false, format: "contactCardDraft" },
      { kind: "enum", name: "invalid", required: false, values: ["0", "1"] },
    ],
  },
  {
    id: "contacts.merge",
    path: "/contacts/merge",
    routeFiles: ["app/(shell)/contacts/merge.tsx"],
    label: "Merge contacts",
    section: "contacts",
    phonePlacement: "Full-screen review",
    widePlacement: "Dialog",
    deepLink: "none",
    availability: "notYetImplemented",
    summary: "Combine two saved contacts.",
    limitation: "Contact merging is not available yet.",
    backPath: "/contacts",
    params: [
      { kind: "scalar", name: "sourceId", required: true },
      { kind: "scalar", name: "targetId", required: true },
    ],
  },
  {
    id: "nodes.index",
    path: "/nodes",
    routeFiles: ["app/(shell)/nodes/index.tsx"],
    label: "Nodes",
    section: "nodes",
    phonePlacement: "Nodes tab root",
    widePlacement: "Nodes rail root",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Manage this device and its paired nodes.",
    limitation: "Node management is available on iOS and Android.",
    backPath: "/nodes",
    params: noParams,
    phoneRootOrder: 2,
    wideRootOrder: 2,
  },
  {
    id: "nodes.local",
    path: "/nodes/local",
    routeFiles: ["app/(shell)/nodes/local/index.tsx"],
    label: "This device",
    section: "nodes",
    phonePlacement: "Push from Nodes",
    widePlacement: "Nodes detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "View detailed network status for this device.",
    limitation: "Detailed node status is available on iOS and Android.",
    backPath: "/nodes",
    params: noParams,
  },
  {
    id: "nodes.managed",
    path: "/nodes/managed/:nodeId",
    routeFiles: ["app/(shell)/nodes/managed/[nodeId].tsx"],
    label: "Manage node",
    section: "nodes",
    phonePlacement: "Push from Nodes",
    widePlacement: "Nodes detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "View a paired node, check its connection, and share its address.",
    limitation:
      "Connection checks and node address sharing are available when the node allows them.",
    backPath: "/nodes",
    params: [{ kind: "scalar", name: "nodeId", required: true }],
  },
  {
    id: "nodes.pair",
    path: "/nodes/pair",
    routeFiles: ["app/(shell)/nodes/pair.tsx"],
    label: "Pair a node",
    section: "nodes",
    phonePlacement: "Full-screen flow",
    widePlacement: "Dialog or detail pane",
    deepLink: "none",
    availability: "implementedScaffold",
    summary: "Pair with a nearby node.",
    limitation: "Pairing is available on iOS and Android with Bluetooth and a nearby node.",
    backPath: "/nodes",
    params: [{ kind: "scalar", name: "candidateId", required: false }],
  },
  {
    id: "nodes.grants",
    path: "/nodes/local/grants",
    routeFiles: ["app/(shell)/nodes/local/grants.tsx"],
    label: "Remote access",
    section: "nodes",
    phonePlacement: "Push from local node",
    widePlacement: "Inspector detail pane",
    deepLink: "none",
    availability: "notYetImplemented",
    summary: "Review who can manage this device.",
    limitation: "Remote-access controls are not available yet.",
    backPath: "/nodes/local",
    params: noParams,
  },
  {
    id: "more.index",
    path: "/more",
    routeFiles: ["app/(shell)/more/index.tsx"],
    label: "More",
    section: "more",
    phonePlacement: "More tab root",
    widePlacement: "More and settings group",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Open identities, interfaces, network inspection, settings, and help.",
    limitation: "Messaging and node connections require the iOS or Android app.",
    backPath: "/more",
    params: noParams,
    phoneRootOrder: 3,
    wideRootOrder: 3,
  },
  {
    id: "identities.index",
    path: "/more/identities",
    routeFiles: ["app/(shell)/more/identities/index.tsx"],
    label: "Identities",
    section: "more",
    phonePlacement: "More stack",
    widePlacement: "Identities rail and detail",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Create and choose identities with separate contacts and conversations.",
    limitation: "Pausing preserves local history and pending deliveries.",
    backPath: "/more",
    params: noParams,
  },
  {
    id: "identities.detail",
    path: "/more/identities/:identityId",
    routeFiles: ["app/(shell)/more/identities/[identityId].tsx"],
    label: "Identity",
    section: "more",
    phonePlacement: "Push",
    widePlacement: "Identities detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Rename, pause or resume an identity and browse its saved history.",
    limitation: "Identity names are private labels. Managed nodes are shared by the installation.",
    backPath: "/more/identities",
    params: [{ kind: "scalar", name: "identityId", required: true }],
  },
  {
    id: "interfaces.index",
    path: "/more/interfaces",
    routeFiles: ["app/(shell)/more/interfaces/index.tsx"],
    label: "Interfaces",
    section: "more",
    phonePlacement: "More stack",
    widePlacement: "Interfaces rail root",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Manage Bluetooth, local network discovery and saved TCP peers.",
    limitation:
      "Native connections require iOS or Android. Available LAN discovery methods depend on the installed build.",
    backPath: "/more",
    params: [{ kind: "enum", name: "section", required: false, values: ["bluetooth", "ip"] }],
    wideRootOrder: 4,
  },
  {
    id: "interfaces.detail",
    path: "/more/interfaces/:interfaceId",
    routeFiles: ["app/(shell)/more/interfaces/[interfaceId].tsx"],
    label: "Interface",
    section: "more",
    phonePlacement: "Push",
    widePlacement: "Interfaces detail pane",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "View a network connection.",
    limitation: "Saved TCP peer details require the native app.",
    backPath: "/more/interfaces",
    params: [{ kind: "scalar", name: "interfaceId", required: true }],
  },
  {
    id: "interfaces.edit",
    path: "/more/interfaces/edit",
    routeFiles: ["app/(shell)/more/interfaces/edit.tsx"],
    label: "Edit interface",
    section: "more",
    phonePlacement: "Full-screen form",
    widePlacement: "Dialog or detail pane",
    deepLink: "none",
    availability: "implementedScaffold",
    summary: "Change a network connection's settings.",
    limitation: "TCP peer settings require the native app.",
    backPath: "/more/interfaces",
    params: [{ kind: "scalar", name: "interfaceId", required: false }],
  },
  {
    id: "notifications.settings",
    path: "/more/notifications",
    routeFiles: ["app/(shell)/more/notifications.tsx"],
    label: "Notifications",
    section: "more",
    phonePlacement: "More stack",
    widePlacement: "Settings detail",
    deepLink: "none",
    availability: "notYetImplemented",
    summary: "Choose when prns sends notifications.",
    limitation: "Notifications and background message delivery are not available yet.",
    backPath: "/more",
    params: noParams,
  },
  {
    id: "activity.index",
    path: "/more/activity",
    routeFiles: ["app/(shell)/more/activity/index.tsx"],
    label: "Network",
    section: "more",
    phonePlacement: "More stack",
    widePlacement: "Network rail root",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Inspect connections, known routes, and heard announcements.",
    limitation:
      "Announcements are limited to this node session; historical message paths are not recorded.",
    backPath: "/more",
    params: [
      {
        kind: "enum",
        name: "filter",
        required: false,
        values: ["connections", "routes", "announcements"],
      },
    ],
    wideRootOrder: 5,
  },
  {
    id: "activity.operation",
    path: "/more/activity/operation/:operationId",
    routeFiles: ["app/(shell)/more/activity/operation/[operationId].tsx"],
    label: "Operation",
    section: "more",
    phonePlacement: "Push or sheet",
    widePlacement: "Inspector pane",
    deepLink: "app-issued",
    availability: "notYetImplemented",
    summary: "View details for a recent activity.",
    limitation: "Activity details are not available yet.",
    backPath: "/more/activity",
    params: [{ kind: "scalar", name: "operationId", required: true }],
  },
  {
    id: "settings.index",
    path: "/more/settings",
    routeFiles: ["app/(shell)/more/settings/index.tsx"],
    label: "Settings",
    section: "more",
    phonePlacement: "More stack",
    widePlacement: "Settings rail root",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "View app information and manage this app's data.",
    limitation: "These settings affect only this app.",
    backPath: "/more",
    params: noParams,
  },
  {
    id: "settings.storage",
    path: "/more/settings/storage",
    routeFiles: ["app/(shell)/more/settings/storage.tsx"],
    label: "Storage",
    section: "more",
    phonePlacement: "Settings stack",
    widePlacement: "Settings detail",
    deepLink: "none",
    availability: "notYetImplemented",
    summary: "See how much space prns uses on this device.",
    limitation: "Storage details are not available yet.",
    backPath: "/more/settings",
    params: noParams,
  },
  {
    id: "settings.recovery",
    path: "/more/settings/recovery",
    routeFiles: ["app/(shell)/more/settings/recovery.tsx"],
    label: "Reset and recovery",
    section: "more",
    phonePlacement: "Settings stack",
    widePlacement: "Settings detail",
    deepLink: "none",
    availability: "notYetImplemented",
    summary: "Reset or recover app data.",
    limitation: "Recovery without resetting is not available yet.",
    backPath: "/more/settings",
    params: [
      {
        kind: "enum",
        name: "resetScope",
        required: false,
        values: ["preferences", "service-data", "identity", "installation"],
      },
    ],
  },
  {
    id: "settings.about",
    path: "/more/settings/about",
    routeFiles: ["app/(shell)/more/settings/about.tsx"],
    label: "About",
    section: "more",
    phonePlacement: "Settings stack",
    widePlacement: "Settings detail",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "View app and version information.",
    limitation: "A build label identifies a package; the API contract is not a source revision.",
    backPath: "/more/settings",
    params: noParams,
  },
  {
    id: "help.index",
    path: "/more/help",
    routeFiles: ["app/(shell)/more/help/index.tsx"],
    label: "Help",
    section: "more",
    phonePlacement: "More stack",
    widePlacement: "Help detail",
    deepLink: "external-navigation",
    availability: "implementedScaffold",
    summary: "Find setup, messaging, identity and node help.",
    limitation: "The maintainer alpha uses test data and foreground operation.",
    backPath: "/more",
    params: noParams,
  },
] as const satisfies readonly ScreenCatalogEntry[];

export type ScreenId = (typeof screenCatalog)[number]["id"];
export type RawRouteParams = Readonly<Record<string, string | string[] | undefined>>;

export function screenById(id: ScreenId): (typeof screenCatalog)[number] {
  const screen = screenCatalog.find((candidate) => candidate.id === id);
  if (screen === undefined) {
    throw new Error(`screen catalog entry is missing: ${id}`);
  }
  return screen;
}

function isNonEmptySingle(value: string | string[] | undefined): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function valueMatches(rule: RouteParamRule, value: string): boolean {
  if (rule.kind === "scalar") {
    if (rule.format === "groupId" || rule.format === "invitationId")
      return /^[0-9a-fA-F]{64}$/u.test(value);
    return rule.format === "destinationHash" ? /^[0-9a-fA-F]{32}$/u.test(value) : true;
  }
  return rule.values.includes(value);
}

export function routeParamsAreValid(entry: ScreenCatalogEntry, params: RawRouteParams): boolean {
  const allowed = new Set(entry.params.map((rule) => rule.name));
  if (entry.path === "/setup" || /^\/(contacts|inbox)(?:\/|$)/.test(entry.path)) {
    allowed.add("owner");
    const owner = params.owner;
    if (
      owner !== undefined &&
      (typeof owner !== "string" || !/^[a-f0-9]{32}-[1-9][0-9]{0,19}-[a-f0-9]{32}$/.test(owner))
    )
      return false;
  }
  for (const [name, value] of Object.entries(params)) {
    if (!allowed.has(name) || Array.isArray(value)) {
      return false;
    }
  }

  return entry.params.every((rule) => {
    const value = params[rule.name];
    if (value === undefined) {
      return !rule.required;
    }
    // Owner selection carries editable input, including an empty reset value.
    // URI admission and native signature verification still happen before save.
    if (rule.kind === "scalar" && rule.format === "contactCardDraft")
      return typeof value === "string" && value.length <= MAX_CONTACT_CARD_URI_LENGTH + 1;
    return isNonEmptySingle(value) && valueMatches(rule, value);
  });
}

export function navigationEntries(
  layout: "phone" | "wide",
  showUnavailableFeatures: boolean,
): readonly ScreenCatalogEntry[] {
  const orderFor = (entry: ScreenCatalogEntry): number | undefined =>
    layout === "phone" ? entry.phoneRootOrder : entry.wideRootOrder;
  return screenCatalog
    .filter((entry) => orderFor(entry) !== undefined)
    .filter((entry) => showUnavailableFeatures || entry.availability === "implementedScaffold")
    .sort((left, right) => (orderFor(left) ?? 0) - (orderFor(right) ?? 0));
}
