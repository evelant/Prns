import { type DevelopmentNodeSnapshot, DevelopmentNodeRuntime } from "@prns-internal/expo";
import { buildInfo, supportReport } from "./support-report";

const config = {
  name: "prns",
  slug: "prns",
  version: "0.1.0",
  android: { versionCode: 7 },
  extra: { prnsBuild: { variant: "production", label: "alpha-7" } },
};

test("reports recorded build metadata without inventing native or source identity", () => {
  const build = buildInfo(config, "android");
  expect(build).toMatchObject({
    version: "0.1.0",
    configuredBuild: "7",
    buildLabel: "alpha-7",
    variant: "production",
  });
  expect(buildInfo(null, "ios")).toMatchObject({
    buildLabel: "Not recorded",
    configuredBuild: "Not recorded",
    variant: "Not recorded",
  });
  expect(
    buildInfo(
      { ...config, extra: { prnsBuild: { variant: "private-name", label: "private/path\nname" } } },
      "android",
    ),
  ).toMatchObject({ buildLabel: "Not recorded", variant: "Not recorded" });
});

test("exports only explicit status fields, even when nested errors and records contain private data", () => {
  const secret = "DO-NOT-EXPORT-message-contact-identity-key-address";
  const snapshot = {
    runtime: DevelopmentNodeRuntime.Running,
    contractFingerprint: secret,
    primaryIdentity: { tag: "Present", inner: { identityHash: secret } },
    bluetooth: {
      desiredEnabled: true,
      state: { tag: "Connected", inner: { detail: secret } },
      peers: [{ connected: true, name: secret, interfaceId: secret, details: secret }],
    },
    network: {
      state: { tag: "Ready" },
      routes: [{ destination: secret }],
      announces: [{ name: secret }],
    },
    failure: { detail: secret },
    pairedTargets: [{ name: secret }],
    messages: [{ text: secret }],
  } as unknown as DevelopmentNodeSnapshot;
  const report = supportReport(buildInfo(config, "android"), snapshot);
  expect(report).toContain("Node: Running");
  expect(report).toContain("Connected Bluetooth peers: 1");
  expect(report).toContain("Routes: 1");
  expect(report).toContain("Node failure recorded: Yes");
  expect(report).not.toContain(secret);
  const unrecognized = {
    ...snapshot,
    bluetooth: { ...snapshot.bluetooth, state: { tag: secret } },
  } as unknown as DevelopmentNodeSnapshot;
  expect(supportReport(buildInfo(config, "android"), unrecognized)).toContain("Bluetooth: Unknown");
  expect(supportReport(buildInfo(config, "android"), unrecognized)).not.toContain(secret);
});

test("keeps missing observations unavailable rather than inventing a healthy connection", () => {
  const report = supportReport(
    buildInfo(config, "android"),
    null,
    new Date("2026-10-18T12:34:56.000Z"),
  );
  expect(report).toContain("Status checked (UTC): 2026-10-18T12:34:56.000Z");
  expect(report).toContain("Runtime snapshot: Unavailable");
  expect(report).not.toContain("Connected Bluetooth peers: 0");
});
