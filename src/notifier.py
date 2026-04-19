"""
Windows 通知模块 - ntfy-Notifier
使用 winrt 发送 Windows 10/11 原生通知（toast notification）
"""

import sys
import json
import traceback

try:
    from winrt.windows.ui.notifications import ToastNotificationManager, ToastNotification
    from winrt.windows.data.xml.dom import XmlDocument
    _WINRT_AVAILABLE = True
except ImportError:
    _WINRT_AVAILABLE = False


if _WINRT_AVAILABLE:
    def _create_toast_xml(title: str, message: str) -> XmlDocument:
        """构建 Toast XML 文档。"""
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
else:
    _create_toast_xml = None


def send_toast(title: str, message: str, app_id: str = "ntfy-Notifier") -> bool:
    """
    发送 Windows 原生 Toast 通知。

    Args:
        title:      通知标题
        message:    通知正文
        app_id:     App User Model ID（需要与应用包名匹配以显示应用图标）

    Returns:
        发送是否成功
    """
    if not _WINRT_AVAILABLE or _create_toast_xml is None:
        print(f"[通知] {title}: {message}", file=sys.stderr)
        return False

    try:
        notifier = ToastNotificationManager.create_notifier(app_id)
        toast = ToastNotification(_create_toast_xml(title, message))
        notifier.show(toast)
        return True
    except Exception:
        traceback.print_exc()
        return False


# ── 以下为 requests 轮询相关 ────────────────────────────────────────────────
import json

def fetch_ntfy_messages(server: str, topic: str, username: str = "", password: str = "") -> list[dict]:
    """
    从 ntfy 服务器拉取最新消息。

    Args:
        server:    ntfy 服务端地址，例如 http://114.55.43.156:8080
        topic:     订阅的话题名称
        username:  可选，登录用户名
        password:  可选，登录密码

    Returns:
        消息列表，每条消息为 dict（包含 title、message、id 等字段）
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
