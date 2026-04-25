"""
ntfy-Notifier 主程序
监听 ntfy 消息并弹出 Windows 原生通知
遵循 Fluent Design 视觉风格

订阅模式：SSE (Server-Sent Events) — 实时推送，无需轮询
"""

import sys
import threading
import time
import traceback
from typing import Optional
from threading import Event

import requests

from src.config import load_config, save_config
from src.notifier import send_toast, NtfySSESubscriber
from src.tray import TrayIcon

# ── 全局状态 ────────────────────────────────────────────────────────────────

_config = {}
_subscriber: Optional[NtfySSESubscriber] = None
_running = True
_connected = False
_tray: Optional[TrayIcon] = None
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
    """在主 Tk 线程中弹出设置窗口（通过 after 调度，避免 pystray 子线程操作 Tk）。"""
    if _root is None:
        return

    def on_save(cfg: dict):
        global _config, _subscriber
        
        save_config(cfg)
        _config = cfg
        _set_auto_start(cfg.get("auto_start", False))
        
        # 重新连接 SSE（如果之前有连接）
        # 在后台线程中执行 stop + restart，避免 join 阻塞 Tk 主线程
        if _subscriber and _connected:
            print("[ntfy] 配置已更新，后台重连 SSE...", file=sys.stderr)
            
            def _reconnect():
                _subscriber.stop()
                _start_sse_subscription()
            
            threading.Thread(target=_reconnect, daemon=True).start()

    from src.ui import SettingsWindow
    win = SettingsWindow(_config, on_save=on_save, on_cancel=None, master=_root)
    win.show()


def _open_settings_thread_safe():
    """线程安全入口：pystray 回调调用此函数，内部通过 after 切换到主 Tk 线程。"""
    if _root is not None:
        _root.after(0, _open_settings)


def _quit_thread_safe():
    """线程安全入口：pystray 退出回调，通过 after 切换到主 Tk 线程执行退出。"""
    if _root is not None:
        _root.after(0, _quit)
    else:
        _quit()


def _on_ntfy_message(msg: dict):
    """处理 ntfy 消息（在 SSE 线程中执行）。"""
    global _connected
    
    msg_id = str(msg.get("id", ""))
    if not msg_id:
        return
    
    # 检查是否已处理过（避免重复通知）
    if hasattr(_on_ntfy_message, 'seen_ids'):
        seen_ids = _on_ntfy_message.seen_ids
    else:
        seen_ids = set()
        _on_ntfy_message.seen_ids = seen_ids
    
    # 清理旧 ID（保留最近 1000 条）
    if len(seen_ids) > 1000:
        seen_ids.clear()
    
    if msg_id in seen_ids:
        return
    
    seen_ids.add(msg_id)
    
    title = msg.get("title") or "ntfy 消息"
    message = msg.get("message") or str(msg)
    
    print(f"[ntfy] 收到新消息：{title}", file=sys.stderr)
    send_toast(title, message, app_id="ntfy-Notifier")


def _start_sse_subscription():
    """启动 SSE 订阅。"""
    global _subscriber, _connected
    
    cfg = _config
    server = cfg.get("server", "")
    topic = cfg.get("topic", "")
    username = cfg.get("username", "")
    password = cfg.get("password", "")
    
    if not server or not topic:
        return
    
    try:
        # 停止旧的订阅
        if _subscriber:
            _subscriber.stop()
        
        # 创建新的 SSE 订阅器
        def on_connected():
            """SSE 连接成功回调。"""
            global _connected
            _connected = True
            if _tray:
                _tray.update(True)
            print("[ntfy] ✅ SSE 订阅已连接", file=sys.stderr)
        
        def on_disconnected():
            """SSE 连接断开回调。"""
            global _connected
            _connected = False
            if _tray:
                _tray.update(False)
            print("[ntfy] ⚠️ SSE 连接断开", file=sys.stderr)
        
        _subscriber = NtfySSESubscriber(
            server=server,
            topic=topic,
            username=username,
            password=password,
            on_message=_on_ntfy_message,
            on_connected=on_connected,
            on_disconnected=on_disconnected,
        )
        
        _subscriber.start()
        print("[ntfy] SSE 订阅线程已启动", file=sys.stderr)
        
    except Exception as e:
        print(f"[ntfy] ⚠️ SSE 订阅失败：{e}", file=sys.stderr)
        traceback.print_exc()


def main():
    global _root, _config, _tray, _running

    import tkinter as tk

    _config, is_first_run = load_config()

    # 单例 Tk root（始终存在，隐藏）
    _root = tk.Tk()
    _root.withdraw()
    # 拦截关闭，防止 root 被意外销毁
    _root.protocol("WM_DELETE_WINDOW", lambda: None)

    # 首次运行 → 在 mainloop 启动后立即弹出设置窗口（已在 Tk 线程，无需 after）
    if is_first_run:
        _root.after(200, _open_settings)

    if _config.get("auto_start"):
        _set_auto_start(True)

    # 启动托盘（此时 Tk 已在运行），使用线程安全入口
    _tray = TrayIcon(on_settings=_open_settings_thread_safe, on_quit=_quit_thread_safe)
    _tray.start(connected=False)

    # 启动 SSE 订阅
    _start_sse_subscription()

    # 主 Tk 线程：永不退出
    _root.mainloop()
    _quit()


def _quit():
    global _running, _subscriber, _tray
    _running = False
    
    if _subscriber:
        print("[ntfy] 正在关闭 SSE 订阅...", file=sys.stderr)
        _subscriber.stop()
    
    if _tray:
        _tray.stop()
    
    if _root:
        try:
            _root.quit()
            _root.destroy()
        except Exception:
            pass
    
    sys.exit(0)


if __name__ == "__main__":
    main()
