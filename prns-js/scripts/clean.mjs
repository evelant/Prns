import { rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

await Promise.all(
  ["dist", "dist-cjs", "native", "wasm", "LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md", "SDK-CONTENTS.sha256"].map(
    (path) => rm(resolve(packageRoot, path), { force: true, recursive: true }),
  ),
);
