import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const checksumFile = "SDK-CONTENTS.sha256";
const manifest = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
const sourceCheckout = existsSync(resolve(root, "src"));

if (sourceCheckout) {
  execFileSync("python3", ["../tools/release/verify-js-sdk-pack.py", ".", "--prepare-facade"], {
    cwd: root,
    stdio: "inherit",
  });
}

// npm installs only the distributable files. Repacking that directory must
// validate the previously prepared bytes without trusting sibling packages.
const digest = createHash("sha256");
function addFile(name) {
  const path = resolve(root, name);
  const stat = lstatSync(path);
  if (stat.isDirectory()) {
    for (const child of readdirSync(path).sort()) addFile(`${name}/${child}`);
    return;
  }
  if (!stat.isFile()) throw new Error(`Unexpected SDK release entry: ${name}`);
  const hash = createHash("sha256").update(readFileSync(path)).digest("hex");
  digest.update(`${hash}  ${name}\n`, "utf8");
}
for (const name of ["package.json", ...manifest.files].sort()) {
  if (name.split("/").some((part) => !part || part === "." || part === ".." || /[\\:]/.test(part))) {
    throw new Error(`Unexpected SDK package file path: ${name}`);
  }
  if (name !== checksumFile) addFile(name);
}
const checksum = digest.digest("hex") + "\n";
const path = resolve(root, checksumFile);
if (sourceCheckout) {
  writeFileSync(path, checksum);
} else if (readFileSync(path, "utf8") !== checksum) {
  throw new Error("The packed SDK contents or third-party notices differ from the prepared release.");
}
