import * as Bindings from "@prns-internal/expo";
import { render } from "@testing-library/react-native";

import { GroupSyncStatus } from "./group-sync-status";

const peer: Bindings.GroupPeerSynchronizationProgress = {
  endpointId: new Uint8Array(32).fill(1),
  endpointIncarnation: 1n,
  identity: new Uint8Array(16).fill(2),
  phase: Bindings.GroupSynchronizationPhase.Waiting,
  pending: undefined,
  retryAfterMillis: undefined,
  failures: 0,
  updatedAgoMillis: 0n,
  lastIssue: undefined,
};

test("retry activity remains distinct from durable receipt evidence", () => {
  const screen = render(
    <GroupSyncStatus
      progress={{
        running: true,
        observed: true,
        peers: [
          {
            ...peer,
            phase: Bindings.GroupSynchronizationPhase.RetryScheduled,
            pending: true,
            retryAfterMillis: 14_001n,
            failures: 3,
            lastIssue: Bindings.GroupSynchronizationIssue.Transport,
          },
        ],
      }}
    />,
  );
  expect(screen.getByText(/Retry scheduled · about 15 seconds remaining/)).toBeTruthy();
  expect(screen.getByText(/Connection not confirmed on the last attempt/)).toBeTruthy();
  expect(screen.getByText(/Message receipts below confirm which devices received/)).toBeTruthy();
  expect(screen.queryByText(/Delivered|Received by|Connected$/)).toBeNull();
});

test("stopped runner hides previous peer activity instead of showing a live retry", () => {
  const screen = render(
    <GroupSyncStatus
      progress={{ running: false, observed: true, peers: [{ ...peer, pending: true }] }}
    />,
  );
  expect(screen.queryByText("Delivery activity")).toBeNull();
});

test("unknown queue observation does not imply delivery completion", () => {
  const screen = render(
    <GroupSyncStatus progress={{ running: true, observed: true, peers: [peer] }} />,
  );
  expect(
    screen.getByText(/Device 02020202 · Waiting to synchronize · observed just now/),
  ).toBeTruthy();
  expect(screen.queryByText(/All delivered|Up to date|No pending/)).toBeNull();
});

test("old activity retains its observation age", () => {
  const screen = render(
    <GroupSyncStatus
      progress={{
        running: true,
        observed: true,
        peers: [
          {
            ...peer,
            phase: Bindings.GroupSynchronizationPhase.Connecting,
            updatedAgoMillis: 120_000n,
          },
        ],
      }}
    />,
  );
  expect(screen.getByText(/Connecting · observed 2 minutes ago/)).toBeTruthy();
});

test("membership and storage barriers are visible without asking for a replacement send", () => {
  const screen = render(
    <GroupSyncStatus
      progress={{
        running: true,
        observed: true,
        peers: [
          {
            ...peer,
            phase: Bindings.GroupSynchronizationPhase.StorageBlocked,
            lastIssue: Bindings.GroupSynchronizationIssue.StorageFull,
          },
          {
            ...peer,
            endpointIncarnation: 2n,
            phase: Bindings.GroupSynchronizationPhase.MembershipCatchUp,
            lastIssue: Bindings.GroupSynchronizationIssue.MembershipChanged,
          },
        ],
      }}
    />,
  );
  expect(screen.getByText(/Waiting for storage · Storage full/)).toBeTruthy();
  expect(screen.getByText(/Catching up on membership · Membership changed/)).toBeTruthy();
  expect(screen.queryByRole("button")).toBeNull();
});
