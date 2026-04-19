"""
Windows 通知模块 - ntfy-Notifier
通知优先级：
  1. winrt（Windows Toast 通知，静音）
  2. win32gui.MessageBox（弹窗，有声音）
  3. print stderr（后备）
"""

import sys
import json
import traceback

# ── 通知后端检测 ────────────────────────────────────────────────────────────
_WINRT_AVAILABLE = False
try:
    from winrt.windows.ui.notifications import ToastNotificationManager, ToastNotification
    from winrt.windows.data.xml.dom import XmlDocument
    _WINRT_AVAILABLE = True
except ImportError:
    pass

_WIN32GUI_AVAILABLE = False
try:
    import win32gui
    _WIN32GUI_AVAILABLE = True
except ImportError:
    pass

_MB_ICONINFORMATION = 0x40
_MB_OK = 0


# ── winrt Toast 实现 ────────────────────────────────────────────────────────
def _create_toast_xml(title: str, message: str):
    xml_string = (
        f'<toast activationType="protocol">'
        f'<visual><binding template="ToastGeneric">'
        f'<text>{title}</text>'
        f'<text>{message}</text>'
        f'</binding></visual>'
        f'<audio src="ms-winsoundevent:Notification.IM" />'
        f'</toast>'
    )
    doc = XmlDocument()
    doc.LoadXml(xml_string)
    return doc


def send_toast(title: str, message: str, app_id: str = "ntfy-Notifier") -> bool:
    """
    发送 Windows 通知。

    优先级：winrt Toast → win32gui MessageBox → print stderr
    """
    # 方案 1：winrt Toast（静默通知，Windows 10/11 原生样式）
    if _WINRT_AVAILABLE:
        try:
            notifier = ToastNotificationManager.create_notifier(app_id)
            toast = ToastNotification(_create_toast_xml(title, message))
            notifier.show(toast)
            return True
        except Exception:
            traceback.print_exc()

    # 方案 2：win32gui 弹窗（有系统提示音）
    if _WIN32GUI_AVAILABLE:
        try:
            # MessageBox 在不同线程调用可能阻塞，先尝试
            win32gui.MessageBox(0, message, title, _MB_ICONINFORMATION | _MB_OK)
            return True
        except Exception:
            traceback.print_exc()

    # 方案 3：stderr 打印（仅调试用）
    print(f"[ntfy-Notifier 通知] {title}: {message}", file=sys.stderr)
    return False


# ── ntfy 轮询 ──────────────────────────────────────────────────────────────

def fetch_ntfy_messages(server: str, topic: str,
                        username: str = "", password: str = "") -> list[dict]:
    """
    从 ntfy 服务器拉取最新消息。

    Args:
        server:    ntfy 服务端地址，例如 http://114.55.43.156:8080
        topic:     订阅话题名称
        username:  可选，登录用户名
        password:  可选，登录密码

    Returns:
        消息列表，每条消息为 dict（含 title、message、id 等字段）
    """
    import requests

    url = f"{server.rstrip('/')}/{topic}/json"
    auth = (username, password) if username else None

    try:
        resp = requests.get(url, auth=auth, timeout=10)
        resp.raise_for_status()
        text = resp.text.strip()
        return json.loads(text) if text.startswith("[") else []
    except Exception:
        traceback.print_exc()
        return []
