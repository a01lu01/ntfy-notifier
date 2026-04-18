"""
系统托盘模块 - ntfy-Notifier
基于 pystray 实现跨平台系统托盘（Windows/macOS/Linux）
"""

import threading
from typing import Callable, Optional

try:
    from pystray import Icon, Menu, MenuItem as _MenuItem
    from PIL import Image
    _PYSTRAY_AVAILABLE = True
except ImportError:
    _PYSTRAY_AVAILABLE = False


def _build_tray_icon_image() -> "Image.Image":
    """用 Pillow 在内存中绘制托盘图标：256x256，圆形蓝色底 + 白色铃铛。"""
    from PIL import Image as PILImage, ImageDraw

    size = 256
    img = PILImage.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 圆形蓝色背景
    draw.ellipse([16, 16, size - 16, size - 16], fill=(0, 120, 212, 255))

    cx = size // 2
    bell_body_top = 72
    bell_body_bottom = 172
    bell_top_w = 56
    bell_bottom_w = 100

    for y in range(bell_body_top, bell_body_bottom):
        ratio = (y - bell_body_top) / (bell_body_bottom - bell_body_top)
        w = bell_top_w + int((bell_bottom_w - bell_top_w) * ratio)
        x0 = cx - w // 2
        x1 = cx + w // 2
        draw.line([(x0, y), (x1, y)], fill=(255, 255, 255, 255), width=10)

    draw.ellipse([cx - 20, bell_body_bottom - 10, cx + 20, bell_body_bottom + 34], fill=(255, 255, 255, 255))
    draw.ellipse([cx - 16, bell_body_top - 30, cx + 16, bell_body_top + 6], fill=(255, 255, 255, 255))

    return img


class TrayIcon:
    """托盘图标封装，支持状态切换和事件回调。"""

    def __init__(
        self,
        on_settings: Optional[Callable[[], None]] = None,
        on_quit: Optional[Callable[[], None]] = None,
    ):
        self._on_settings = on_settings
        self._on_quit = on_quit
        self._icon: Optional["Icon"] = None
        self._thread: Optional[threading.Thread] = None

    def _build_menu(self, connected: bool) -> "Menu":
        status_label = "状态: 已连接" if connected else "状态: 未连接"
        return Menu(
            _MenuItem("📱 ntfy-Notifier", lambda _: None, enabled=False),
            Menu.SEPARATOR,
            _MenuItem(status_label, lambda _: None, enabled=False),
            Menu.SEPARATOR,
            _MenuItem("⚙️ 设置...", lambda _: self._on_settings and self._on_settings()),
            Menu.SEPARATOR,
            _MenuItem("❌ 退出", lambda _: self._on_quit and self._on_quit()),
        )

    def _make_icon(self, connected: bool) -> "Icon":
        icon = Icon(
            "ntfy-notifier",
            _build_tray_icon_image(),
            "ntfy-Notifier",
            self._build_menu(connected),
        )
        return icon

    def start(self, connected: bool = False):
        """在后台线程启动托盘图标。"""
        if not _PYSTRAY_AVAILABLE:
            import sys
            print("[Tray] pystray 未安装，托盘功能不可用。", file=sys.stderr)
            return
        self._icon = self._make_icon(connected)
        self._thread = threading.Thread(target=self._run, daemon=True, name="TrayIconThread")
        self._thread.start()

    def _run(self):
        if self._icon:
            self._icon.run()

    def update(self, connected: bool):
        """切换托盘菜单状态（连接/断开）。"""
        if self._icon:
            self._icon.menu = self._build_menu(connected)
            self._icon.update_menu()

    def stop(self):
        """停止并销毁托盘图标。"""
        if self._icon:
            self._icon.stop()
            self._icon = None
