import * as Bindings from "@prns-internal/expo";

import { formatBytes } from "@/features/nodes/format";
import { Disclosure } from "@/ui/disclosure";
import { Badge, BodyText, CardSection, KeyValue } from "@/ui/primitives";
import { connectionStateLabel } from "./ip-connections";

/** Physical rows are details; their parent totals already include this traffic. */
export function ipPhysicalConnections(snapshot: Bindings.ConnectionSnapshot | null | undefined) {
  return (
    snapshot?.physicalConnections.filter(
      (peer) => !peer.localOnly && /tcp|wifi|lan|udp|backbone|websocket/iu.test(peer.kind),
    ) ?? []
  );
}

export function connectedIpCount(snapshot: Bindings.ConnectionSnapshot | null | undefined): number {
  return ipPhysicalConnections(snapshot).filter(
    (peer) => peer.state === Bindings.LocalConnectionState.Connected,
  ).length;
}

export function IpPhysicalConnections({
  snapshot,
}: {
  readonly snapshot: Bindings.ConnectionSnapshot | null | undefined;
}) {
  const peers = ipPhysicalConnections(snapshot);
  return (
    <>
      {peers.length === 0 ? <BodyText>No LAN or TCP peers connected.</BodyText> : null}
      {peers.map((peer) => {
        const profile = snapshot?.settings.settings.tcpPeers.find(
          (profile) => profile.id === peer.profileId,
        );
        return (
          <CardSection
            key={formatBytes(peer.interfaceId)}
            title={profile?.label ?? peer.name ?? peer.endpoint ?? "Network peer"}
          >
            <Badge>{connectionStateLabel(peer.state)}</Badge>
            {peer.endpoint === undefined ? null : <BodyText>{peer.endpoint}</BodyText>}
            <Disclosure
              label={`Network connection details for ${peer.name ?? peer.endpoint ?? formatBytes(peer.interfaceId)}`}
            >
              <KeyValue label="Connection ID" value={formatBytes(peer.interfaceId)} />
              <KeyValue label="Logical interface ID" value={formatBytes(peer.logicalInterfaceId)} />
              {profile === undefined ? null : (
                <KeyValue label="Saved TCP peer" value={profile.label} />
              )}
              {peer.parentInterfaceId === undefined ? null : (
                <KeyValue label="Parent interface ID" value={formatBytes(peer.parentInterfaceId)} />
              )}
              <KeyValue label="Received on this connection" value={`${peer.rxBytes} bytes`} />
              <KeyValue label="Sent on this connection" value={`${peer.txBytes} bytes`} />
            </Disclosure>
          </CardSection>
        );
      })}
    </>
  );
}
