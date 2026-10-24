import * as Bindings from "@prns-internal/expo";
import { useCallback, useEffect, useRef, useState } from "react";
import { Linking } from "react-native";

import { formatBytes } from "@/features/nodes/format";
import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { Disclosure } from "@/ui/disclosure";
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
} from "@/ui/primitives";
import { TcpPeerForm, type TcpPeerDraft } from "./tcp-peer-form";

export function connectionStateLabel(state: Bindings.LocalConnectionState): string {
  switch (state) {
    case Bindings.LocalConnectionState.Stopped:
      return "Node stopped";
    case Bindings.LocalConnectionState.Disabled:
      return "Disabled";
    case Bindings.LocalConnectionState.Applying:
      return "Applying settings";
    case Bindings.LocalConnectionState.Discovering:
      return "Discovering";
    case Bindings.LocalConnectionState.Connecting:
      return "Connecting";
    case Bindings.LocalConnectionState.Connected:
      return "Connected";
    case Bindings.LocalConnectionState.Reconnecting:
      return "Retrying";
    case Bindings.LocalConnectionState.Unavailable:
      return "Unavailable";
    case Bindings.LocalConnectionState.Failed:
      return "Needs attention";
    case Bindings.LocalConnectionState.Retiring:
      return "Disconnecting";
  }
}

export function lanDiscoveryLabel(method: Bindings.LanDiscoveryMethod): string {
  switch (method) {
    case Bindings.LanDiscoveryMethod.Bonjour:
      return "Bonjour · compatible PRNS peers";
    case Bindings.LanDiscoveryMethod.StockMulticast:
      return "Reticulum multicast discovery";
    case Bindings.LanDiscoveryMethod.Combined:
      return "Reticulum multicast and Bonjour";
    case Bindings.LanDiscoveryMethod.Unavailable:
      return "Discovery unavailable";
  }
}

export function tcpEndpoint(profile: Pick<Bindings.TcpPeerProfile, "host" | "port">): string {
  return `${profile.host.includes(":") && !profile.host.startsWith("[") ? `[${profile.host}]` : profile.host}:${profile.port}`;
}

function useConnectionSettings() {
  const runtime = useDevelopmentRuntime();
  const { readConnectionSettings, updateConnectionSettings, refreshConnections } = runtime;
  const [saved, setSaved] = useState<Bindings.ConnectionSettingsSnapshot | null>(
    runtime.connectionSnapshot?.settings ?? null,
  );
  const [pending, setPending] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [permissionBlocked, setPermissionBlocked] = useState(false);
  const mounted = useRef(false);
  const activeRead = useRef(readConnectionSettings);
  activeRead.current = readConnectionSettings;
  const inFlight = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const observed = runtime.connectionSnapshot?.settings;
  useEffect(() => {
    if (observed !== undefined)
      setSaved((previous) =>
        previous?.ownerId === observed.ownerId && previous.revision > observed.revision
          ? previous
          : observed,
      );
  }, [observed]);
  const load = useCallback(async () => {
    if (typeof readConnectionSettings !== "function") return false;
    const result = await readConnectionSettings();
    if (!mounted.current || activeRead.current !== readConnectionSettings) return false;
    if (
      result.type === "outcome" &&
      (result.outcome.tag === "Ready" || result.outcome.tag === "Conflict")
    ) {
      setSaved(result.outcome.inner.snapshot);
      return true;
    } else {
      setSaved(null);
      setFeedback(
        result.type === "operationFailure"
          ? result.detail
          : result.outcome.tag === "InvalidSaved" || result.outcome.tag === "Unavailable"
            ? result.outcome.inner.detail
            : "Connection settings could not be checked. Refresh to try again.",
      );
      return false;
    }
  }, [readConnectionSettings]);
  useEffect(() => {
    setSaved(null);
    setFeedback(null);
    void load();
  }, [load]);
  const save = async (settings: Bindings.ConnectionSettings): Promise<boolean> => {
    if (saved === null || inFlight.current || typeof updateConnectionSettings !== "function")
      return false;
    inFlight.current = true;
    setPending(true);
    setFeedback(null);
    const owner = readConnectionSettings;
    try {
      let permission: Bindings.LocalNetworkPermission | undefined;
      if (!saved.settings.lanEnabled && settings.lanEnabled) {
        const access = await runtime.requestLocalNetworkPermission();
        if (!mounted.current || activeRead.current !== owner) return false;
        if (access.type === "operationFailure") {
          setFeedback(access.detail);
          return false;
        }
        permission = access.outcome;
        setPermissionBlocked(permission === "denied" || permission === "blocked");
      }
      const result = await updateConnectionSettings({
        ownerId: saved.ownerId,
        expectedRevision: saved.revision,
        settings,
      });
      if (!mounted.current || activeRead.current !== owner) return false;
      if (result.type === "outcome" && result.outcome.tag === "Ready") {
        setSaved(result.outcome.inner.snapshot);
        setFeedback(
          permission === "denied" || permission === "blocked"
            ? "Saved. Allow local network access in app settings to discover LAN peers."
            : "Saved. Connection status shows whether your peer is available.",
        );
        return true;
      }
      if (result.type === "outcome" && result.outcome.tag === "Conflict") {
        setSaved(result.outcome.inner.snapshot);
        setFeedback(
          "Connection settings changed. Review the current saved settings before trying again.",
        );
        return false;
      }
      if (result.type === "outcome" && result.outcome.tag === "Invalid") {
        setFeedback(result.outcome.inner.detail);
        return false;
      }
      // A lost acknowledgement does not prove that a durable save failed.
      setSaved(null);
      const confirmed = await load();
      if (mounted.current && activeRead.current === owner)
        setFeedback(
          confirmed
            ? "The save result was not confirmed. Saved settings have been checked; review them before another change."
            : "The save result is still unconfirmed. Refresh saved settings before trying another change.",
        );
      return false;
    } finally {
      inFlight.current = false;
      if (mounted.current && activeRead.current === owner) setPending(false);
    }
  };
  const refresh = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    setFeedback(null);
    try {
      await load();
      await refreshConnections?.();
    } finally {
      inFlight.current = false;
      if (mounted.current) setPending(false);
    }
  };
  const retry = async (profileId: string) => {
    const snapshot = runtime.connectionSnapshot;
    if (inFlight.current || saved === null || !snapshot) return;
    inFlight.current = true;
    setPending(true);
    setFeedback(null);
    const owner = readConnectionSettings;
    try {
      const result = await runtime.retryConnection({
        ownerId: saved.ownerId,
        generationId: snapshot.generationId,
        expectedRevision: saved.revision,
        profileId,
      });
      if (!mounted.current || activeRead.current !== owner) return;
      setFeedback(
        result.type === "operationFailure"
          ? result.detail
          : result.outcome.tag === "Unavailable"
            ? result.outcome.inner.detail
            : result.outcome.tag === "Busy"
              ? "Connections are changing. Wait a moment and try again."
              : "Retry started. Connection status shows when the peer is connected.",
      );
    } finally {
      inFlight.current = false;
      if (mounted.current && activeRead.current === owner) setPending(false);
    }
  };
  return { runtime, saved, pending, feedback, save, refresh, retry, permissionBlocked };
}

function canRetry(state: Bindings.LocalConnectionState | undefined): boolean {
  return (
    state === Bindings.LocalConnectionState.Reconnecting ||
    state === Bindings.LocalConnectionState.Failed ||
    state === Bindings.LocalConnectionState.Unavailable
  );
}

/** Setup offers an explicit choice; revisiting it never changes saved intent. */
export function SetupLanChoice() {
  const { runtime, saved, pending, feedback, save, permissionBlocked } = useConnectionSettings();
  const enabled = saved?.settings.lanEnabled;
  const lan = runtime.connectionSnapshot?.lan;
  return (
    <CardSection title="Local network discovery">
      <BodyText>
        Find compatible devices on the same local network. You can also connect to a saved TCP peer
        from Interfaces → LAN & TCP.
      </BodyText>
      {enabled ? (
        <Badge>
          {lan === undefined ? "Enabled · status not checked" : connectionStateLabel(lan.state)}
        </Badge>
      ) : (
        <Button
          disabled={pending || saved === null}
          tone="secondary"
          onPress={() => {
            if (saved) void save({ ...saved.settings, lanEnabled: true });
          }}
        >
          Enable local network discovery
        </Button>
      )}
      {lan === undefined ? null : (
        <BodyText muted>{lanDiscoveryLabel(lan.discoveryMethod)}</BodyText>
      )}
      {feedback ? <BodyText>{feedback}</BodyText> : null}
      {permissionBlocked ? (
        <Button disabled={pending} tone="secondary" onPress={() => void Linking.openSettings()}>
          Open local network settings
        </Button>
      ) : null}
    </CardSection>
  );
}

export function IpConnections({
  profileId,
  initiallyEditing = false,
}: {
  readonly profileId?: string;
  readonly initiallyEditing?: boolean;
}) {
  const { runtime, saved, pending, feedback, save, refresh, retry, permissionBlocked } =
    useConnectionSettings();
  const [editor, setEditor] = useState<string | null>(
    initiallyEditing ? (profileId ?? "new") : null,
  );
  const [removing, setRemoving] = useState<string | null>(null);
  const snapshot = runtime.connectionSnapshot;
  const settings = saved?.settings;
  const peers =
    settings?.tcpPeers.filter((peer) => profileId === undefined || peer.id === profileId) ?? [];
  const changePeers = (tcpPeers: Bindings.TcpPeerProfile[]) =>
    settings === undefined ? Promise.resolve(false) : save({ ...settings, tcpPeers });
  const submit = async (draft: TcpPeerDraft, original?: Bindings.TcpPeerProfile) => {
    const peer = { ...draft, id: original?.id ?? "", enabled: original?.enabled ?? true };
    const next =
      original === undefined
        ? [...(settings?.tcpPeers ?? []), peer]
        : (settings?.tcpPeers ?? []).map((item) => (item.id === original.id ? peer : item));
    if (await changePeers(next)) {
      setEditor(null);
      return true;
    }
    return false;
  };
  return (
    <>
      {profileId === undefined ? (
        <Card>
          <CardHeader title="Local network">
            <Badge>
              {snapshot === null || snapshot === undefined
                ? "Status not checked"
                : connectionStateLabel(snapshot.lan.state)}
            </Badge>
          </CardHeader>
          <BodyText>Find compatible peers on your local network automatically.</BodyText>
          {snapshot?.lan.detail ? <BodyText>{snapshot.lan.detail}</BodyText> : null}
          <Button
            disabled={pending || settings === undefined}
            tone="secondary"
            onPress={() => {
              if (settings) void save({ ...settings, lanEnabled: !settings.lanEnabled });
            }}
          >
            {settings?.lanEnabled
              ? "Disable local network discovery"
              : "Enable local network discovery"}
          </Button>
          {settings?.lanEnabled && canRetry(snapshot?.lan.state) ? (
            <Button disabled={pending} tone="secondary" onPress={() => void retry("lan")}>
              Retry local network discovery
            </Button>
          ) : null}
          {permissionBlocked ||
          (settings?.lanEnabled &&
            (snapshot?.lan.state === Bindings.LocalConnectionState.Unavailable ||
              snapshot?.lan.state === Bindings.LocalConnectionState.Failed)) ? (
            <Button tone="secondary" onPress={() => void Linking.openSettings()}>
              Open local network settings
            </Button>
          ) : null}
          <Disclosure label="Local network details">
            <KeyValue
              label="Saved discovery setting"
              value={
                settings?.lanEnabled
                  ? "Enabled"
                  : settings === undefined
                    ? "Not checked"
                    : "Disabled"
              }
            />
            <KeyValue
              label="Discovery method"
              value={
                snapshot === null || snapshot === undefined
                  ? "Not checked"
                  : lanDiscoveryLabel(snapshot.lan.discoveryMethod)
              }
            />
            <KeyValue
              label="Connected LAN peers"
              value={snapshot?.lan.connectedPeerCount.toString() ?? "Not checked"}
            />
            {snapshot?.lan.stockMulticastAvailable === false ? (
              <BodyText>
                Stock Reticulum multicast discovery is unavailable in this build. Bonjour discovers
                compatible PRNS peers when available.
              </BodyText>
            ) : null}
            <BodyText>LAN discovery and Bluetooth discovery groups are independent.</BodyText>
          </Disclosure>
        </Card>
      ) : null}
      <Card>
        <CardHeader
          title={profileId === undefined ? "Saved TCP peers" : peers[0]?.label || "TCP peer"}
        />
        {saved === null ? (
          <BodyText>Saved connection settings have not been checked.</BodyText>
        ) : null}
        {saved !== null && peers.length === 0 && profileId === undefined ? (
          <BodyText>No TCP peers saved.</BodyText>
        ) : null}
        {saved !== null && peers.length === 0 && profileId !== undefined ? (
          <BodyText>This saved peer is no longer available.</BodyText>
        ) : null}
        {saved !== null &&
        snapshot?.settings.ownerId === saved.ownerId &&
        snapshot.appliedRevision !== saved.revision ? (
          <BodyText>Saved settings are waiting to be applied to the node.</BodyText>
        ) : null}
        {peers.map((peer) => {
          const current =
            snapshot !== null &&
            snapshot !== undefined &&
            saved !== null &&
            snapshot.settings.ownerId === saved.ownerId &&
            snapshot.settings.revision === saved.revision
              ? snapshot.tcpPeers.find((item) => item.profile.id === peer.id)
              : undefined;
          const state =
            current === undefined
              ? peer.enabled
                ? "Waiting for connection status"
                : "Disabled"
              : connectionStateLabel(current.state);
          return (
            <CardSection key={peer.id} title={peer.label || tcpEndpoint(peer)}>
              <Badge>{state}</Badge>
              <BodyText>{tcpEndpoint(peer)}</BodyText>
              {current?.detail ? <BodyText>{current.detail}</BodyText> : null}
              <Disclosure label={`Connection details for ${peer.label || tcpEndpoint(peer)}`}>
                <KeyValue
                  label="Saved settings revision"
                  value={saved?.revision.toString() ?? "Not checked"}
                />
                <KeyValue
                  label="Applied settings revision"
                  value={snapshot?.appliedRevision?.toString() ?? "Not confirmed"}
                />
                <KeyValue label="Received" value={`${current?.rxBytes ?? 0n} bytes`} />
                <KeyValue label="Sent" value={`${current?.txBytes ?? 0n} bytes`} />
                {current?.interfaceId === undefined ? null : (
                  <KeyValue label="Interface ID" value={formatBytes(current.interfaceId)} />
                )}
                <KeyValue label="Saved setting" value={peer.enabled ? "Enabled" : "Disabled"} />
              </Disclosure>
              {editor === peer.id ? (
                <TcpPeerForm
                  key={peer.id}
                  initial={peer}
                  busy={pending}
                  canSave={settings !== undefined}
                  onSave={(draft) => submit(draft, peer)}
                  onCancel={() => setEditor(null)}
                />
              ) : (
                <ActionRow>
                  <Button
                    disabled={pending}
                    tone="secondary"
                    onPress={() =>
                      void changePeers(
                        settings?.tcpPeers.map((item) =>
                          item.id === peer.id ? { ...item, enabled: !item.enabled } : item,
                        ) ?? [],
                      )
                    }
                  >
                    {peer.enabled ? "Disable peer" : "Enable peer"}
                  </Button>
                  <Button disabled={pending} tone="secondary" onPress={() => setEditor(peer.id)}>
                    Edit peer
                  </Button>
                  <Button disabled={pending} tone="secondary" onPress={() => setRemoving(peer.id)}>
                    Remove peer
                  </Button>
                  {peer.enabled && canRetry(current?.state) ? (
                    <Button disabled={pending} tone="secondary" onPress={() => void retry(peer.id)}>
                      Retry peer
                    </Button>
                  ) : null}
                </ActionRow>
              )}
              {removing === peer.id ? (
                <>
                  <BodyText>
                    Remove this saved peer and disconnect it? You can add it again later.
                  </BodyText>
                  <ActionRow>
                    <Button
                      disabled={pending}
                      tone="destructive"
                      onPress={() => {
                        void changePeers(
                          settings?.tcpPeers.filter((item) => item.id !== peer.id) ?? [],
                        ).then((saved) => {
                          if (saved) setRemoving(null);
                        });
                      }}
                    >
                      Remove saved peer
                    </Button>
                    <Button disabled={pending} tone="secondary" onPress={() => setRemoving(null)}>
                      Keep peer
                    </Button>
                  </ActionRow>
                </>
              ) : null}
              {profileId === undefined ? (
                <NavigationLink
                  accessibilityLabel={`Open peer details for ${peer.label}`}
                  href={{
                    pathname: "/more/interfaces/[interfaceId]",
                    params: { interfaceId: peer.id },
                  }}
                >
                  Open peer details
                </NavigationLink>
              ) : null}
            </CardSection>
          );
        })}
        {profileId === undefined ? (
          editor === "new" ? (
            <TcpPeerForm
              busy={pending}
              canSave={settings !== undefined}
              onSave={(draft) => submit(draft)}
              onCancel={() => setEditor(null)}
            />
          ) : (
            <Button
              disabled={pending || settings === undefined || settings.tcpPeers.length >= 16}
              onPress={() => setEditor("new")}
            >
              Add TCP peer
            </Button>
          )
        ) : null}
        {settings?.tcpPeers.length === 16 ? (
          <BodyText>You can save up to 16 TCP peers.</BodyText>
        ) : null}
        {feedback ? <BodyText>{feedback}</BodyText> : null}
        {runtime.connectionFailure ? <BodyText>{runtime.connectionFailure}</BodyText> : null}
        <Button disabled={pending} tone="secondary" onPress={() => void refresh()}>
          {pending ? "Checking connections…" : "Refresh saved connections"}
        </Button>
      </Card>
    </>
  );
}
