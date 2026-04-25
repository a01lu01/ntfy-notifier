"""
系统托盘模块 - ntfy-Notifier
使用 pystray 实现托盘图标（兼容 Windows/macOS/Linux）
支持连接/断开状态切换不同图标
"""

import os
import threading
from typing import Callable, Optional

from PIL import Image

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


# ── pystray Menu 项 ─────────────────────────────────────────────────────────
def _make_menu(on_settings: Optional[Callable], on_quit: Optional[Callable]):
    """构建 pystray 菜单。"""
    import pystray

    def settings_action(icon=None, item=None):
        if on_settings:
            on_settings()

    def quit_action(icon=None, item=None):
        if on_quit:
            on_quit()

    return pystray.Menu(
        pystray.MenuItem("⚙️  设置...", settings_action, default=True),
        pystray.MenuItem("❌  退出", quit_action),
    )


# ── TrayIcon 类 ─────────────────────────────────────────────────────────────
class TrayIcon:
    def __init__(self, on_settings: Optional[Callable] = None,
                 on_quit: Optional[Callable] = None):
        self._on_settings = on_settings
        self._on_quit = on_quit
        self._icon: Optional["pystray.Icon"] = None
        self._thread: Optional[threading.Thread] = None
        self._connected = False

    # ── 生命周期 ─────────────────────────────────────────────────────────────

    def start(self, connected: bool = False) -> bool:
        """在独立线程中启动 pystray 托盘图标。"""
        import pystray

        try:
            self._connected = connected
            icon_image = _load_icon(connected)
            menu = _make_menu(self._on_settings, self._on_quit)

            self._icon = pystray.Icon(
                "ntfy-Notifier",
                icon_image,
                "ntfy-Notifier",
                menu,
            )

            self._thread = threading.Thread(
                target=self._icon.run,
                daemon=True,
                name="pystray-thread",
            )
            self._thread.start()
            return True
        except Exception:
            return False

    def update(self, connected: bool):
        """更新托盘图标和提示文字。"""
        self._connected = connected
        if self._icon:
            try:
                icon_img = _load_icon(connected)
                tip = "ntfy-Notifier · 已连接" if connected else "ntfy-Notifier · 未连接"
                self._icon.icon = icon_img
                self._icon.title = tip
            except Exception:
                pass

    def stop(self):
        """安全停止托盘图标。"""
        if self._icon:
            try:
                self._icon.stop()
            except Exception:
                pass
            self._icon = None
