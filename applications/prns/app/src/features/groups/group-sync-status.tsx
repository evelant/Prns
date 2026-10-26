import {
  type GroupPeerSynchronizationProgress,
  GroupSynchronizationIssue,
  GroupSynchronizationPhase,
  type GroupSynchronizationProgress,
} from "@prns-internal/expo";

import { BodyText, Card, Subheading } from "@/ui/primitives";
import { identifierHex } from "./format";

/** A read-only view of the native runner, independent of durable device receipts. */
export function GroupSyncStatus({ progress }: { readonly progress: GroupSynchronizationProgress }) {
  if (!progress.running) return null;
  return (
    <Card>
      <Subheading>Delivery activity</Subheading>
      {!progress.observed ? (
        <BodyText muted>Waiting for synchronization activity.</BodyText>
      ) : progress.peers.length === 0 ? (
        <BodyText muted>No peer activity to show yet.</BodyText>
      ) : (
        progress.peers.map((peer) => (
          <BodyText
            key={`${identifierHex(peer.endpointId)}:${peer.endpointIncarnation.toString()}`}
          >
            {`Device ${identifierHex(peer.identity).slice(0, 8)} · ${phaseLabel(peer)}${issueLabel(peer.lastIssue)} · observed ${observationAge(peer.updatedAgoMillis)}`}
          </BodyText>
        ))
      )}
      <BodyText muted>
        Activity shows the last observations. Message receipts below confirm which devices received
        each message.
      </BodyText>
    </Card>
  );
}

function observationAge(millis: bigint): string {
  if (millis < 1_000n) return "just now";
  if (millis < 60_000n) return `${(millis / 1_000n).toString()} seconds ago`;
  if (millis < 3_600_000n) return `${(millis / 60_000n).toString()} minutes ago`;
  return `${(millis / 3_600_000n).toString()} hours ago`;
}

function phaseLabel(peer: GroupPeerSynchronizationProgress): string {
  switch (peer.phase) {
    case GroupSynchronizationPhase.Unknown:
      return "Activity unavailable";
    case GroupSynchronizationPhase.Waiting:
      return peer.pending === true ? "Saved work waiting to synchronize" : "Waiting to synchronize";
    case GroupSynchronizationPhase.Connecting:
      return "Connecting";
    case GroupSynchronizationPhase.Synchronizing:
      return "Synchronizing records and receipts";
    case GroupSynchronizationPhase.RetryScheduled:
      return peer.retryAfterMillis === undefined
        ? "Waiting to retry"
        : `Retry scheduled · about ${((peer.retryAfterMillis + 999n) / 1000n).toString()} seconds remaining`;
    case GroupSynchronizationPhase.StorageBlocked:
      return "Waiting for storage";
    case GroupSynchronizationPhase.MembershipCatchUp:
      return "Catching up on membership";
  }
}

function issueLabel(issue: GroupSynchronizationIssue | undefined): string {
  switch (issue) {
    case undefined:
      return "";
    case GroupSynchronizationIssue.Transport:
      return " · Connection not confirmed on the last attempt";
    case GroupSynchronizationIssue.StorageBusy:
      return " · Storage busy";
    case GroupSynchronizationIssue.StorageFull:
      return " · Storage full";
    case GroupSynchronizationIssue.StorageUnavailable:
      return " · Storage unavailable";
    case GroupSynchronizationIssue.MembershipChanged:
      return " · Membership changed";
    case GroupSynchronizationIssue.TimeUncertain:
      return " · Device time needs attention";
    case GroupSynchronizationIssue.Rejected:
      return " · Last exchange was rejected";
    case GroupSynchronizationIssue.UnsupportedPeer:
      return " · Peer does not support this exchange";
    case GroupSynchronizationIssue.Configuration:
      return " · Group configuration needs attention";
  }
}
