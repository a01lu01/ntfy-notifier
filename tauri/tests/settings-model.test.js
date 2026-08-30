import test from "node:test";
import assert from "node:assert/strict";

import {
  HIDDEN_PASSWORD_STATE,
  createSettingsDraft,
  resolveTheme,
  restoreSettingsState
} from "../src/settings-model.js";

const savedConfig = {
  server: "https://ntfy.example.com",
  username: "alice",
  password: "secret",
  topic: "alerts",
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
    themeMode: "light",
    autoStart: true,
    autoCopyOtp: false
  });
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
