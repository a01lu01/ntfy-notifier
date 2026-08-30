import test from "node:test";
import assert from "node:assert/strict";

import { aboutContent, PROJECT_URL } from "../src/about-model.js";

test("project URL is the single allowed HTTPS repository address", () => {
  assert.equal(PROJECT_URL, "https://github.com/a01lu01/ntfy-notifier");
});

test("desktop about content describes the Windows tray tool", () => {
  const content = aboutContent(false, "1.1.9");
  assert.match(content.blurb, /Windows 系统托盘工具/);
  assert.match(content.version, /Rust\/Tauri/);
  assert.match(content.version, /1\.1\.9/);
});

test("mobile about content describes the Android persistent notifier", () => {
  const content = aboutContent(true, "1.1.9");
  assert.match(content.blurb, /Android 常驻通知工具/);
  assert.match(content.version, /Android/);
  assert.doesNotMatch(content.version, /Rust\/Tauri/);
  assert.match(content.version, /1\.1\.9/);
});
