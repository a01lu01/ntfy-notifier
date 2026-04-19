"""
系统托盘模块 - ntfy-Notifier
使用 pystray 实现托盘图标（兼容 Windows/macOS/Linux）
pystray 内置处理线程调度，不存在 Tk/Win32 消息循环冲突
"""

import threading
from typing import Callable, Optional

from PIL import Image, ImageDraw

# ── 图标生成 ────────────────────────────────────────────────────────────────
def _create_icon_image() -> Image:
    """用 Pillow 画一个蓝色圆形图标（32x32）。"""
    size = 32
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    draw.ellipse([2, 2, size - 3, size - 3], fill=(0, 120, 212, 255))  # #0078D4
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
        self._icon: Optional[pystray.Icon] = None
        self._thread: Optional[threading.Thread] = None

    # ── 生命周期 ─────────────────────────────────────────────────────────────

    def start(self, connected: bool = False) -> bool:
        """在独立线程中启动 pystray 托盘图标。"""
        import pystray

        try:
            icon_image = _create_icon_image()
            menu = _make_menu(self._on_settings, self._on_quit)

            self._icon = pystray.Icon(
                "ntfy-Notifier",
                icon_image,
                "ntfy-Notifier",
                menu,
            )

            # 在守护线程中运行 pystray 事件循环，与 Tk mainloop 完全独立
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
        """更新托盘状态（预留，未来可改图标颜色）。"""
        # TODO: 连接/断开时切换不同图标
        pass

    def stop(self):
        """安全停止托盘图标。"""
        if self._icon:
            try:
                self._icon.stop()
            except Exception:
                pass
            self._icon = None
