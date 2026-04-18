"""
ntfy-Notifier 主程序
监听 ntfy 消息并弹出 Windows 原生通知
遵循 Fluent Design 视觉风格
"""

import sys
import threading
import time
import traceback
from threading import Event

from src.config import load_config, save_config
from src.notifier import fetch_ntfy_messages, send_toast
from src.tray import TrayIcon

# ── 全局状态 ────────────────────────────────────────────────────────────────

_config = {}
_polling_thread: threading.Thread | None = None
_running = True
_connected = False
_tray: TrayIcon | None = None
_root: "tk.Tk | None" = None


def _set_auto_start(enabled: bool):
    """设置 Windows 开机自启动（通过注册表）。"""
    try:
        import winreg
        key = winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            0, winreg.KEY_SET_VALUE,
        )
        if enabled:
            exe_path = sys.executable
            winreg.SetValueEx(key, "ntfy-Notifier", 0, winreg.REG_SZ, f'"{exe_path}"')
        else:
            try:
                winreg.DeleteValue(key, "ntfy-Notifier")
            except FileNotFoundError:
                pass
        winreg.CloseKey(key)
    except Exception:
        traceback.print_exc()


def _open_settings():
    """在主 Tk 线程中弹出设置窗口（非阻塞，通过 after 调度）。"""
    if _root is None:
        return

    def on_save(cfg: dict):
        global _config
        save_config(cfg)
        _config = cfg
        _set_auto_start(cfg.get("auto_start", False))

    from src.ui import SettingsWindow
    win = SettingsWindow(_config, on_save=on_save, on_cancel=None, master=_root)
    # 立即显示（Tk 已运行，窗口会立即出现）
    win.show()


def main():
    global _root, _config, _tray, _running

    import tkinter as tk

    _config = load_config()

    # 单例 Tk root（始终存在，隐藏）
    _root = tk.Tk()
    _root.withdraw()
    # 拦截关闭，防止 root 被意外销毁
    _root.protocol("WM_DELETE_WINDOW", lambda: None)

    # 首次运行无配置 → 在 mainloop 启动后立即弹出设置
    if not _config.get("server") or not _config.get("topic"):
        _root.after(100, _open_settings)

    if _config.get("auto_start"):
        _set_auto_start(True)

    # 启动托盘（此时 Tk 已在运行）
    _tray = TrayIcon(on_settings=_open_settings, on_quit=_quit)
    _tray.start(connected=False)

    # 启动轮询线程
    _polling_thread = threading.Thread(target=_poll_loop, daemon=True, name="PollThread")
    _polling_thread.start()

    # 主 Tk 线程：永不退出
    _root.mainloop()
    _quit()


def _quit():
    global _running, _tray
    _running = False
    if _tray:
        _tray.stop()
    if _root:
        try:
            _root.quit()
            _root.destroy()
        except Exception:
            pass
    sys.exit(0)


def _poll_loop():
    global _connected, _running, _config, _tray

    seen_ids: set[str] = set()

    while _running:
        cfg = _config
        server = cfg.get("server", "")
        topic = cfg.get("topic", "")
        username = cfg.get("username", "")
        password = cfg.get("password", "")

        if not server or not topic:
            if _connected:
                _connected = False
                if _tray:
                    _tray.update(False)
            time.sleep(cfg.get("poll_interval", 3))
            continue

        try:
            messages = fetch_ntfy_messages(server, topic, username, password)
            if not _connected:
                _connected = True
                if _tray:
                    _tray.update(True)

            for msg in messages:
                msg_id = str(msg.get("id", msg.get("time", "")))
                if msg_id not in seen_ids:
                    seen_ids.add(msg_id)
                    title = msg.get("title") or "ntfy 消息"
                    message = msg.get("message") or str(msg)
                    send_toast(title, message, app_id="ntfy-Notifier")
        except Exception:
            if _connected:
                _connected = False
                if _tray:
                    _tray.update(False)
            traceback.print_exc()

        time.sleep(cfg.get("poll_interval", 3))


if __name__ == "__main__":
    main()
