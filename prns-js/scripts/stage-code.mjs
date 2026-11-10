import { copyFile } from "node:fs/promises";

await copyFile(
  new URL("./commonjs-package.json", import.meta.url),
  new URL("../dist-cjs/package.json", import.meta.url),
);
