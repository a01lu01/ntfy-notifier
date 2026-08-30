import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const tauriConfig = JSON.parse(
  readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")
);
const capability = JSON.parse(
  readFileSync(
    new URL("../src-tauri/capabilities/default.json", import.meta.url),
    "utf8"
  )
);

test("production CSP only permits local scripts and Tauri IPC", () => {
  const csp = tauriConfig.app.security.csp;

  assert.deepEqual(csp["script-src"], ["'self'"]);
  assert.deepEqual(csp["connect-src"], ["ipc:", "http://ipc.localhost"]);
  assert.deepEqual(csp["object-src"], ["'none'"]);
  assert.deepEqual(csp["frame-src"], ["'none'"]);
  assert.deepEqual(csp["frame-ancestors"], ["'none'"]);
  assert.deepEqual(csp["form-action"], ["'none'"]);
  assert.deepEqual(csp["base-uri"], ["'none'"]);
});

test("development CSP permits only the local Vite and HMR endpoints", () => {
  const connectSources = tauriConfig.app.security.devCsp["connect-src"];

  assert.deepEqual(connectSources, [
    "ipc:",
    "http://ipc.localhost",
    "http://localhost:1420",
    "ws://localhost:1420"
  ]);
});

test("WebView capability excludes broad defaults and scopes the opener URL", () => {
  const permissions = capability.permissions;
  const identifiers = permissions.map((permission) =>
    typeof permission === "string" ? permission : permission.identifier
  );

  assert.deepEqual(identifiers, [
    "core:event:allow-listen",
    "core:event:allow-unlisten",
    "dialog:allow-confirm",
    "dialog:allow-message",
    "opener:allow-open-url"
  ]);
  assert.deepEqual(permissions.at(-1).allow, [
    { url: "https://github.com/a01lu01/ntfy-notifier" }
  ]);
});
