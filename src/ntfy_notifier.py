"""
ntfy-Notifier 主程序
监听 ntfy 消息并弹出 Windows 原生通知
遵循 Fluent Design 视觉风格

订阅模式：SSE (Server-Sent Events) — 实时推送，无需轮询

修复记录：
- 添加 Windows 命名互斥体单例锁
- 开机启动时主动 HTTP 探测网络就绪，替代固定 15s 延迟
- 修复：托盘图标更新改为 PostMessage 线程安全方式（tray.py）
- 修复：SSE 健康检查防止僵尸连接（notifier.py）
"""

import os
import sys
import threading
import time
import traceback
from queue import Empty, Queue
from typing import Optional
from threading import Event

import requests

from src.config import load_config, save_config
from src.history import record_message
from src.notifier import send_toast, NtfySSESubscriber
from src.tray import TrayIcon

# ── 单例锁 ───────────────────────────────────────────

_mutex = None  # 保持互斥体引用，防止被 GC 回收


def _check_single_instance():
    """Windows 命名互斥体单例锁，防止多个实例同时运行。"""
    global _mutex
    try:
        import ctypes
        import ctypes.wintypes
        _mutex = ctypes.windll.kernel32.CreateMutexW(
            None, False, "ntfy-Notifier-SingleInstance"
        )
        last_error = ctypes.windll.kernel32.GetLastError()
        if last_error == 183:  # ERROR_ALREADY_EXISTS
            print("[ntfy] 另一个实例已在运行，退出", file=sys.stderr)
            sys.exit(0)
    except Exception as e:
        print(f"[ntfy] 单例锁检查失败: {e}", file=sys.stderr)


def _is_boot_period():
    """检测是否在开机后2分钟内。"""
    try:
        import ctypes
        import ctypes.wintypes
        # GetTickCount64 returns milliseconds since boot
        class ULARGE(ctypes.Structure):
            _fields_ = [("low", ctypes.wintypes.DWORD), ("high", ctypes.wintypes.DWORD)]
        tick = ULARGE()
        ctypes.windll.kernel32.GetTickCount64(ctypes.byref(tick))
        ms = tick.low | (tick.high << 32)
        return ms < 120000  # 120 seconds
    except Exception:
        return False


def _wait_for_network(server_url: str, max_wait: int = 60, interval: int = 3) -> bool:
    """等待网络就绪，通过 HTTP 探测服务器可达性。

    在后台线程中调用，不会阻塞 Tk 主线程。

    Args:
        server_url: ntfy 服务器地址（如 http://your-server:8080）
        max_wait: 最大等待时间（秒）
        interval: 探测间隔（秒）

    Returns:
        True 表示服务器可达，False 表示超时
    """
    start = time.time()
    attempt = 0
    while time.time() - start < max_wait:
        attempt += 1
        try:
            # 简单的 HTTP GET 探测，只关心能否建立连接
            resp = requests.get(
                server_url,
                timeout=5,
                proxies={"http": None, "https": None},
            )
            print(f"[ntfy] 网络就绪探测成功（第 {attempt} 次，HTTP {resp.status_code}）", file=sys.stderr)
            return True
        except (requests.ConnectionError, requests.Timeout):
            elapsed = time.time() - start
            print(
                f"[ntfy] 网络未就绪（第 {attempt} 次，已等待 {elapsed:.0f}s），"
                f"{interval}s 后重试...",
                file=sys.stderr,
            )
            time.sleep(interval)
        except Exception as e:
            print(f"[ntfy] 网络探测异常: {type(e).__name__}: {e}", file=sys.stderr)
            time.sleep(interval)

    print(f"[ntfy] 网络就绪探测超时（{max_wait}s），放弃等待", file=sys.stderr)
    return False


# ── 全局状态 ────────────────────────────────────────────────────────────────

_config = {}
_subscriber: Optional[NtfySSESubscriber] = None
_running = True
_connected = False
_tray: Optional[TrayIcon] = None
_root: "tk.Tk | None" = None
_ui_queue: Optional[Queue] = None


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
            if getattr(sys, "frozen", False):
                target = f'"{sys.executable}"'
            else:
                # 源码运行：用 pythonw.exe（避免黑框）+ 主脚本绝对路径
                python = sys.executable
                pythonw = os.path.join(os.path.dirname(python), "pythonw.exe")
                if not os.path.exists(pythonw):
                    pythonw = python
                script = os.path.join(
                    os.path.dirname(os.path.abspath(__file__)), "ntfy_notifier.py"
                )
                target = f'"{pythonw}" "{script}"'
            winreg.SetValueEx(key, "ntfy-Notifier", 0, winreg.REG_SZ, target)
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
    """在主 Tk 线程中弹出设置窗口。"""
    if _root is None:
        return

    def on_save(cfg: dict):
        global _config, _subscriber
        
        save_config(cfg)
        _config = cfg
        _set_auto_start(cfg.get("auto_start", False))
        
        # 重新连接 SSE：无论当前是否已连接都重启订阅，
        # 保证断线状态下修改的配置也能立即生效
        # 在后台线程中执行 stop + restart，避免 join 阻塞 Tk 主线程
        if _subscriber is not None:
            print("[ntfy] 配置已更新，后台重连 SSE...", file=sys.stderr)
            
            def _reconnect():
                _subscriber.stop()
                _start_sse_subscription()
            
            threading.Thread(target=_reconnect, daemon=True).start()

    from src.ui import SettingsWindow
    win = SettingsWindow(_config, on_save=on_save, on_cancel=None, master=_root)
    win.show()


def _open_history():
    """在主 Tk 线程中打开推送历史窗口。"""
    if _root is None:
        return
    from src.ui import HistoryWindow
    win = HistoryWindow(master=_root, on_settings=_open_settings)
    win.show()


def _post_to_ui(fn):
    """把任务投递到主 Tk 线程（线程安全，非阻塞）。"""
    if _ui_queue is not None:
        _ui_queue.put(fn)


def _drain_ui_queue():
    """主线程轮询 UI 队列并执行任务（由 root.after 驱动）。"""
    if _ui_queue is not None:
        while True:
            try:
                fn = _ui_queue.get_nowait()
            except Empty:
                break
            try:
                fn()
            except Exception:
                traceback.print_exc()
    if _root is not None:
        try:
            _root.after(50, _drain_ui_queue)
        except Exception:
            pass


def _open_settings_thread_safe():
    """线程安全入口：pystray 回调调用此函数，内部切换到主 Tk 线程。"""
    _post_to_ui(_open_settings)


def _open_history_thread_safe():
    """线程安全入口：托盘点击历史入口时调用。"""
    _post_to_ui(_open_history)


def _quit_thread_safe():
    """线程安全入口：pystray 退出回调，切换到主 Tk 线程执行退出。"""
    if _root is None:
        _quit()
    else:
        _post_to_ui(_quit)


def _on_ntfy_message(msg: dict):
    """处理 ntfy 消息（在 SSE 线程中执行）。"""
    global _connected
    
    msg_id = str(msg.get("id", ""))
    if not msg_id:
        return
    
    # 持久化去重：历史库中已存在该 id → 重复消息，跳过通知
    try:
        recorded = record_message(msg)
    except Exception as e:
        print(f"[ntfy] 历史记录写入异常: {e}", file=sys.stderr)
        recorded = None

    if recorded is False:
        print(f"[ntfy] 跳过重复消息：{msg_id}", file=sys.stderr)
        return

    if recorded is None:
        # 数据库不可用 → 回退到内存去重，保证不重复通知
        if not hasattr(_on_ntfy_message, "_seen_ids"):
            _on_ntfy_message._seen_ids = set()
        seen_ids = _on_ntfy_message._seen_ids
        if msg_id in seen_ids:
            return
        seen_ids.add(msg_id)
        # 超过 1000 条时逐条淘汰，不再整体清空
        if len(seen_ids) > 1000:
            for _ in range(len(seen_ids) // 2):
                seen_ids.pop()
    
    title = msg.get("title") or "ntfy 消息"
    message = msg.get("message") or str(msg)
    
    print(f"[ntfy] 收到新消息：{title}", file=sys.stderr)
    
    # 读取自动复制验证码开关
    auto_copy_otp = _config.get("auto_copy_otp", False)
    send_toast(title, message, app_id="ntfy-Notifier", auto_copy_otp=auto_copy_otp)


def _start_sse_subscription():
    """启动 SSE 订阅。"""
    global _subscriber, _connected
    
    cfg = _config
    server = cfg.get("server", "")
    topic = cfg.get("topic", "")
    username = cfg.get("username", "")
    password = cfg.get("password", "")
    
    # 无论配置是否完整，都先停止旧订阅，避免旧连接残留
    if _subscriber is not None:
        _subscriber.stop()
        _subscriber = None

    if not server or not topic:
        print("[ntfy] 未配置服务器或主题，不启动订阅", file=sys.stderr)
        return

    try:
        
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
    global _root, _config, _tray, _running, _ui_queue

    import tkinter as tk

    # 单例锁检查（必须在任何 UI 初始化之前）
    _check_single_instance()

    _config, is_first_run, config_was_corrupt = load_config()

    # 注册 AUMID（让通知中心显示铃铛图标）
    _register_aumid()

    # 单例 Tk root（始终存在，隐藏）
    _root = tk.Tk()
    _root.withdraw()
    # 拦截关闭，防止 root 被意外销毁
    _root.protocol("WM_DELETE_WINDOW", lambda: None)

    # 主线程 UI 任务队列（pystray/后台线程只投递，不直接操作 Tk）
    _ui_queue = Queue()
    _root.after(50, _drain_ui_queue)

    if config_was_corrupt:
        _root.after(400, lambda: tk.messagebox.showwarning(
            "ntfy-Notifier",
            "配置文件损坏，已备份并重置为默认配置，请重新填写设置。",
            parent=_root,
        ))

    # 首次运行 → 在 mainloop 启动后立即弹出设置窗口（已在 Tk 线程，无需 after）
    if is_first_run:
        _root.after(200, _open_settings)

    if _config.get("auto_start"):
        _set_auto_start(True)

    # 启动托盘（此时 Tk 已在运行），使用线程安全入口
    _tray = TrayIcon(
        on_settings=_open_settings_thread_safe,
        on_history=_open_history_thread_safe,
        on_quit=_quit_thread_safe,
    )
    _tray.start(connected=False)

    # 开机启动时：主动探测网络就绪后再启动 SSE
    if _is_boot_period():
        print("[ntfy] 检测到开机启动，探测网络就绪后启动 SSE...", file=sys.stderr)
        server_url = _config.get("server", "")
        if server_url:
            def _boot_delayed_start():
                _wait_for_network(server_url, max_wait=60, interval=3)
                # 无论探测是否成功，都尝试启动 SSE（SSE 本身有重连机制）
                _post_to_ui(_start_sse_subscription)
            threading.Thread(target=_boot_delayed_start, daemon=True, name="BootNetProbe").start()
        else:
            _root.after(15000, _start_sse_subscription)  # 无服务器地址时的后备
    else:
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
