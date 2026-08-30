export const HIDDEN_PASSWORD_STATE = Object.freeze({
  inputType: "password",
  toggleLabel: "显示"
});

export function createSettingsDraft(config = {}) {
  return {
    server: config.server || "",
    username: config.username || "",
    password: config.password || "",
    topic: config.topic || "",
    themeMode: config.theme_mode || "system",
    autoStart: !!config.auto_start,
    autoCopyOtp: !!config.auto_copy_otp
  };
}

export function resolveTheme(mode, prefersDark = false) {
  return mode === "dark" || (mode === "system" && prefersDark)
    ? "dark"
    : "light";
}

export function restoreSettingsState(config, prefersDark = false) {
  const draft = createSettingsDraft(config);
  return {
    draft,
    resolvedTheme: resolveTheme(draft.themeMode, prefersDark),
    password: { ...HIDDEN_PASSWORD_STATE }
  };
}
