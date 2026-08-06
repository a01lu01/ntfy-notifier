"""
系统托盘模块 - ntfy-Notifier
使用 pystray 实现托盘图标（兼容 Windows/macOS/Linux）
支持连接/断开状态切换不同图标

修复记录：
- 预加载图标到内存，避免 SSE 线程中文件 IO 失败
- 增加错误日志，不再静默吞掉异常
- 提供后备方案（Pillow 绘制简单圆形）
- 使用 PostMessage + _message_handlers 在 pystray 线程中安全更新图标
  （修复：跨线程 Shell_NotifyIconW 导致图标颜色与 tooltip 不一致的 bug）
"""

import ctypes
import os
import sys
import threading
from typing import Callable, Optional

from PIL import Image

# ── 自定义 Windows 消息 ─────────────────────────────────────────────────────
# WM_USER + 100，避免与 pystray 内部消息（WM_STOP, WM_NOTIFY 等）冲突
_WM_UPDATE_STATE = 0x0400 + 100  # WM_USER = 0x0400

# ── 图标路径 ────────────────────────────────────────────────────────────────
_DIR = os.path.dirname(os.path.abspath(__file__))
_Parent_DIR = os.path.dirname(_DIR)
_ICON_CONNECTED = os.path.join(_Parent_DIR, "connected.ico")
_ICON_DISCONNECTED = os.path.join(_Parent_DIR, "disconnected.ico")


def _load_icon(connected: bool) -> Image:
    """加载对应状态的托盘图标。"""
    path = _ICON_CONNECTED if connected else _ICON_DISCONNECTED
    if os.path.exists(path):
        return Image.open(path)
    # 后备：用 Pillow 画一个简单圆形
    size = 32
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    from PIL import ImageDraw
    draw = ImageDraw.Draw(img)
    color = (26, 250, 41, 255) if connected else (216, 30, 6, 255)
    draw.ellipse([2, 2, size - 3, size - 3], fill=color)
    return img


def _make_fallback_icon(connected: bool) -> Image:
    """用 Pillow 绘制简单圆形图标（后备方案，不依赖文件 IO）。"""
    size = 32
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    from PIL import ImageDraw
    draw = ImageDraw.Draw(img)
    color = (26, 250, 41, 255) if connected else (216, 30, 6, 255)
    draw.ellipse([2, 2, size - 3, size - 3], fill=color)
    return img


# ── pystray Menu 项 ─────────────────────────────────────────────────────────
def _make_menu(on_push: Optional[Callable], on_settings: Optional[Callable],
               on_quit: Optional[Callable]):
    """构建 pystray 菜单。"""
    import pystray

    def push_action(icon=None, item=None):
        if on_push:
            on_push()

    def settings_action(icon=None, item=None):
        if on_settings:
            on_settings()

    def quit_action(icon=None, item=None):
        if on_quit:
            on_quit()

    return pystray.Menu(
        pystray.MenuItem("推送", push_action, default=True),
        pystray.MenuItem("设置", settings_action),
        pystray.MenuItem("退出", quit_action),
    )


# ── TrayIcon 类 ─────────────────────────────────────────────────────────────
class TrayIcon:
    def __init__(self, on_settings: Optional[Callable] = None,
                 on_push: Optional[Callable] = None,
                 on_quit: Optional[Callable] = None):
        self._on_settings = on_settings
        self._on_push = on_push
        self._on_quit = on_quit
        self._icon: Optional["pystray.Icon"] = None
        self._thread: Optional[threading.Thread] = None
        self._connected = False

        # 预加载图标到内存，避免在线程中执行文件 IO
        self._icon_connected_img: Optional[Image] = None
        self._icon_disconnected_img: Optional[Image] = None
        try:
            self._icon_connected_img = _load_icon(True)
            self._icon_disconnected_img = _load_icon(False)
        except Exception as e:
            print(f"[tray] 预加载图标失败: {e}", file=sys.stderr)
            # 后备：使用 Pillow 绘制简单圆形
            try:
                self._icon_connected_img = _make_fallback_icon(True)
                self._icon_disconnected_img = _make_fallback_icon(False)
            except Exception as e2:
                print(f"[tray] 后备图标也失败: {e2}", file=sys.stderr)

        # 待更新的连接状态（用于 PostMessage 跨线程调度）
        self._pending_connected: Optional[bool] = None
        self._lock = threading.Lock()

    # ── 生命周期 ─────────────────────────────────────────────────────────────

    def start(self, connected: bool = False) -> bool:
        """在独立线程中启动 pystray 托盘图标。"""
        import pystray

        try:
            self._connected = connected
            # 优先使用预加载的图标
            icon_image = self._get_cached_icon(connected)
            menu = _make_menu(self._on_push, self._on_settings, self._on_quit)

            tip = "ntfy-Notifier · 已连接" if connected else "ntfy-Notifier · 未连接"

            self._icon = pystray.Icon(
                "ntfy-Notifier",
                icon_image,
                tip,
                menu,
            )

            # 注册自定义消息处理函数，使 pystray 线程的 _dispatcher
            # 能在正确线程上处理 WM_UPDATE_STATE 消息
            # 防御 pystray 内部结构变化：缺失时降级为公开 setter
            handlers = getattr(self._icon, "_message_handlers", None)
            if handlers is not None:
                try:
                    handlers[_WM_UPDATE_STATE] = self._on_wm_update_state
                except Exception as e:
                    print(f"[tray] 注册消息处理失败，降级更新方式: {e}", file=sys.stderr)

            self._thread = threading.Thread(
                target=self._icon.run,
                daemon=True,
                name="pystray-thread",
            )
            self._thread.start()
            return True
        except Exception as e:
            print(f"[tray] 启动失败: {e}", file=sys.stderr)
            return False

    def _get_cached_icon(self, connected: bool) -> Image:
        """获取预加载的图标，如果不存在则实时加载或使用后备方案。"""
        cached = self._icon_connected_img if connected else self._icon_disconnected_img
        if cached is not None:
            return cached
        # 缓存不存在，尝试实时加载
        try:
            return _load_icon(connected)
        except Exception as e:
            print(f"[tray] 实时加载图标失败: {e}", file=sys.stderr)
            return _make_fallback_icon(connected)

    def _on_wm_update_state(self, wparam, lparam):
        """在 pystray 线程上执行的状态更新处理函数。

        当 pystray 线程的 _dispatcher 收到 WM_UPDATE_STATE 消息时调用。
        此时 Shell_NotifyIconW 在正确的线程上执行，图标更新不会静默失败。
        """
        with self._lock:
            connected = self._pending_connected
            self._pending_connected = None

        if connected is not None:
            self._connected = connected
            self._apply_update(connected)

    def update(self, connected: bool):
        """更新托盘图标和提示文字（线程安全）。

        从任何线程调用都是安全的。通过 PostMessage 将更新请求投递到
        pystray 线程的消息队列，由 _on_wm_update_state 在正确线程上执行。
        """
        with self._lock:
            self._connected = connected
            self._pending_connected = connected

        if self._icon:
            hwnd = getattr(self._icon, '_hwnd', None)
            if hwnd:
                # 通过 PostMessage 将更新调度到 pystray 线程
                # 这是 Windows 跨线程 UI 操作的正确方式
                ctypes.windll.user32.PostMessageW(
                    hwnd, _WM_UPDATE_STATE,
                    1 if connected else 0, 0
                )
            else:
                # _hwnd 尚未就绪（pystray 线程还没创建窗口），
                # 延迟 500ms 重试
                threading.Timer(0.5, self.update, args=[connected]).start()

    def _apply_update(self, connected: bool):
        """实际执行图标更新（在 pystray 线程上调用，设置属性 + 错误日志 + 后备方案）。"""
        try:
            icon_img = self._get_cached_icon(connected)
            tip = "ntfy-Notifier · 已连接" if connected else "ntfy-Notifier · 未连接"
            self._icon.icon = icon_img
            self._icon.title = tip
        except Exception as e:
            print(f"[tray] 图标更新失败: {e}", file=sys.stderr)
            # 后备方案：用 Pillow 绘制简单圆形
            try:
                fallback = _make_fallback_icon(connected)
                self._icon.icon = fallback
                tip = "ntfy-Notifier · 已连接" if connected else "ntfy-Notifier · 未连接"
                self._icon.title = tip
            except Exception as e2:
                print(f"[tray] 后备图标更新也失败: {e2}", file=sys.stderr)

    def stop(self):
        """安全停止托盘图标。"""
        if self._icon:
            try:
                self._icon.stop()
            except Exception as e:
                print(f"[tray] 停止失败: {e}", file=sys.stderr)
            self._icon = None
