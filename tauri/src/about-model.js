export const PROJECT_URL = "https://github.com/a01lu01/ntfy-notifier";

export function aboutContent(isMobile, version) {
  if (isMobile) {
    return {
      blurb: "Android 常驻通知工具，订阅 ntfy 消息并弹出系统通知。",
      version: `版本 ${version}（Android）`
    };
  }
  return {
    blurb: "Windows 系统托盘工具，订阅 ntfy 消息并弹出系统通知。",
    version: `版本 ${version}（Rust/Tauri）`
  };
}
