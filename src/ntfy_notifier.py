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


def _register_aumid():
    """
    注册 AUMID（App User Model ID），让 Windows 通知中心能显示应用图标。

    原理：在开始菜单创建一个快捷方式（.lnk），设置其 AUMID，
    Windows 通过 AUMID 匹配通知来源和图标。
    只写当前用户（HKCU + Start Menu），不需要管理员权限。
    """
    try:
        import os
        import shutil
        import winreg
        import pythoncom
        from win32com.shell import shell, shellcon

        APP_ID = "ntfy-Notifier"
        exe_path = sys.executable

        # ── 图标持久化：复制到 %APPDATA%\ntfy-Notifier\ ──────────────────
        # PyInstaller 打包后 sys._MEIPASS 是临时目录，退出即清理，
        # Windows 通知中心需要持久的图标路径，所以复制到 AppData。
        persistent_dir = os.path.join(
            os.environ.get('APPDATA', ''), 'ntfy-Notifier'
        )
        os.makedirs(persistent_dir, exist_ok=True)
        persistent_icon = os.path.join(persistent_dir, 'connected.ico')

        # 查找源图标
        src_icon = ""
        if getattr(sys, 'frozen', False):
            if hasattr(sys, '_MEIPASS'):
                candidate = os.path.join(sys._MEIPASS, "connected.ico")
                if os.path.exists(candidate):
                    src_icon = candidate
            if not src_icon:
                candidate = os.path.join(os.path.dirname(exe_path), "connected.ico")
                if os.path.exists(candidate):
                    src_icon = candidate
        else:
            base_dir = os.path.dirname(os.path.abspath(__file__))
            candidate = os.path.join(os.path.dirname(base_dir), "connected.ico")
            if os.path.exists(candidate):
                src_icon = candidate

        # 复制图标到持久目录（仅在文件不存在或大小不同时更新）
        if src_icon:
            need_copy = True
            if os.path.exists(persistent_icon):
                try:
                    src_size = os.path.getsize(src_icon)
                    dst_size = os.path.getsize(persistent_icon)
                    need_copy = src_size != dst_size
                except OSError:
                    need_copy = True
            if need_copy:
                shutil.copy2(src_icon, persistent_icon)

        icon_path = persistent_icon if os.path.exists(persistent_icon) else exe_path

        # ── 创建开始菜单快捷方式 ────────────────────────────────────────
        start_menu = shell.SHGetFolderPath(0, shellcon.CSIDL_STARTMENU, None, 0)
        shortcut_dir = os.path.join(start_menu, "Programs")
        os.makedirs(shortcut_dir, exist_ok=True)
        shortcut_path = os.path.join(shortcut_dir, "ntfy-Notifier.lnk")

        # 检查是否需要重建快捷方式
        need_create = True
        if os.path.exists(shortcut_path):
            try:
                existing = pythoncom.CoCreateInstance(
                    shell.CLSID_ShellLink, None,
                    pythoncom.CLSCTX_INPROC_SERVER, shell.IID_IShellLink
                )
                existing.QueryInterface(pythoncom.IID_IPersistFile).Load(shortcut_path)
                existing_path = existing.GetPath(shell.SLGP_SHORTPATH)[0]
                if os.path.normcase(existing_path) == os.path.normcase(exe_path):
                    need_create = False
            except Exception:
                pass

        if need_create:
            shortcut = pythoncom.CoCreateInstance(
                shell.CLSID_ShellLink, None,
                pythoncom.CLSCTX_INPROC_SERVER, shell.IID_IShellLink
            )
            shortcut.SetPath(exe_path)
            shortcut.SetIconLocation(icon_path, 0)
            shortcut.SetDescription("ntfy-Notifier 通知工具")

            # ── 设置 AUMID（关键步骤）─────────────────────────────────
            # 使用 propsys.IID_IPropertyStore 获取属性存储接口，
            # 然后设置 System.AppUserModel.ID 属性。
            try:
                from win32com.propsys import propsys
                property_store = shortcut.QueryInterface(propsys.IID_IPropertyStore)
                property_store.SetValue(
                    propsys.PSGetPropertyKeyFromName("System.AppUserModel.ID"),
                    propsys.PROPVARIANT(APP_ID)
                )
                property_store.Commit()
                print(f"[ntfy] AUMID 已写入快捷方式: {APP_ID}", file=sys.stderr)
            except Exception as e:
                print(f"[ntfy] ⚠️ AUMID 写入失败: {e}", file=sys.stderr)

            persist = shortcut.QueryInterface(pythoncom.IID_IPersistFile)
            persist.Save(shortcut_path, True)
            print(f"[ntfy] 开始菜单快捷方式已创建: {shortcut_path}", file=sys.stderr)

        # ── 注册 AUMID 到注册表 ────────────────────────────────────────
        try:
            key = winreg.CreateKeyEx(
                winreg.HKEY_CURRENT_USER,
                rf"Software\Classes\AppUserModelId\{APP_ID}",
                0, winreg.KEY_SET_VALUE,
            )
            winreg.SetValueEx(key, "DisplayName", 0, winreg.REG_SZ, "ntfy-Notifier")
            winreg.SetValueEx(key, "IconUri", 0, winreg.REG_SZ, icon_path)
            winreg.CloseKey(key)
            print(f"[ntfy] AUMID 已注册到注册表: {APP_ID}", file=sys.stderr)
        except Exception as e:
            print(f"[ntfy] ⚠️ AUMID 注册表写入失败: {e}", file=sys.stderr)

    except Exception as e:
        # AUMID 注册失败不影响通知发送，只是图标显示为默认
        print(f"[ntfy] ⚠️ AUMID 注册失败: {e}", file=sys.stderr)


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

    # 注册 AUMID（让通知中心显示铃铛图标）
    _register_aumid()

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
