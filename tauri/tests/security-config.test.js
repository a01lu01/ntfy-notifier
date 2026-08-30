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
const androidManifest = readFileSync(
  new URL(
    "../src-tauri/gen/android/app/src/main/AndroidManifest.xml",
    import.meta.url
  ),
  "utf8"
);
const legacyBackupRules = readFileSync(
  new URL(
    "../src-tauri/gen/android/app/src/main/res/xml/backup_rules.xml",
    import.meta.url
  ),
  "utf8"
);
const dataExtractionRules = readFileSync(
  new URL(
    "../src-tauri/gen/android/app/src/main/res/xml/data_extraction_rules.xml",
    import.meta.url
  ),
  "utf8"
);
const rustConfig = readFileSync(
  new URL("../src-tauri/src/config.rs", import.meta.url),
  "utf8"
);

const BACKUP_ALLOWLIST = Object.freeze([
  { domain: "root", path: "preferences.json" },
  { domain: "root", path: "ui_state.json" }
]);

function parseAttributes(source) {
  const attributes = {};
  const pattern = /([A-Za-z_][\w:.-]*)\s*=\s*"([^"]*)"/g;
  for (const match of source.matchAll(pattern)) {
    assert.equal(
      Object.hasOwn(attributes, match[1]),
      false,
      `duplicate XML attribute: ${match[1]}`
    );
    attributes[match[1]] = match[2];
  }
  assert.equal(
    source.replace(pattern, "").trim(),
    "",
    "XML attributes must use the expected name=\"value\" form"
  );
  return attributes;
}

function stripXmlPreamble(xml) {
  return xml
    .replace(/<\?xml[\s\S]*?\?>/g, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    .trim();
}

function parseContainer(xml, name) {
  const normalized = stripXmlPreamble(xml);
  const match = normalized.match(
    new RegExp(`^<${name}\\b([^>]*)>([\\s\\S]*)<\\/${name}>$`)
  );
  assert.ok(match, `expected a single <${name}> root element`);
  assert.deepEqual(parseAttributes(match[1]), {});
  return match[2];
}

function parseBackupAllowlist(body) {
  const includePattern = /<include\b([^>]*)\/>/g;
  const includes = [...body.matchAll(includePattern)].map((match) =>
    parseAttributes(match[1])
  );
  assert.equal(
    body.replace(includePattern, "").trim(),
    "",
    "backup section must contain only self-closing include elements"
  );
  return includes;
}

function takeSection(body, name) {
  const pattern = new RegExp(`<${name}\\b([^>]*)>([\\s\\S]*?)<\\/${name}>`);
  const match = body.match(pattern);
  assert.ok(match, `missing <${name}> backup section`);
  assert.deepEqual(parseAttributes(match[1]), {});
  return {
    allowlist: parseBackupAllowlist(match[2]),
    remainder: body.replace(pattern, "")
  };
}

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

test("Android manifest binds explicit backup allowlist resources", () => {
  const applications = [...androidManifest.matchAll(/<application\b([^>]*)>/g)];
  assert.equal(applications.length, 1, "Android manifest must contain one application");
  const attributes = parseAttributes(applications[0][1]);

  assert.equal(attributes["android:allowBackup"], "true");
  assert.equal(attributes["android:fullBackupContent"], "@xml/backup_rules");
  assert.equal(
    attributes["android:dataExtractionRules"],
    "@xml/data_extraction_rules"
  );
});

test("Android 11 and earlier backup rules contain the exact safe allowlist", () => {
  const body = parseContainer(legacyBackupRules, "full-backup-content");

  assert.deepEqual(parseBackupAllowlist(body), BACKUP_ALLOWLIST);
});

test("Android 12+ cloud backup and device transfer use the exact safe allowlist", () => {
  let body = parseContainer(dataExtractionRules, "data-extraction-rules");
  const cloud = takeSection(body, "cloud-backup");
  body = cloud.remainder;
  const transfer = takeSection(body, "device-transfer");

  assert.deepEqual(cloud.allowlist, BACKUP_ALLOWLIST);
  assert.deepEqual(transfer.allowlist, BACKUP_ALLOWLIST);
  assert.equal(
    transfer.remainder.trim(),
    "",
    "data extraction rules must not contain any other backup section"
  );
});

test("backed-up preferences schema contains only the theme", () => {
  const preferences = rustConfig.match(/struct\s+Preferences\s*\{([^}]*)\}/);
  assert.ok(preferences, "Rust Preferences schema must exist");
  const fields = [...preferences[1].matchAll(/^\s*(?:pub\s+)?([A-Za-z_]\w*)\s*:/gm)]
    .map((match) => match[1]);

  assert.deepEqual(fields, ["theme_mode"]);
});
