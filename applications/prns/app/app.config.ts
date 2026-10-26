import type { ConfigContext, ExpoConfig } from "expo/config";

const variants = {
  development: {
    name: "prns dev",
    slug: "prns-dev",
    identifier: "rs.reticulum.prns.dev",
    iosIdentifier: "rs.reticulum.prns.dev",
  },
  production: {
    name: "prns",
    slug: "prns",
    identifier: "rs.reticulum.prns",
    iosIdentifier: "rs.reticulum.prns.chat",
  },
} as const;

type AppVariant = keyof typeof variants;

function selectedVariant(value: string | undefined): AppVariant {
  if (value === undefined || value === "development") {
    return "development";
  }
  if (value === "production") {
    return value;
  }
  throw new Error(
    `PRNS_APP_VARIANT must be development or production, received ${JSON.stringify(value)}`,
  );
}

function selectedIosBuildNumber(value: string | undefined): string {
  const buildNumber = value?.trim() ?? "1";
  if (!/^[1-9][0-9]*$/.test(buildNumber) || !Number.isSafeInteger(Number(buildNumber))) {
    throw new Error("PRNS_IOS_BUILD_NUMBER must be a positive decimal safe integer");
  }
  return buildNumber;
}

export default ({ config }: ConfigContext): ExpoConfig => {
  const variant = selectedVariant(process.env.PRNS_APP_VARIANT);
  const selection = variants[variant];
  const iosBuildNumber = selectedIosBuildNumber(process.env.PRNS_IOS_BUILD_NUMBER);
  const buildLabel = process.env.PRNS_BUILD_ID?.trim();
  if (buildLabel && !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(buildLabel)) {
    throw new Error(
      "PRNS_BUILD_ID must be a public build label of 1–64 letters, numbers, dots, underscores or hyphens",
    );
  }
  const developmentTcpTarget =
    variant === "development" ? process.env.EXPO_PUBLIC_PRNS_LXMF_TCP_TARGET?.trim() : undefined;
  const bluetoothRestorationPrefix = `${selection.iosIdentifier}.bluetooth-auto`;

  return {
    ...config,
    name: selection.name,
    slug: selection.slug,
    scheme: [selection.identifier, "rns-contact"],
    version: "0.1.0",
    runtimeVersion: { policy: "fingerprint" },
    extra: {
      ...config.extra,
      prnsBuild: { variant, ...(buildLabel ? { label: buildLabel } : {}) },
    },
    orientation: "default",
    userInterfaceStyle: "automatic",
    plugins: [
      "expo-router",
      [
        "expo-camera",
        {
          cameraPermission: "prns uses the camera to scan contact QR codes.",
          microphonePermission: false,
          recordAudioAndroid: false,
          barcodeScannerEnabled: true,
        },
      ],
      ["expo-build-properties", { ios: { enableSceneSupport: true } }],
      "./tools/with-ios-18",
      "./tools/with-android-runtime",
    ],
    ios: {
      ...config.ios,
      bundleIdentifier: selection.iosIdentifier,
      scheme: selection.iosIdentifier,
      buildNumber: iosBuildNumber,
      icon: "./assets/icon.png",
      infoPlist: {
        ...config.ios?.infoPlist,
        ...(developmentTcpTarget ? { PRNSDevelopmentTcpTarget: developmentTcpTarget } : {}),
        NSBluetoothAlwaysUsageDescription:
          "prns uses Bluetooth to connect to nearby Reticulum nodes.",
        NSLocalNetworkUsageDescription:
          "prns uses your local network to find nearby Reticulum nodes and connect to saved peers.",
        NSBonjourServices: ["_reticulum._tcp", "_reticulum._udp"],
        PRNSCoreBluetoothCentralRestorationIdentifier: `${bluetoothRestorationPrefix}.central.v1`,
        PRNSCoreBluetoothPeripheralRestorationIdentifier: `${bluetoothRestorationPrefix}.peripheral.v1`,
        UIBackgroundModes: ["bluetooth-central", "bluetooth-peripheral"],
      },
      supportsTablet: true,
    },
    android: {
      ...config.android,
      package: selection.identifier,
      allowBackup: false,
    },
    web: {
      ...config.web,
      bundler: "metro",
    },
  };
};
