import { DevelopmentNodeRuntime } from "@prns-internal/expo";
import { useState } from "react";

import { AndroidBluetoothCard } from "@/features/nodes/android-bluetooth-card";
import { IosBluetoothCard } from "@/features/nodes/ios-bluetooth-card";
import { formatBytes, formatRuntime } from "@/features/nodes/format";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { Disclosure } from "@/ui/disclosure";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import {
  ActionRow,
  Badge,
  BodyText,
  Button,
  Card,
  CardHeader,
  CardSection,
  KeyValue,
  Screen,
  ScreenHeading,
} from "@/ui/primitives";
import { SectionPanel } from "@/ui/section-panel";
import { SectionTabs } from "@/ui/section-tabs";
import { bluetoothStatus } from "./bluetooth-status";
import { DiscoveryGroupsSettings } from "./discovery-groups-settings";
import { IpConnections } from "./ip-connections";
import { IpPhysicalConnections } from "./ip-physical-connections";

export function ConnectionsScreen({
  initialSection = "bluetooth",
}: {
  readonly initialSection?: "bluetooth" | "ip";
}) {
  const runtime = useDevelopmentRuntime();
  const [section, setSection] = useState(initialSection);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const bluetooth = runtime.snapshot?.bluetooth;
  const status = bluetoothStatus(runtime);
  const disabled =
    pending ||
    runtime.phase === "starting" ||
    runtime.stoppingNode ||
    bluetooth?.desiredEnabled === undefined ||
    bluetooth.state.tag === "Disabling";

  const changeBluetooth = async () => {
    if (disabled || bluetooth?.desiredEnabled === undefined) return;
    setPending(true);
    setFailure(null);
    try {
      const result = await runtime.setBluetoothEnabled(!bluetooth.desiredEnabled);
      if (result.type === "operationFailure") setFailure(result.detail);
      else if (result.outcome.tag === "Busy")
        setFailure("This phone is changing connections. Wait a moment and try again.");
      else if (result.outcome.tag === "Unavailable")
        setFailure(
          "The Bluetooth setting could not be confirmed. Refresh to check it, then try again.",
        );
    } finally {
      setPending(false);
    }
  };
  const refresh = async () => {
    if (pending) return;
    setPending(true);
    setFailure(null);
    try {
      await runtime.androidRuntime?.refresh();
      const result = await runtime.refreshSnapshot();
      if (result.type === "operationFailure") setFailure(result.detail);
    } finally {
      setPending(false);
    }
  };

  return (
    <Screen>
      <ScreenHeading
        trailing={
          <InfoDisclosure label="About interfaces">
            <BodyText>
              Connect using Bluetooth, local network discovery or saved TCP peers.
            </BodyText>
            <BodyText>
              Turning connections off keeps your identities, contacts and messages saved.
              {runtime.snapshot?.runtime === DevelopmentNodeRuntime.Running
                ? " Your node keeps running."
                : ""}
            </BodyText>
            <BodyText>
              Bluetooth peers need matching Bluetooth discovery groups. Connected peers do not
              confirm that a message was delivered.
            </BodyText>
          </InfoDisclosure>
        }
      >
        Interfaces
      </ScreenHeading>
      <BodyText muted>
        Choose how this phone connects. Settings apply to all your identities.
      </BodyText>
      <Card>
        <CardHeader title="This phone">
          <Badge>
            {runtime.snapshot === null ? "Getting ready" : formatRuntime(runtime.snapshot.runtime)}
          </Badge>
        </CardHeader>
        {runtime.canStartNode ? <Button onPress={runtime.startNode}>Start node</Button> : null}
        <Disclosure label="Setup and diagnostics">
          <NavigationLink href="/setup">Connection setup</NavigationLink>
          <ActionRow>
            <NavigationLink href="/nodes/local">Node diagnostics</NavigationLink>
            <NavigationLink href="/more/activity">Network details</NavigationLink>
          </ActionRow>
        </Disclosure>
      </Card>
      <SectionTabs
        label="Interface types"
        options={[
          { value: "bluetooth", label: "Bluetooth" },
          { value: "ip", label: "LAN & TCP" },
        ]}
        value={section}
        onChange={setSection}
      />
      <SectionPanel active={section === "bluetooth"}>
        <Card>
          <CardHeader title="Bluetooth connections">
            <Badge tone={status.warning ? "warning" : "neutral"}>{status.label}</Badge>
          </CardHeader>
          <BodyText>{status.description}</BodyText>
          {runtime.availability.type === "available" ? (
            <ActionRow>
              <Button disabled={disabled} onPress={() => void changeBluetooth()} tone="secondary">
                {pending
                  ? "Please wait…"
                  : bluetooth?.desiredEnabled === false
                    ? "Enable Bluetooth connections"
                    : "Disable Bluetooth connections"}
              </Button>
              <Button disabled={pending} onPress={() => void refresh()} tone="secondary">
                Refresh connections
              </Button>
            </ActionRow>
          ) : null}
          {failure === null ? null : <BodyText>{failure}</BodyText>}
          {status.showConnectedPeers
            ? bluetooth?.peers
                .filter((peer) => peer.connected)
                .map((peer, index) => (
                  <CardSection
                    key={formatBytes(peer.interfaceId)}
                    title={peer.name ?? "Nearby device"}
                  >
                    <Badge>Connected</Badge>
                    <Disclosure
                      label="Connection details"
                      accessibilityLabel={`Connection details for ${peer.name ?? "nearby device"} ${index + 1}`}
                    >
                      <KeyValue label="Connection ID" value={formatBytes(peer.interfaceId)} />
                      {peer.details === undefined ? null : <BodyText>{peer.details}</BodyText>}
                      {peer.rssiDbm === undefined ? null : (
                        <KeyValue label="Signal" value={`${peer.rssiDbm} dBm`} />
                      )}
                      <KeyValue
                        label="Received on this connection"
                        value={`${peer.rxBytes.toString()} bytes`}
                      />
                      <KeyValue
                        label="Sent on this connection"
                        value={`${peer.txBytes.toString()} bytes`}
                      />
                    </Disclosure>
                  </CardSection>
                ))
            : null}
        </Card>
        <AndroidBluetoothCard />
        <IosBluetoothCard />
        <DiscoveryGroupsSettings />
      </SectionPanel>
      <SectionPanel active={section === "ip"}>
        <IpConnections />
        <Card>
          <CardHeader title="Live LAN and TCP connections" />
          <IpPhysicalConnections snapshot={runtime.connectionSnapshot} />
        </Card>
      </SectionPanel>
      <NavigationLink href="/nodes" direction="back">
        Back to Nodes
      </NavigationLink>
    </Screen>
  );
}
