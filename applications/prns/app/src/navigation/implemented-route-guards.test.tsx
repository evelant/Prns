import { render } from "@testing-library/react-native";
import type { ComponentType } from "react";
import ContactRoute from "../../app/(shell)/contacts/[destination]";
import AddContactRoute from "../../app/(shell)/contacts/add";
import MyContactCardRoute from "../../app/(shell)/contacts/card";
import ImportContactCardRoute from "../../app/(shell)/contacts/import";
import ContactsRoute from "../../app/(shell)/contacts/index";
import ComposeRoute from "../../app/(shell)/inbox/compose";
import ConversationRoute from "../../app/(shell)/inbox/conversation/[destination]";
import GroupConversationRoute from "../../app/(shell)/inbox/group/[groupId]";
import InviteToGroupRoute from "../../app/(shell)/inbox/group/[groupId]/invite";
import CreateGroupRoute from "../../app/(shell)/inbox/group/new";
import InboxRoute from "../../app/(shell)/inbox/index";
import GroupInvitationRoute from "../../app/(shell)/inbox/invitation/[invitationId]";
import ActivityRoute from "../../app/(shell)/more/activity/index";
import HelpRoute from "../../app/(shell)/more/help/index";
import MoreRoute from "../../app/(shell)/more/index";
import ConnectionsRoute from "../../app/(shell)/more/interfaces/index";
import AboutRoute from "../../app/(shell)/more/settings/about";
import SettingsRoute from "../../app/(shell)/more/settings/index";
import NodesRoute from "../../app/(shell)/nodes/index";
import LocalNodeRoute from "../../app/(shell)/nodes/local/index";
import SetupRoute from "../../app/(shell)/setup";
import OnboardingRoute from "../../app/onboarding/index";
import RecoveryRoute from "../../app/recovery";
import type { RawRouteParams } from "./catalog";

const mockUseLocalSearchParams = jest.fn<RawRouteParams, []>();

jest.mock("expo-router", () => ({
  useLocalSearchParams: () => mockUseLocalSearchParams(),
  useRouter: () => ({ replace: jest.fn() }),
}));

jest.mock("@/features/onboarding/onboarding-screen", () => ({
  OnboardingScreen: () => null,
}));

jest.mock("@/features/setup/setup-screen", () => ({ SetupScreen: () => null }));

jest.mock("@/features/onboarding/recovery-screen", () => ({
  RecoveryScreen: () => null,
}));

jest.mock("@/features/inbox/inbox-screen", () => ({
  ComposeScreen: () => null,
  ConversationScreen: () => null,
  InboxScreen: () => null,
}));
jest.mock("@/features/groups/group-conversation-screen", () => ({
  GroupConversationScreen: () => null,
}));
jest.mock("@/features/groups/create-group-screen", () => ({ CreateGroupScreen: () => null }));
jest.mock("@/features/groups/invite-to-group-screen", () => ({ InviteToGroupScreen: () => null }));
jest.mock("@/features/groups/group-invitation-screen", () => ({
  GroupInvitationScreen: () => null,
}));

jest.mock("@/features/contacts/contacts-screen", () => ({
  AddContactScreen: () => null,
  ContactDetailScreen: () => null,
  ContactsScreen: () => null,
}));

jest.mock("@/features/contacts/my-contact-card-screen", () => ({
  MyContactCardScreen: () => null,
}));
jest.mock("@/features/contacts/contact-card-import", () => ({
  ContactCardImportScreen: () => null,
}));

jest.mock("@/features/nodes/nodes-screen", () => ({
  LocalNodeScreen: () => null,
  NodesScreen: () => null,
}));

jest.mock("@/features/connections/connections-screen", () => ({ ConnectionsScreen: () => null }));
jest.mock("@/features/network/network-screen", () => ({ NetworkScreen: () => null }));

jest.mock("@/features/more/more-screen", () => ({
  MoreScreen: () => null,
}));

jest.mock("@/features/settings/settings-screen", () => ({
  SettingsScreen: () => null,
}));

jest.mock("@/features/settings/about-screen", () => ({
  AboutScreen: () => null,
}));

jest.mock("@/features/settings/help-screen", () => ({
  HelpScreen: () => null,
}));

type ImplementedRouteCase = {
  readonly label: string;
  readonly Component: ComponentType;
  readonly validParams: RawRouteParams;
};

const destination = "00".repeat(16);
const implementedRoutes: readonly ImplementedRouteCase[] = [
  { label: "onboarding", Component: OnboardingRoute, validParams: {} },
  { label: "connection setup", Component: SetupRoute, validParams: {} },
  {
    label: "recovery",
    Component: RecoveryRoute,
    validParams: { installationId: "current" },
  },
  { label: "inbox", Component: InboxRoute, validParams: {} },
  {
    label: "conversation",
    Component: ConversationRoute,
    validParams: { destination },
  },
  { label: "compose", Component: ComposeRoute, validParams: { destination } },
  {
    label: "group conversation",
    Component: GroupConversationRoute,
    validParams: { groupId: "11".repeat(32) },
  },
  { label: "contacts", Component: ContactsRoute, validParams: {} },
  { label: "new group", Component: CreateGroupRoute, validParams: {} },
  {
    label: "invite device",
    Component: InviteToGroupRoute,
    validParams: { groupId: "11".repeat(32) },
  },
  {
    label: "group invitation",
    Component: GroupInvitationRoute,
    validParams: { invitationId: "22".repeat(32) },
  },
  { label: "contact", Component: ContactRoute, validParams: { destination } },
  { label: "add contact", Component: AddContactRoute, validParams: {} },
  { label: "my contact card", Component: MyContactCardRoute, validParams: {} },
  {
    label: "import contact card",
    Component: ImportContactCardRoute,
    validParams: { card: "rns-contact://v1/YWJj" },
  },
  { label: "nodes", Component: NodesRoute, validParams: {} },
  { label: "this device", Component: LocalNodeRoute, validParams: {} },
  { label: "more", Component: MoreRoute, validParams: {} },
  { label: "connections", Component: ConnectionsRoute, validParams: {} },
  { label: "network", Component: ActivityRoute, validParams: { filter: "routes" } },
  { label: "settings", Component: SettingsRoute, validParams: {} },
  { label: "about", Component: AboutRoute, validParams: {} },
  { label: "help", Component: HelpRoute, validParams: {} },
];

describe("implemented route parameter guards", () => {
  it.each(implementedRoutes)("rejects unknown $label parameters", ({ Component, validParams }) => {
    mockUseLocalSearchParams.mockReturnValue({ ...validParams, unexpected: "value" });

    const view = render(<Component />);

    expect(view.getByText("Not found")).toBeTruthy();
  });

  it.each(implementedRoutes)("accepts declared $label parameters", ({ Component, validParams }) => {
    mockUseLocalSearchParams.mockReturnValue(validParams);

    const view = render(<Component />);

    expect(view.queryByText("Not found")).toBeNull();
  });
});
