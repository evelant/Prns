import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

type JsonRecord = Readonly<Record<string, unknown>>;

const appRoot = fileURLToPath(new URL("..", import.meta.url));
const bluetoothUsageDescription = "prns uses Bluetooth to connect to nearby Reticulum nodes.";
const localNetworkUsageDescription =
  "prns uses your local network to find nearby Reticulum nodes and connect to saved peers.";

function fail(message: string): never {
  throw new Error(`config:check: ${message}`);
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readRecord(value: unknown, owner: string): JsonRecord {
  if (!isRecord(value)) {
    return fail(`${owner} must be an object`);
  }
  return value;
}

function render(
  variant: "development" | "production",
  environment: Readonly<Record<string, string>> = {},
): JsonRecord {
  const output = execFileSync("expo", ["config", "--type", "public", "--json"], {
    cwd: appRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PRNS_IOS_BUILD_NUMBER: undefined,
      ...environment,
      PRNS_APP_VARIANT: variant,
    },
  });
  const parsed: unknown = JSON.parse(output);
  return readRecord(parsed, `${variant} Expo config`);
}

function assertVariant(
  variant: "development" | "production",
  expected: {
    readonly name: string;
    readonly slug: string;
    readonly identifier: string;
    readonly iosIdentifier: string;
  },
): void {
  const config = render(variant);
  const ios = readRecord(config.ios, `${variant}.ios`);
  const infoPlist = readRecord(ios.infoPlist, `${variant}.ios.infoPlist`);
  const android = readRecord(config.android, `${variant}.android`);
  const web = readRecord(config.web, `${variant}.web`);
  const build = readRecord(
    readRecord(config.extra, `${variant}.extra`).prnsBuild,
    `${variant}.extra.prnsBuild`,
  );
  if (build.variant !== variant) fail(`${variant} must record its build variant`);

  const checks: ReadonlyArray<readonly [string, unknown, string]> = [
    ["name", config.name, expected.name],
    ["slug", config.slug, expected.slug],
    ["version", config.version, "0.1.0"],
    ["ios.bundleIdentifier", ios.bundleIdentifier, expected.iosIdentifier],
    ["ios.scheme", ios.scheme, expected.iosIdentifier],
    ["ios.buildNumber", ios.buildNumber, "1"],
    ["ios.icon", ios.icon, "./assets/icon.png"],
    ["android.package", android.package, expected.identifier],
    ["web.bundler", web.bundler, "metro"],
  ];
  for (const [field, actual, wanted] of checks) {
    if (actual !== wanted) {
      fail(`${variant}.${field} must be ${wanted}, received ${String(actual)}`);
    }
  }
  if (
    !Array.isArray(config.scheme) ||
    config.scheme.length !== 2 ||
    config.scheme[0] !== expected.identifier ||
    config.scheme[1] !== "rns-contact"
  ) {
    fail(`${variant} must support its app scheme and the portable contact-card scheme`);
  }
  const cameraPlugins = Array.isArray(config.plugins)
    ? config.plugins.filter((plugin) => Array.isArray(plugin) && plugin[0] === "expo-camera")
    : [];
  const camera = cameraPlugins[0];
  if (
    cameraPlugins.length !== 1 ||
    !Array.isArray(camera) ||
    !isRecord(camera[1]) ||
    camera[1].cameraPermission !== "prns uses the camera to scan contact QR codes." ||
    camera[1].microphonePermission !== false ||
    camera[1].recordAudioAndroid !== false ||
    camera[1].barcodeScannerEnabled !== true
  ) {
    fail(`${variant} must enable QR scanning with camera permission and no microphone permission`);
  }
  if (infoPlist.NSBluetoothAlwaysUsageDescription !== bluetoothUsageDescription) {
    fail(
      `${variant}.ios.infoPlist.NSBluetoothAlwaysUsageDescription must be the tracked product copy`,
    );
  }
  if (infoPlist.NSLocalNetworkUsageDescription !== localNetworkUsageDescription) {
    fail(
      `${variant}.ios.infoPlist.NSLocalNetworkUsageDescription must cover LAN discovery and saved peers`,
    );
  }
  const bonjour = infoPlist.NSBonjourServices;
  if (
    !Array.isArray(bonjour) ||
    bonjour.length !== 2 ||
    !bonjour.includes("_reticulum._tcp") ||
    !bonjour.includes("_reticulum._udp")
  )
    fail(`${variant}.ios.infoPlist must declare both Reticulum Bonjour service types`);
  const multicastEnabled = variant === "production";
  if (
    readRecord(ios.entitlements, `${variant}.ios.entitlements`)[
      "com.apple.developer.networking.multicast"
    ] !== (multicastEnabled ? true : undefined) ||
    infoPlist.PRNSMulticastEnabled !== multicastEnabled
  )
    fail(`${variant} must keep its multicast entitlement and native discovery policy in sync`);
  const expectedCentral = `${expected.iosIdentifier}.bluetooth-auto.central.v1`;
  if (infoPlist.PRNSCoreBluetoothCentralRestorationIdentifier !== expectedCentral) {
    fail(`${variant}.ios.infoPlist must contain its exact central restoration identifier`);
  }
  if (
    infoPlist.PRNSCoreBluetoothPeripheralRestorationIdentifier !==
    `${expected.iosIdentifier}.bluetooth-auto.peripheral.v1`
  ) {
    fail(`${variant}.ios.infoPlist must contain its exact peripheral restoration identifier`);
  }
  const scenePlugins = Array.isArray(config.plugins)
    ? config.plugins.filter(
        (plugin) => Array.isArray(plugin) && plugin[0] === "expo-build-properties",
      )
    : [];
  const scenePlugin = scenePlugins[0];
  if (
    scenePlugins.length !== 1 ||
    !Array.isArray(scenePlugin) ||
    !isRecord(scenePlugin[1]) ||
    !isRecord(scenePlugin[1].ios) ||
    scenePlugin[1].ios.enableSceneSupport !== true
  ) {
    fail(`${variant} must enable Expo's scene lifecycle support`);
  }
  if (Object.keys(infoPlist).some((key) => key.startsWith("NSAccessorySetup"))) {
    fail(
      `${variant}.ios.infoPlist must use ordinary Bluetooth authorization, not accessory-only access`,
    );
  }
  const backgroundModes = infoPlist.UIBackgroundModes;
  if (
    !Array.isArray(backgroundModes) ||
    backgroundModes.length !== 2 ||
    !backgroundModes.includes("bluetooth-central") ||
    !backgroundModes.includes("bluetooth-peripheral")
  ) {
    fail(`${variant}.ios.infoPlist must declare central and peripheral Bluetooth background modes`);
  }
  if (android.allowBackup !== false) {
    fail(`${variant}.android must disable backups of disposable development state`);
  }
  if (!Array.isArray(config.plugins) || !config.plugins.includes("./tools/with-android-runtime")) {
    fail(`${variant} must apply the Android API 29 runtime plugin`);
  }
}

assertVariant("development", {
  name: "prns dev",
  slug: "prns-dev",
  identifier: "rs.reticulum.prns.dev",
  iosIdentifier: "rs.reticulum.prns.dev",
});
assertVariant("production", {
  name: "prns",
  slug: "prns",
  identifier: "rs.reticulum.prns",
  iosIdentifier: "rs.reticulum.prns.chat",
});

for (const variant of ["development", "production"] as const) {
  const fixture = "127.0.0.1:4242";
  const fixtureConfig = render(variant, { EXPO_PUBLIC_PRNS_LXMF_TCP_TARGET: fixture });
  const ios = readRecord(fixtureConfig.ios, `${variant}.ios`);
  const info = readRecord(ios.infoPlist, `${variant}.ios.infoPlist`);
  if (info.PRNSDevelopmentTcpTarget !== (variant === "development" ? fixture : undefined)) {
    fail(`${variant} must include the native TCP fixture only in the development variant`);
  }
}

const taggedBuild = render("production", { PRNS_BUILD_ID: "alpha.20261018-1" });
if (
  readRecord(
    readRecord(taggedBuild.extra, "production.extra").prnsBuild,
    "production.extra.prnsBuild",
  ).label !== "alpha.20261018-1"
) {
  fail("the public build label must survive into the embedded config");
}
const untaggedBuild = render("production", { PRNS_BUILD_ID: "" });
if (
  readRecord(
    readRecord(untaggedBuild.extra, "production.extra").prnsBuild,
    "production.extra.prnsBuild",
  ).label !== undefined
) {
  fail("an unrecorded build label must remain explicitly absent");
}
const invalidLabel = spawnSync("expo", ["config", "--type", "public", "--json"], {
  cwd: appRoot,
  encoding: "utf8",
  env: { ...process.env, PRNS_APP_VARIANT: "production", PRNS_BUILD_ID: "private/path\nlabel" },
});
if (invalidLabel.status === 0) fail("build labels with paths or newlines must be rejected");

const noFixture = render("development", { EXPO_PUBLIC_PRNS_LXMF_TCP_TARGET: "" });
if (
  readRecord(readRecord(noFixture.ios, "development.ios").infoPlist, "development.ios.infoPlist")
    .PRNSDevelopmentTcpTarget !== undefined
) {
  fail("an unset native TCP fixture must remain absent");
}

for (const variant of ["development", "production"] as const) {
  const config = render(variant, { PRNS_IOS_BUILD_NUMBER: "37" });
  if (readRecord(config.ios, `${variant}.ios`).buildNumber !== "37") {
    fail(`${variant} must use the requested iOS build number`);
  }
}

for (const buildNumber of ["", "0", "-1", "1.5", "2e3", "01", "abc", "9007199254740992"]) {
  const invalidBuild = spawnSync("expo", ["config", "--type", "public", "--json"], {
    cwd: appRoot,
    encoding: "utf8",
    env: { ...process.env, PRNS_APP_VARIANT: "production", PRNS_IOS_BUILD_NUMBER: buildNumber },
  });
  if (
    invalidBuild.status === 0 ||
    !`${invalidBuild.stdout}${invalidBuild.stderr}`.includes("PRNS_IOS_BUILD_NUMBER")
  ) {
    fail(`invalid iOS build number ${JSON.stringify(buildNumber)} must be rejected explicitly`);
  }
}

const invalid = spawnSync("expo", ["config", "--type", "public", "--json"], {
  cwd: appRoot,
  encoding: "utf8",
  env: { ...process.env, PRNS_APP_VARIANT: "preview" },
});
if (invalid.status === 0) {
  fail("PRNS_APP_VARIANT=preview must be rejected");
}

console.log(
  "config:check: variant coordinates, iOS release metadata, dual-role Bluetooth restoration, and Android runtime policy are exact",
);
