import {
  type DevelopmentNodeSnapshot,
  DevelopmentNodeRuntime,
  HOST_CONTRACT_FINGERPRINT,
  NATIVE_CONTRACT_FINGERPRINT,
} from "@prns-internal/expo";
import type { ExpoConfig } from "expo/config";

export type BuildInfo = {
  readonly application: string;
  readonly version: string;
  readonly configuredBuild: string;
  readonly buildLabel: string;
  readonly variant: string;
  readonly platform: "ios" | "android" | "web";
};

/** Public build metadata, never a device/session identifier or a guessed source hash. */
export function buildInfo(config: ExpoConfig | null, platform: BuildInfo["platform"]): BuildInfo {
  const value: unknown = config?.extra?.prnsBuild;
  const recorded = typeof value === "object" && value !== null ? value : {};
  const variant = "variant" in recorded ? recorded.variant : undefined;
  const label = "label" in recorded ? recorded.label : undefined;
  const configuredBuild =
    platform === "ios"
      ? config?.ios?.buildNumber
      : platform === "android"
        ? config?.android?.versionCode
        : undefined;
  return {
    application: config?.name ?? "prns",
    version: config?.version ?? "Not recorded",
    configuredBuild: configuredBuild === undefined ? "Not recorded" : String(configuredBuild),
    buildLabel:
      typeof label === "string" && /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(label)
        ? label
        : "Not recorded",
    variant: variant === "production" || variant === "development" ? variant : "Not recorded",
    platform,
  };
}

export const buildContract = NATIVE_CONTRACT_FINGERPRINT;

function known(value: unknown, choices: readonly string[]): string {
  return typeof value === "string" && choices.includes(value) ? value : "Unknown";
}

function runtimeState(snapshot: DevelopmentNodeSnapshot): string {
  switch (snapshot.runtime) {
    case DevelopmentNodeRuntime.Stopped:
      return "Stopped";
    case DevelopmentNodeRuntime.Starting:
      return "Starting";
    case DevelopmentNodeRuntime.Running:
      return "Running";
    case DevelopmentNodeRuntime.Stopping:
      return "Stopping";
    case DevelopmentNodeRuntime.Failed:
      return "Failed";
    default:
      return "Unknown";
  }
}

/** Explicit allowlist: do not serialize snapshots, errors, peers or identity state. */
export function supportReport(
  build: BuildInfo,
  snapshot: DevelopmentNodeSnapshot | null,
  checkedAt: Date | null = null,
): string {
  const lines = [
    "prns diagnostic summary · format 1",
    `Application: ${build.application}`,
    `Version: ${build.version}`,
    `Configured build: ${build.configuredBuild}`,
    `Build label: ${build.buildLabel}`,
    `Variant: ${build.variant}`,
    `Platform: ${build.platform}`,
    `App contract: ${NATIVE_CONTRACT_FINGERPRINT}`,
    `Host contract: ${HOST_CONTRACT_FINGERPRINT}`,
    "",
    `Status checked (UTC): ${checkedAt?.toISOString() ?? "Not recorded"}`,
  ];
  if (snapshot === null) {
    lines.push("Runtime snapshot: Unavailable");
  } else {
    lines.push(
      `Node: ${runtimeState(snapshot)}`,
      `Bluetooth enabled: ${snapshot.bluetooth.desiredEnabled === undefined ? "Unknown" : snapshot.bluetooth.desiredEnabled ? "Yes" : "No"}`,
      `Bluetooth: ${known(snapshot.bluetooth.state.tag, ["Stopped", "Starting", "Disabled", "Disabling", "Connecting", "WaitingForPeers", "Connected", "RadioOff", "Unavailable"])}`,
      `Connected Bluetooth peers: ${snapshot.bluetooth.peers.filter((peer) => peer.connected).length}`,
      `Network: ${known(snapshot.network.state.tag, ["Stopped", "Starting", "Ready", "Unavailable"])}`,
      `Routes: ${snapshot.network.routes.length}`,
      `Node failure recorded: ${snapshot.failure === undefined ? "No" : "Yes"}`,
    );
  }
  lines.push(
    "",
    "Status was observed when this report was refreshed; it is not delivery evidence.",
  );
  return lines.join("\n");
}
