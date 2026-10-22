import { useState } from "react";
import { Linking } from "react-native";
import { bluetoothStatus } from "@/features/connections/bluetooth-status";
import { connectedIpCount } from "@/features/connections/ip-physical-connections";
import { SetupLanChoice } from "@/features/connections/ip-connections";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { InfoDisclosure } from "@/ui/info-disclosure";
import { NavigationLink } from "@/ui/navigation-link";
import { Badge, BodyText, Button, Card, CardHeader } from "@/ui/primitives";

export function NearbySetupStep({ onContinue }: { readonly onContinue: () => void }) {
  const runtime = useDevelopmentRuntime();
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const status = bluetoothStatus(runtime);
  const networkReady = connectedIpCount(runtime.connectionSnapshot) > 0;
  const ready = status.label === "Ready" || status.showConnectedPeers === true || networkReady;
  const android = runtime.androidRuntime;
  const access = android?.status;
  const authorization = runtime.bluetoothAuthorization?.authorization;
  const blocked = access?.bluetoothPermission === "blocked" || authorization === "denied";
  const needsPermission =
    access !== undefined && access !== null && access.bluetoothPermission !== "granted";
  const enabled = runtime.snapshot?.bluetooth.desiredEnabled;

  const act = async (action: () => Promise<void>) => {
    if (pending) return;
    setPending(true);
    setFailure(null);
    try {
      await action();
    } catch {
      setFailure("That action could not finish. Try again.");
    } finally {
      setPending(false);
    }
  };
  const refresh = async () => {
    await android?.refresh();
    const result = await runtime.refreshSnapshot();
    if (result.type === "operationFailure")
      setFailure("Connection status could not be checked. Try again.");
  };
  const enable = async () => {
    const result = await runtime.setBluetoothEnabled(true);
    if (result.type === "operationFailure" || result.outcome.tag === "Unavailable")
      setFailure("Bluetooth could not be enabled. Refresh and try again.");
    else if (result.outcome.tag === "Busy")
      setFailure("Connections are changing. Wait a moment, then try again.");
  };

  return (
    <Card>
      <CardHeader title="Choose your connections" />
      <Badge tone={ready ? "neutral" : status.warning ? "warning" : "neutral"}>
        {ready
          ? status.showConnectedPeers
            ? status.label
            : networkReady
              ? "Network connected"
              : "Ready for nearby phones"
          : status.label}
      </Badge>
      <BodyText>
        {networkReady
          ? "Your LAN or TCP connection is ready. Message delivery is shown in each conversation."
          : ready
            ? "Keep prns open on both phones."
            : status.description}
      </BodyText>
      <NavigationLink href="/more/interfaces">Set up local network or TCP</NavigationLink>
      <SetupLanChoice />
      {blocked ? (
        <Button disabled={pending} onPress={() => void act(Linking.openSettings)}>
          Open app settings
        </Button>
      ) : needsPermission && android !== null ? (
        <Button
          disabled={pending}
          onPress={() =>
            void act(async () => {
              await android.requestBluetoothPermissions();
              await refresh();
            })
          }
        >
          Allow Bluetooth
        </Button>
      ) : null}
      {access?.bluetoothRadio === "off" ? (
        <Button
          disabled={pending}
          onPress={() => void act(() => Linking.sendIntent("android.settings.BLUETOOTH_SETTINGS"))}
        >
          Open Bluetooth settings
        </Button>
      ) : null}
      {access?.locationServices === "off" ? (
        <Button
          disabled={pending}
          onPress={() =>
            void act(() => Linking.sendIntent("android.settings.LOCATION_SOURCE_SETTINGS"))
          }
        >
          Open Location settings
        </Button>
      ) : null}
      {enabled === false && !blocked && !needsPermission && authorization !== "restricted" ? (
        <Button disabled={pending} onPress={() => void act(enable)}>
          {pending ? "Enabling…" : "Enable nearby connections"}
        </Button>
      ) : null}
      {authorization === "notDetermined" && enabled !== false ? (
        <BodyText>Allow Bluetooth when your phone asks.</BodyText>
      ) : null}
      {failure === null ? null : <BodyText>{failure}</BodyText>}
      <Button disabled={pending} tone="secondary" onPress={() => void act(refresh)}>
        {pending ? "Checking…" : "Check again"}
      </Button>
      <Button disabled={!ready || pending} onPress={onContinue}>
        Continue to your identity
      </Button>
      {!ready ? (
        <>
          <BodyText>You can set up your identity now and connect later.</BodyText>
          <Button disabled={pending} tone="secondary" onPress={onContinue}>
            Continue offline
          </Button>
        </>
      ) : null}
      <InfoDisclosure label="Other connections and details">
        <BodyText>
          No pairing is needed between phones. Bluetooth readiness does not mean a message has been
          delivered.
        </BodyText>
        <BodyText>
          Your current discovery group and radio settings are kept. Phones using different discovery
          groups will not find each other.
        </BodyText>
        <NavigationLink href="/more/interfaces">Connection settings</NavigationLink>
        <NavigationLink href="/nodes/pair">Pair a radio node</NavigationLink>
      </InfoDisclosure>
    </Card>
  );
}
