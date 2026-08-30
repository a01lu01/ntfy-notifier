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
    allowInsecureHttp: !!config.allow_insecure_http,
    themeMode: config.theme_mode || "system",
    autoStart: !!config.auto_start,
    autoCopyOtp: !!config.auto_copy_otp
  };
}

export function isRemoteInsecureHttp(server) {
  let url;
  try {
    url = new URL(server);
  } catch {
    return false;
  }

  if (url.protocol !== "http:") return false;

  const hostname = url.hostname.toLowerCase();
  if (hostname === "localhost" || hostname === "[::1]" || hostname === "::1") {
    return false;
  }

  const octets = hostname.split(".");
  const isIpv4Loopback = octets.length === 4
    && octets.every((octet) => /^\d+$/.test(octet) && Number(octet) <= 255)
    && Number(octets[0]) === 127;

  return !isIpv4Loopback;
}

export function getInsecureHttpSaveAction(server, allowInsecureHttp) {
  if (!isRemoteInsecureHttp(server)) return "safe";
  return allowInsecureHttp ? "confirm" : "blocked";
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
