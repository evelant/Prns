import type { DevelopmentNodeSnapshot } from "@prns-internal/expo";
import Constants from "expo-constants";
import { useCallback, useEffect, useRef, useState } from "react";
import { Share, Text } from "react-native";

import { useDevelopmentRuntime } from "@/native/development-runtime-context";
import { runtimeProvider } from "@/native/runtime-provider";
import { Disclosure } from "@/ui/disclosure";
import { Badge, BodyText, Button, Card, KeyValue, Screen, ScreenHeading } from "@/ui/primitives";
import { useAppPalette } from "@/ui/theme";
import { buildContract, buildInfo, type BuildInfo, supportReport } from "./support-report";

export function AboutScreen() {
  const { availability } = runtimeProvider;
  const build = buildInfo(Constants.expoConfig, availability.platform);
  return (
    <Screen>
      <ScreenHeading>About prns</ScreenHeading>
      <Badge>{availability.type === "available" ? "Maintainer alpha" : "Web preview"}</Badge>
      <BodyText>Direct and group messaging, node management and network inspection.</BodyText>
      <BodyText muted>{`Version ${build.version}`}</BodyText>
      {availability.type === "unavailable" ? (
        <BodyText>
          Messaging and node connections are available in the iOS and Android apps.
        </BodyText>
      ) : null}
      <Card>
        <Disclosure label="Build details">
          <KeyValue label="Application" value={build.application} />
          <KeyValue label="Configured build" value={build.configuredBuild} />
          <KeyValue label="Build label" value={build.buildLabel} />
          <KeyValue label="Variant" value={build.variant} />
          <KeyValue label="Platform" value={build.platform} />
          <KeyValue label="App contract" value={buildContract} />
          <BodyText muted>
            A build label identifies a packaged build when recorded. The contract identifies its
            API, not its source revision.
          </BodyText>
        </Disclosure>
      </Card>
      <Card>
        <Disclosure label="Diagnostics">
          {availability.type === "available" ? (
            <Diagnostics build={build} />
          ) : (
            <BodyText>Diagnostic sharing is available in the iOS and Android apps.</BodyText>
          )}
        </Disclosure>
      </Card>
    </Screen>
  );
}

function Diagnostics({ build }: { readonly build: BuildInfo }) {
  const runtime = useDevelopmentRuntime();
  const palette = useAppPalette();
  const [snapshot, setSnapshot] = useState<DevelopmentNodeSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [checkedAt, setCheckedAt] = useState<Date | null>(null);
  const [sharing, setSharing] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const alive = useRef(false);
  const busy = useRef(false);
  const shareBusy = useRef(false);
  const refreshSnapshot = useRef(runtime.refreshSnapshot);
  refreshSnapshot.current = runtime.refreshSnapshot;

  const refresh = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    setLoading(true);
    setFailure(null);
    try {
      const result = await refreshSnapshot.current();
      if (alive.current) {
        setSnapshot(result.type === "outcome" ? result.outcome : null);
        setCheckedAt(new Date());
      }
    } catch {
      if (alive.current) {
        setSnapshot(null);
        setCheckedAt(new Date());
      }
    } finally {
      busy.current = false;
      if (alive.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    alive.current = true;
    void refresh();
    return () => {
      alive.current = false;
    };
  }, [refresh]);

  const report = supportReport(build, snapshot, checkedAt);
  const share = async () => {
    if (loading || shareBusy.current) return;
    shareBusy.current = true;
    setSharing(true);
    setFailure(null);
    try {
      await Share.share({ message: report, title: "prns diagnostics" });
    } catch {
      if (alive.current)
        setFailure("Diagnostics could not be shared. Try again or copy the text below.");
    } finally {
      shareBusy.current = false;
      if (alive.current) setSharing(false);
    }
  };

  return (
    <>
      <BodyText muted>
        Review before sharing. This summary contains build information and connection status,
        without message content, contact names, addresses, identity IDs or keys.
      </BodyText>
      {loading ? (
        <BodyText>Reading status…</BodyText>
      ) : (
        <Text selectable style={{ color: palette.text, fontSize: 13, lineHeight: 20 }}>
          {report}
        </Text>
      )}
      <Button disabled={loading || sharing} onPress={() => void share()}>
        {sharing ? "Opening share…" : "Share diagnostics"}
      </Button>
      <Button disabled={loading || sharing} tone="secondary" onPress={() => void refresh()}>
        Refresh diagnostics
      </Button>
      {failure === null ? null : <BodyText>{failure}</BodyText>}
    </>
  );
}
