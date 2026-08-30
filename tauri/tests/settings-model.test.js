import test from "node:test";
import assert from "node:assert/strict";

import {
  HIDDEN_PASSWORD_STATE,
  createSettingsDraft,
  getInsecureHttpSaveAction,
  isRemoteInsecureHttp,
  resolveTheme,
  restoreSettingsState
} from "../src/settings-model.js";

const savedConfig = {
  server: "https://ntfy.example.com",
  username: "alice",
  password: "secret",
  topic: "alerts",
  allow_insecure_http: true,
  theme_mode: "light",
  auto_start: true,
  auto_copy_otp: false
};

test("settings draft is recreated from saved config", () => {
  const first = createSettingsDraft(savedConfig);
  first.server = "https://draft.example.com";
  first.themeMode = "dark";

  assert.deepEqual(createSettingsDraft(savedConfig), {
    server: "https://ntfy.example.com",
    username: "alice",
    password: "secret",
    topic: "alerts",
    allowInsecureHttp: true,
    themeMode: "light",
    autoStart: true,
    autoCopyOtp: false
  });
});

test("settings draft defaults insecure remote HTTP opt-in to disabled", () => {
  assert.equal(createSettingsDraft({}).allowInsecureHttp, false);
});

test("remote HTTP detection accepts only loopback HTTP without opt-in", () => {
  for (const server of [
    "http://localhost/topic",
    "HTTP://LOCALHOST/topic",
    "http://127.0.0.1/topic",
    "http://127.255.10.8/topic",
    "http://127.1/topic",
    "http://[::1]/topic"
  ]) {
    assert.equal(isRemoteInsecureHttp(server), false, server);
  }

  for (const server of [
    "http://localhost.evil/topic",
    "http://127.evil/topic",
    "http://192.168.1.20/topic",
    "http://10.0.0.8/topic",
    "http://ntfy.example.com/topic",
    "HTTP://NTFY.EXAMPLE.COM/topic",
    "http://[2001:db8::1]/topic"
  ]) {
    assert.equal(isRemoteInsecureHttp(server), true, server);
  }
});

test("HTTPS and invalid URLs are left to backend validation", () => {
  for (const server of [
    "https://ntfy.example.com/topic",
    "HTTPS://LOCALHOST/topic",
    "not a url",
    "",
    "ftp://ntfy.example.com/topic"
  ]) {
    assert.equal(isRemoteInsecureHttp(server), false, server);
  }
});

test("remote HTTP save decision fails closed and always confirms opt-in", () => {
  assert.equal(
    getInsecureHttpSaveAction("http://ntfy.example.com", false),
    "blocked"
  );
  assert.equal(
    getInsecureHttpSaveAction("HTTP://NTFY.EXAMPLE.COM", true),
    "confirm"
  );
  assert.equal(
    getInsecureHttpSaveAction("http://127.0.0.1:8080", false),
    "safe"
  );
  assert.equal(
    getInsecureHttpSaveAction("https://ntfy.example.com", false),
    "safe"
  );
});

test("theme rollback resolves the saved theme instead of the preview", () => {
  assert.equal(resolveTheme("light", true), "light");
  assert.equal(resolveTheme("dark", false), "dark");
  assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("system", false), "light");
});

test("restoring settings always hides the password again", () => {
  const restored = restoreSettingsState(savedConfig, true);

  assert.equal(restored.resolvedTheme, "light");
  assert.deepEqual(restored.password, HIDDEN_PASSWORD_STATE);
  assert.notStrictEqual(restored.password, HIDDEN_PASSWORD_STATE);
});
