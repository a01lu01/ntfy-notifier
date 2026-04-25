"""
Windows 通知模块 - ntfy-Notifier
通知优先级：
  1. winotify（WinRT Toast 通知，支持 AUMID 图标 + 声音）
  2. plyer（Windows Toast 通知，无 AUMID 支持）
  3. win32gui.MessageBox（弹窗，有声音）
  4. print stderr（后备）

订阅模式：SSE (Server-Sent Events) — 实时推送，无需轮询
"""

import sys
import json
import traceback
import threading
import time
from typing import Callable, Optional

# ── 通知后端检测 ────────────────────────────────────────────────────────────

_WINOTIFY_AVAILABLE = False
try:
    from winotify import Notification, audio as winotify_audio
    _WINOTIFY_AVAILABLE = True
except ImportError:
    pass

_WINRT_AVAILABLE = False
try:
    from winrt.windows.ui.notifications import ToastNotificationManager, ToastNotification
    from winrt.windows.data.xml.dom import XmlDocument
    _WINRT_AVAILABLE = True
except ImportError:
    pass

_PLYER_AVAILABLE = False
try:
    from plyer import notification as plyer_notify
    _PLYER_AVAILABLE = True
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


# ── winotify Toast 实现（首选，支持 AUMID + 声音）─────────────────────────
def _send_winotify_toast(title: str, message: str, app_id: str = "ntfy-Notifier") -> bool:
    """使用 winotify 发送 WinRT Toast 通知（通过 AUMID 显示应用图标）。"""
    try:
        # 不传 icon，让通知中心使用默认图标
        toast = Notification(
            app_id=app_id,
            title=title,
            msg=message,
            duration="short",
        )
        toast.set_audio(winotify_audio.Default, loop=False)
        toast.show()
        return True
    except Exception:
        traceback.print_exc()
        return False


# ── Plyer Toast 实现（后备，不支持 AUMID）─────────────────────────────────
def _send_plyer_toast(title: str, message: str) -> bool:
    """使用 plyer 发送原生 Toast 通知。"""
    try:
        plyer_notify.notify(
            title=title,
            message=message,
            app_name="ntfy-Notifier",
            timeout=10,
        )
        return True
    except Exception:
        traceback.print_exc()
        return False


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

    优先级：winotify → plyer → winrt Toast → win32gui MessageBox → print stderr
    """
    # 方案 1：winotify（WinRT Toast，支持 AUMID 图标）
    if _WINOTIFY_AVAILABLE:
        try:
            return _send_winotify_toast(title, message, app_id)
        except Exception:
            pass

    # 方案 2：plyer（跨平台，无 AUMID 支持，图标为默认）
    if _PLYER_AVAILABLE:
        try:
            return _send_plyer_toast(title, message)
        except Exception:
            pass

    # 方案 3：winrt Toast（静默通知，Windows 10/11 原生样式）
    if _WINRT_AVAILABLE:
        try:
            notifier = ToastNotificationManager.create_notifier(app_id)
            toast = ToastNotification(_create_toast_xml(title, message))
            notifier.show(toast)
            return True
        except Exception:
            traceback.print_exc()

    # 方案 4：win32gui 弹窗（有系统提示音）
    if _WIN32GUI_AVAILABLE:
        try:
            win32gui.MessageBox(0, message, title, _MB_ICONINFORMATION | _MB_OK)
            return True
        except Exception:
            traceback.print_exc()

    # 方案 5：stderr 打印（仅调试用）
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
                 on_message: Optional[Callable] = None,
                 on_connected: Optional[Callable] = None,
                 on_disconnected: Optional[Callable] = None):
        self.server = server.rstrip('/')
        self.topic = topic
        self.username = username
        self.password = password
        self.on_message = on_message
        self.on_connected = on_connected        # SSE 连接成功回调
        self.on_disconnected = on_disconnected  # SSE 连接断开回调
        
        self._running = False
        self._thread: Optional[threading.Thread] = None
        self._session_id: Optional[str] = None
        self._resp: Optional[requests.Response] = None  # 保存响应对象以便关闭

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
        # 关闭响应以中断 iter_lines 阻塞
        if self._resp is not None:
            try:
                self._resp.close()
            except Exception:
                pass
            self._resp = None
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
                
                self._resp = requests.get(
                    url,
                    auth=auth,
                    timeout=None,  # SSE 是长连接，不设置超时
                    proxies={"http": None, "https": None},
                    stream=True,
                )
                
                if self._resp.status_code != 200:
                    print(f"[ntfy] ⚠️ SSE 连接失败：HTTP {self._resp.status_code}", file=sys.stderr)
                    self._resp = None
                    if self.on_disconnected:
                        self.on_disconnected()
                    time.sleep(5)
                    continue
                
                # 通知连接成功
                if self.on_connected:
                    self.on_connected()
                
                print("[ntfy] ✅ SSE 已连接，等待消息...", file=sys.stderr)
                
                # 解析 SSE 事件流
                for line in self._resp.iter_lines():
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
                
                # SSE 连接断开
                self._resp = None
                if not self._running:
                    break  # 主动停止，不重连
                if self.on_disconnected:
                    self.on_disconnected()
                print("[ntfy] ⚠️ SSE 连接断开，5 秒后重连...", file=sys.stderr)
                time.sleep(5)
                
            except requests.exceptions.ConnectionError:
                self._resp = None
                if not self._running:
                    break
                if self.on_disconnected:
                    self.on_disconnected()
                print("[ntfy] ⚠️ 网络连接失败，5 秒后重试...", file=sys.stderr)
                time.sleep(5)
            except Exception as e:
                self._resp = None
                if not self._running:  # 主动停止时的异常忽略
                    break
                if self.on_disconnected:
                    self.on_disconnected()
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
        subscriber = subscribe_ntfy("http://114.55.43.156:8080", "sms", "iPhone", "your_password")
        
        def on_message(msg):
            title = msg.get("title") or "ntfy 消息"
            message = msg.get("message") or str(msg)
            send_toast(title, message)
        
        subscriber.on_message = on_message
    """
    subscriber = NtfySSESubscriber(server, topic, username, password)
    subscriber.start()
    return subscriber
