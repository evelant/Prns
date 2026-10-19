import * as Bindings from "@prns-internal/expo";
import { fireEvent, render } from "@testing-library/react-native";
import { identityReferenceKey, type NamedIdentity } from "@/native/named-identity-context";
import { IdentityActivityLabel } from "./identity-activity";
import { IdentityPicker } from "./identity-picker";

const personal: NamedIdentity = {
  reference: {
    incarnation: new Uint8Array(16),
    sequence: 1n,
    publicHash: new Uint8Array(16).fill(1),
  },
  label: "Personal",
  publishedName: undefined,
  state: Bindings.NamedIdentityState.Enabled,
  ready: true,
  transitioning: false,
};
const work: NamedIdentity = {
  ...personal,
  reference: { ...personal.reference, sequence: 2n, publicHash: new Uint8Array(16).fill(2) },
  label: "Work",
  state: Bindings.NamedIdentityState.Paused,
  ready: false,
};
const retired: NamedIdentity = {
  ...personal,
  reference: { ...personal.reference, sequence: 3n },
  label: "Old",
  state: Bindings.NamedIdentityState.Retired,
  ready: false,
};
let mockActivity: Bindings.NamedIdentityActivity[];
let mockFailure: string | null;
const mockCatalog = { selected: personal.reference, identities: [personal, work, retired] };
jest.mock("@/native/named-identity-context", () => ({
  ...jest.requireActual("@/native/named-identity-context"),
  useNamedIdentity: () => ({
    catalog: mockCatalog,
    activity: mockActivity,
    activityFailure: mockFailure,
  }),
}));
beforeEach(() => {
  mockFailure = null;
  mockActivity = [
    {
      identity: work.reference,
      activity: Bindings.IdentityActivityOutcome.Ready.new({
        summary: {
          actionableInvitations: 1,
          unreadGroups: 2,
          unreadDirect: 0,
          conversations: [],
          partial: false,
        },
      }),
    },
  ];
});

test("the compact picker exposes activity in another identity and selection does not change the default", () => {
  const select = jest.fn();
  const view = render(
    <IdentityPicker selectedKey={identityReferenceKey(personal.reference)} onSelect={select} />,
  );
  expect(view.getByText("3 new")).toBeTruthy();
  fireEvent.press(
    view.getByRole("button", { name: "Identity: Personal, 3 updates in other identities" }),
  );
  expect(view.getByText("1 invitation · 2 unread")).toBeTruthy();
  expect(view.queryByRole("radio", { name: /Old/ })).toBeNull();
  fireEvent.press(view.getByRole("radio", { name: "Work, Paused" }));
  expect(select).toHaveBeenCalledWith(work);
  expect(mockCatalog.selected).toBe(personal.reference);
});

test("partial and unavailable counts remain visibly uncertain rather than zero", () => {
  mockActivity[0] = {
    identity: work.reference,
    activity: Bindings.IdentityActivityOutcome.Ready.new({
      summary: {
        actionableInvitations: 1,
        unreadGroups: 2,
        unreadDirect: 0,
        conversations: [],
        partial: true,
      },
    }),
  };
  mockActivity.push({
    identity: personal.reference,
    activity: Bindings.IdentityActivityOutcome.Unavailable.new({ detail: "Busy" }),
  });
  const view = render(
    <IdentityPicker selectedKey={identityReferenceKey(personal.reference)} onSelect={jest.fn()} />,
  );
  expect(view.getByText("3+ new")).toBeTruthy();
  fireEvent.press(view.getByRole("button", { name: /activity incomplete/ }));
  expect(view.getByText("Activity unavailable")).toBeTruthy();
  expect(view.getByText("More activity may be available")).toBeTruthy();
});

test("identity management shows invitations and unread counts for that identity only", () => {
  const view = render(
    <IdentityActivityLabel ownerKey={identityReferenceKey(personal.reference)} />,
  );
  expect(view.queryByText(/invitation/)).toBeNull();
  view.rerender(<IdentityActivityLabel ownerKey={identityReferenceKey(work.reference)} />);
  expect(view.getByText("1 invitation · 2 unread")).toBeTruthy();
});
