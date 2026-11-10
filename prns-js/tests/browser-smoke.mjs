import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { findChromium } from "./chromium.mjs";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageRoot, "..");
const browserTimeoutMs = process.env.CI ? 60_000 : 20_000;
const fixturePath = "/prns-js/tests/browser-auto-consumer.html";
const chromium = findChromium();
assert.ok(chromium, "Chromium is required for the browser package smoke");

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".wasm", "application/wasm"],
]);
let settleBrowserResult;
const browserResult = new Promise((resolveResult) => {
  settleBrowserResult = resolveResult;
});
let activeLaunch;
let browserProgress = "fixture has not reported progress";
const workerStatusSockets = new Set();
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (request.method === "GET" && url.pathname === "/browser-smoke-launch") {
      // Chromium must execute a script and navigate before the functional smoke starts.
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(`<script>location.replace("${fixturePath}" + location.search)</script>`);
      return;
    }
    if (request.method === "GET" && url.pathname === fixturePath) {
      if (!activeLaunch || url.searchParams.get("launch") !== activeLaunch.id) {
        response.writeHead(410);
        response.end();
        return;
      }
      activeLaunch.ready();
    }
    if (
      request.method === "POST" &&
      url.pathname === "/browser-smoke-progress"
    ) {
      const chunks = [];
      for await (const chunk of request) {
        chunks.push(chunk);
      }
      browserProgress = Buffer.concat(chunks).toString("utf8");
      response.writeHead(204);
      response.end();
      return;
    }
    if (
      request.method === "POST" &&
      url.pathname === "/browser-smoke-result"
    ) {
      const chunks = [];
      for await (const chunk of request) {
        chunks.push(chunk);
      }
      const result = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      response.writeHead(204);
      response.end();
      settleBrowserResult(result);
      return;
    }
    if (
      request.method === "POST" &&
      url.pathname === "/close-worker-status-sockets"
    ) {
      response.writeHead(204);
      response.end();
      for (const socket of workerStatusSockets) {
        socket.end(Buffer.from([0x88, 0x00]));
      }
      return;
    }
    const path = resolve(repositoryRoot, `.${decodeURIComponent(url.pathname)}`);
    const metadata = await stat(path);
    assert.ok(path.startsWith(`${repositoryRoot}/`) && metadata.isFile());
    response.writeHead(200, {
      "content-type":
        contentTypes.get(extname(path)) ?? "application/octet-stream",
    });
    response.end(await readFile(path));
  } catch {
    response.writeHead(404);
    response.end();
  }
});
server.on("upgrade", (request, socket) => {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");
  const key = request.headers["sec-websocket-key"];
  if (url.pathname !== "/worker-status-socket" || typeof key !== "string") {
    socket.destroy();
    return;
  }
  const accept = createHash("sha1")
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest("base64");
  socket.write([
    "HTTP/1.1 101 Switching Protocols",
    "Upgrade: websocket",
    "Connection: Upgrade",
    `Sec-WebSocket-Accept: ${accept}`,
    "",
    "",
  ].join("\r\n"));
  workerStatusSockets.add(socket);
  socket.once("close", () => workerStatusSockets.delete(socket));
});

await new Promise((resolveListening) => {
  server.listen(0, "127.0.0.1", resolveListening);
});

const timedOut = Symbol("browser timeout");
async function waitForBrowser(report, exited) {
  let timer;
  try {
    return await Promise.race([
      report,
      exited,
      new Promise((resolveTimeout) => {
        timer = setTimeout(() => resolveTimeout(timedOut), browserTimeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function runBrowser(port) {
  for (let attempt = 1; attempt <= 2; attempt++) {
    const id = String(attempt);
    const profile = await mkdtemp(join(tmpdir(), "prns-browser-smoke-"));
    const ready = new Promise((resolveReady) => {
      activeLaunch = { id, ready: resolveReady };
    });
    const browser = spawn(chromium, [
      "--headless=new",
      "--no-sandbox",
      "--disable-dev-shm-usage",
      "--disable-gpu",
      "--enable-logging=stderr",
      `--user-data-dir=${profile}`,
      `http://127.0.0.1:${port}/browser-smoke-launch?launch=${id}`,
    ], { stdio: ["ignore", "pipe", "pipe"] });
    let diagnostics = "";
    const retainDiagnostics = (chunk) => {
      diagnostics = `${diagnostics}${chunk}`.slice(-16_384);
    };
    browser.stdout.on("data", retainDiagnostics);
    browser.stderr.on("data", retainDiagnostics);
    const closed = new Promise((resolveClosed) => browser.once("close", resolveClosed));
    const exited = new Promise((_, rejectExit) => {
      browser.once("error", rejectExit);
      browser.once("exit", (code, signal) => rejectExit(new Error(
        `Chromium exited before reporting a result: code=${code} signal=${signal}`,
      )));
    });
    try {
      if (await waitForBrowser(ready, exited) === timedOut) {
        const message = `Chromium launch ${attempt} did not navigate to the fixture ` +
          `within ${browserTimeoutMs}ms. Chromium output:\n${diagnostics || "(none)"}`;
        if (attempt === 1) {
          console.warn(`${message}\nRetrying browser startup with a fresh profile.`);
          continue;
        }
        throw new Error(message);
      }
      const result = await waitForBrowser(browserResult, exited);
      if (result === timedOut) {
        throw new Error(`browser smoke timed out after ${browserTimeoutMs}ms; ` +
          `last progress: ${browserProgress}; Chromium output:\n${diagnostics || "(none)"}`);
      }
      return result;
    } finally {
      activeLaunch = undefined;
      browser.kill("SIGTERM");
      const forceKill = setTimeout(() => browser.kill("SIGKILL"), 2_000);
      try {
        await closed;
      } finally {
        clearTimeout(forceKill);
        await rm(profile, { recursive: true, force: true, maxRetries: 3 });
      }
    }
  }
}

try {
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const result = await runBrowser(address.port);
  assert.deepEqual(result, {
    title: "PASS",
    execution: "DedicatedWorker",
    outcome: "Ready",
    command: "Failed:UnknownLink",
    resource: "Failed:UnknownLink",
    blob: "Failed:UnknownLink",
    snapshot: "Consistent",
    persistence: "Restored",
    persistenceFailures: "Typed",
    routePersistence: "Restored",
    webSocketFraming: "Resolved",
    webSocketWorkerStatus: "PushedOnce",
    bluetoothContract: "Shared",
    bluetoothSession: "Bridged",
    bluetoothWorker: "Bridged",
    parallelCrypto: "Verified",
    projection: "Reactive",
    admission: "RuntimeRejected:worker-admission",
    autoWifiAdmission: "Retained",
    stop: "Idempotent",
    compression: "Compressed",
    compressionDetail: "message:message",
  });
} finally {
  for (const socket of workerStatusSockets) {
    socket.destroy();
  }
  await new Promise((resolveClosed, rejectClosed) => {
    server.close((error) => {
      if (error) {
        rejectClosed(error);
      } else {
        resolveClosed();
      }
    });
  });
}
