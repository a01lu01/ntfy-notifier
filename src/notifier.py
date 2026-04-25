"""
Windows 通知模块 - ntfy-Notifier
通知优先级：
  1. winrt（Windows Toast 通知，静音）
  2. win32gui.MessageBox（弹窗，有声音）
  3. print stderr（后备）

订阅模式：SSE (Server-Sent Events) — 实时推送，无需轮询
"""

import sys
import json
import traceback
import threading
import time
from typing import Callable, Optional

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
            win32gui.MessageBox(0, message, title, _MB_ICONINFORMATION | _MB_OK)
            return True
        except Exception:
            traceback.print_exc()

    # 方案 3：stderr 打印（仅调试用）
    print(f"[ntfy-Notifier 通知] {title}: {message}", file=sys.stderr)
    return False


# ── SSE 订阅器 ──────────────────────────────────────────────────────────────

class NtfySSESubscriber:
    """
    ntfy SSE 订阅器 — 实时接收消息推送。
    
    使用方式：
        subscriber = NtfySSESubscriber(server, topic, username, password)
        subscriber.on_message = lambda msg: print(msg)
        subscriber.start()
        
        # ... 程序运行时自动接收消息 ...
        
        subscriber.stop()
    """

    def __init__(self, server: str, topic: str,
                 username: str = "", password: str = "",
                 on_message: Optional[Callable] = None):
        self.server = server.rstrip('/')
        self.topic = topic
        self.username = username
        self.password = password
        self.on_message = on_message  # 收到消息时的回调函数
        
        self._running = False
        self._thread: Optional[threading.Thread] = None
        self._session_id: Optional[str] = None

    def start(self):
        """启动 SSE 订阅（在后台线程运行）。"""
        if self._running:
            return
        
        self._running = True
        self._thread = threading.Thread(
            target=self._subscribe_loop,
            daemon=True,
            name="NtfySSEThread",
        )
        self._thread.start()

    def stop(self):
        """停止 SSE 订阅。"""
        self._running = False
        if self._thread:
            self._thread.join(timeout=5)

    def _subscribe_loop(self):
        """SSE 订阅循环，自动重连。"""
        import requests
        
        while self._running:
            try:
                url = f"{self.server}/{self.topic}/sse"
                auth = (self.username, self.password) if self.username else None
                
                print(f"[ntfy] SSE 连接中... {url}", file=sys.stderr)
                
                resp = requests.get(
                    url,
                    auth=auth,
                    timeout=None,  # SSE 是长连接，不设置超时
                    proxies={"http": None, "https": None},
                    stream=True,
                )
                
                if resp.status_code != 200:
                    print(f"[ntfy] ⚠️ SSE 连接失败：HTTP {resp.status_code}", file=sys.stderr)
                    time.sleep(5)
                    continue
                
                print("[ntfy] ✅ SSE 已连接，等待消息...", file=sys.stderr)
                
                # 解析 SSE 事件流
                for line in resp.iter_lines():
                    if not self._running:
                        break
                    
                    try:
                        text = line.decode('utf-8')
                        
                        # SSE 格式：event: message\n data: {...}\n\n
                        if text.startswith('data: '):
                            data_str = text[6:]  # 去掉 "data: " 前缀
                            msg = json.loads(data_str)
                            
                            event_type = msg.get("event", "")
                            
                            if event_type == "open":
                                self._session_id = msg.get("id")
                                print(f"[ntfy] SSE session opened: {self._session_id}", file=sys.stderr)
                            elif event_type == "message":
                                # 收到新消息，触发回调
                                if self.on_message:
                                    self.on_message(msg)
                    
                    except json.JSONDecodeError:
                        pass  # 忽略非 JSON 行（如注释、空行）
                    except UnicodeDecodeError:
                        pass
                
                # SSE 连接断开，等待后重连
                print("[ntfy] ⚠️ SSE 连接断开，5 秒后重连...", file=sys.stderr)
                time.sleep(5)
                
            except requests.exceptions.ConnectionError:
                print("[ntfy] ⚠️ 网络连接失败，5 秒后重试...", file=sys.stderr)
                time.sleep(5)
            except Exception as e:
                if self._running:  # 忽略主动停止时的异常
                    print(f"[ntfy] ⚠️ SSE 错误：{type(e).__name__}: {e}", file=sys.stderr)
                    time.sleep(5)


# ── 便捷函数 ──────────────────────────────────────────────────────────────

def subscribe_ntfy(server: str, topic: str, username: str = "", password: str = "") -> NtfySSESubscriber:
    """
    创建并启动 ntfy SSE 订阅器。
    
    Args:
        server:   ntfy 服务器地址，例如 http://114.55.43.156:8080
        topic:    订阅话题，例如 sms
        username: 用户名（可选）
        password: 密码（可选）
    
    Returns:
        NtfySSESubscriber 实例
    
    Example:
        subscriber = subscribe_ntfy("http://114.55.43.156:8080", "sms", "iPhone", "WHYntfy2026")
        
        def on_message(msg):
            title = msg.get("title") or "ntfy 消息"
            message = msg.get("message") or str(msg)
            send_toast(title, message)
        
        subscriber.on_message = on_message
    """
    subscriber = NtfySSESubscriber(server, topic, username, password)
    subscriber.start()
    return subscriber
