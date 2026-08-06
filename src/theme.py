"""Fluent 主题引擎 - ntfy-Notifier

提供浅色/深色两套色彩 token，以及跟随系统主题的解析与轮询。
"""

import threading
import time
from typing import Callable, Optional

LIGHT = {
    "window_bg": "#F3F3F3",
    "card_bg": "#FFFFFF",
    "text": "#1F1F1F",
    "subtext": "#5D5D5D",
    "hover": "#E9E9E9",
    "selected": "#E3E3E3",
    "accent": "#6C357C",
    "accent_text": "#6C357C",
    "border": "#E0E0E0",
    "input_border": "#C9C9C9",
    "input_bg": "#FFFFFF",
}

DARK = {
    "window_bg": "#202020",
    "card_bg": "#2B2B2B",
    "text": "#FFFFFF",
    "subtext": "#C7C7C7",
    "hover": "#333333",
    "selected": "#3A3A3A",
    "accent": "#7D3D8E",
    "accent_text": "#A96BB8",
    "border": "#3C3C3C",
    "input_border": "#4A4A4A",
    "input_bg": "#2B2B2B",
}

ALL_TOKEN_KEYS = frozenset(LIGHT.keys())


def _system_is_dark() -> bool:
    """读取 Windows 深浅色设置（AppsUseLightTheme：0=深色，1=浅色）。"""
    try:
        import winreg
        with winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        ) as key:
            value, _ = winreg.QueryValueEx(key, "AppsUseLightTheme")
            return value == 0
    except Exception:
        # 读取失败时按浅色处理
        return False


class ThemeManager:
    """主题模式解析与系统主题监听。"""

    def __init__(self, mode: str = "system"):
        self.mode = mode
        self.current = self.resolve()
        self._running = False
        self._thread: Optional[threading.Thread] = None
        self._on_change: Optional[Callable[[str], None]] = None

    def resolve(self, mode: Optional[str] = None) -> str:
        """把主题模式解析为 'light' / 'dark'。"""
        mode = mode or self.mode
        if mode == "light":
            return "light"
        if mode == "dark":
            return "dark"
        return "dark" if _system_is_dark() else "light"

    def set_mode(self, mode: str):
        self.mode = mode
        self.current = self.resolve()

    def start_polling(self, callback: Callable[[str], None]):
        """启动系统主题轮询（约 2 秒一次），变化时回调。"""
        self._on_change = callback
        self._running = True
        self._thread = threading.Thread(
            target=self._poll_loop, daemon=True, name="ThemeWatch"
        )
        self._thread.start()

    def _poll_loop(self):
        last = self.resolve()
        while self._running:
            time.sleep(2)
            current = self.resolve()
            if current != last:
                last = current
                self.current = current
                if self._on_change is not None:
                    try:
                        self._on_change(current)
                    except Exception:
                        pass

    def stop(self):
        self._running = False
        if self._thread is not None:
            self._thread.join(timeout=3)
            self._thread = None
