"""
Windows 通知模块 - ntfy-Notifier
通知优先级：
  1. winotify（WinRT Toast 通知，支持 AUMID 图标 + 声音）
  2. plyer（Windows Toast 通知，无 AUMID 支持）
  3. win32gui.MessageBox（弹窗，有声音）
  4. print stderr（后备）

订阅模式：SSE (Server-Sent Events) — 实时推送，无需轮询

修复记录：
- on_connected 改为在收到 SSE event:open 时触发（而非 HTTP 200）
- 添加连接超时 (10s) 和读取超时 (90s)
- 添加指数退避重连策略
- 回调异常保护，防止回调异常导致订阅循环崩溃
- 添加健康检查线程：跟踪最后收到数据时间，超时则主动断开重连
"""

import sys
import json
import re
import traceback
import threading
import time
from typing import Callable, Optional

try:
    import win32clipboard
    _CLIPBOARD_AVAILABLE = True
except ImportError:
    _CLIPBOARD_AVAILABLE = False


_OTP_KEYWORDS = re.compile(r"(验证码|动态码|校验码|安全码|一次性密码|OTP)")
_OTP_DIGITS = re.compile(r"(?<![0-9])(\d{4,8})(?![0-9])")


def _extract_otp(text: str) -> Optional[str]:
    """从消息文本中提取 4-8 位纯数字验证码。

    优先取“验证码”等关键词后 30 字符内的独立数字段，
    找不到再取全文首个独立数字段；只匹配纯数字，避免把
    Google 等英文词误当验证码。
    """
    if not text:
        return None

    for m in _OTP_KEYWORDS.finditer(text):
        window = text[m.end():m.end() + 30]
        match = _OTP_DIGITS.search(window)
        if match:
            return match.group(1)

    match = _OTP_DIGITS.search(text)
    return match.group(1) if match else None


def _copy_to_clipboard(text: str) -> bool:
    """将文本复制到系统剪切板（win32clipboard，带重试）。"""
    if not _CLIPBOARD_AVAILABLE:
        return False
    import ctypes
    ctypes.windll.ole32.CoInitialize(None)
    try:
        opened = False
        last_error = None
        for attempt in range(1, 4):
            try:
                win32clipboard.OpenClipboard()
                opened = True
                break
            except Exception as e:
                last_error = e
                time.sleep(0.1)
        if not opened:
            print(f"[ntfy-Notifier] ❌ 打开剪贴板失败: {last_error}", file=sys.stderr)
            return False
        try:
            win32clipboard.EmptyClipboard()
            win32clipboard.SetClipboardText(text, win32clipboard.CF_UNICODETEXT)
        finally:
            win32clipboard.CloseClipboard()
        return True
    except Exception as e:
        print(f"[ntfy-Notifier] ❌ 写入剪贴板失败: {e}", file=sys.stderr)
        return False
    finally:
        ctypes.windll.ole32.CoUninitialize()


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


def _send_winotify_toast(title: str, message: str, app_id: str = "ntfy-Notifier", auto_copy_otp: bool = False) -> bool:
    """使用 winotify 发送 WinRT Toast 通知（通过 AUMID 显示应用图标）。"""
    try:
        # 如果启用了自动复制验证码，提取并写入剪切板（后台线程）
        if auto_copy_otp:
            otp = _extract_otp(message)
            if otp:
                print(f"[ntfy-Notifier] OTP 提取成功: {otp}, 正在写入剪切板...", file=sys.stderr)
                def _do_clipboard():
                    try:
                        _copy_to_clipboard(otp)
                        print(f"[ntfy-Notifier] ✅ OTP 已写入剪切板", file=sys.stderr)
                    except Exception as e:
                        print(f"[ntfy-Notifier] ❌ OTP 写入剪切板失败: {e}", file=sys.stderr)
                threading.Thread(target=_do_clipboard, daemon=True).start()
            else:
                print(f"[ntfy-Notifier] ⚠️ OTP 提取失败，消息内容: {message[:100]}", file=sys.stderr)

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
    from xml.sax.saxutils import escape
    xml_string = (
        f'<toast activationType="protocol">'
        f'<visual><binding template="ToastGeneric">'
        f'<text>{escape(title)}</text>'
        f'<text>{escape(message)}</text>'
        f'</binding></visual>'
        f'<audio src="ms-winsoundevent:Notification.IM" />'
        f'</toast>'
    )
    doc = XmlDocument()
    doc.LoadXml(xml_string)
    return doc


def send_toast(title: str, message: str, app_id: str = "ntfy-Notifier", auto_copy_otp: bool = False) -> bool:
    """
    发送 Windows 通知。

    优先级：winotify → plyer → winrt Toast → win32gui MessageBox → print stderr
    每个后端失败（返回 False 或抛异常）都会继续尝试下一级。
    """
    if _WINOTIFY_AVAILABLE:
        try:
            if _send_winotify_toast(title, message, app_id, auto_copy_otp):
                return True
        except Exception:
            traceback.print_exc()

    if _PLYER_AVAILABLE:
        try:
            if _send_plyer_toast(title, message):
                return True
        except Exception:
            traceback.print_exc()

    if _WINRT_AVAILABLE:
        try:
            notifier = ToastNotificationManager.create_notifier(app_id)
            toast = ToastNotification(_create_toast_xml(title, message))
            notifier.show(toast)
            return True
        except Exception:
            traceback.print_exc()

    if _WIN32GUI_AVAILABLE:
        try:
            win32gui.MessageBox(0, message, title, _MB_ICONINFORMATION | _MB_OK)
            return True
        except Exception:
            traceback.print_exc()

    print(f"[ntfy-Notifier 通知] {title}: {message}", file=sys.stderr)
    return False


# ── SSE 常量 ──────────────────────────────────────────────────────────────

# SSE 读取超时（秒）：如果超过此时间没收到任何数据，认为连接已死
_SSE_READ_TIMEOUT = 90
# SSE 连接超时（秒）
_SSE_CONNECT_TIMEOUT = 10
# 指数退避参数
_RETRY_DELAY_INIT = 5      # 初始重连延迟（秒）
_RETRY_DELAY_MAX = 300     # 最大重连延迟（秒）
# 健康检查参数
_HEALTH_CHECK_INTERVAL = 60   # 健康检查间隔（秒）
_HEALTH_CHECK_TIMEOUT = 120   # 无数据超时阈值（秒）


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
        self.on_connected = on_connected        # SSE 连接成功回调（event:open 时触发）
        self.on_disconnected = on_disconnected  # SSE 连接断开回调
        
        self._running = False
        self._thread: Optional[threading.Thread] = None
        self._session_id: Optional[str] = None
        self._resp = None  # 保存响应对象以便关闭
        self._resp_lock = threading.Lock()
        self._retry_delay = _RETRY_DELAY_INIT   # 指数退避当前延迟

        # 健康检查相关
        self._last_data_time: float = 0         # 最后收到 SSE 数据的时间
        self._connected_flag: bool = False       # SSE 是否已收到 event:open
        self._health_thread: Optional[threading.Thread] = None

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

        # 启动健康检查线程
        self._health_thread = threading.Thread(
            target=self._health_check_loop,
            daemon=True,
            name="NtfyHealthThread",
        )
        self._health_thread.start()

    def stop(self):
        """停止 SSE 订阅。"""
        self._running = False
        # 关闭响应以中断 iter_lines 阻塞
        self._close_resp()
        if self._thread:
            self._thread.join(timeout=5)
        # 健康检查线程是 daemon 的，_running=False 后会自行退出

    def _close_resp(self):
        """加锁关闭并清空当前响应，中断 iter_lines 阻塞。"""
        with self._resp_lock:
            resp = self._resp
            self._resp = None
        if resp is not None:
            try:
                resp.close()
            except Exception:
                pass

    def _notify_disconnected(self):
        """安全地触发 on_disconnected 回调。"""
        self._connected_flag = False
        if self.on_disconnected:
            try:
                self.on_disconnected()
            except Exception as e:
                print(f"[ntfy] on_disconnected 回调异常: {e}", file=sys.stderr)

    def _notify_connected(self):
        """安全地触发 on_connected 回调。"""
        self._connected_flag = True
        if self.on_connected:
            try:
                self.on_connected()
            except Exception as e:
                print(f"[ntfy] on_connected 回调异常: {e}", file=sys.stderr)

    def _wait_with_backoff(self):
        """指数退避等待，连接成功后重置延迟。"""
        delay = self._retry_delay
        self._retry_delay = min(self._retry_delay * 2, _RETRY_DELAY_MAX)
        # 分段等待，以便及时响应 stop()
        end_time = time.time() + delay
        while self._running and time.time() < end_time:
            time.sleep(min(1, end_time - time.time()))

    def _reset_backoff(self):
        """连接成功后重置退避延迟。"""
        self._retry_delay = _RETRY_DELAY_INIT

    def _health_check_loop(self):
        """健康检查循环：检测僵尸连接并主动重连。

        如果 SSE 连接声称已连接但超过 _HEALTH_CHECK_TIMEOUT 秒没收到
        任何数据，说明连接可能已死（TCP 半开），主动关闭连接触发重连。
        """
        while self._running:
            time.sleep(_HEALTH_CHECK_INTERVAL)

            if not self._running:
                break

            # 只在连接声称已连接时检查
            if not self._connected_flag:
                continue

            now = time.time()
            elapsed = now - self._last_data_time

            if self._last_data_time > 0 and elapsed > _HEALTH_CHECK_TIMEOUT:
                print(
                    f"[ntfy] 健康检查：超过 {elapsed:.0f}s 未收到数据，"
                    f"怀疑僵尸连接，主动断开重连",
                    file=sys.stderr,
                )
                # 关闭响应以中断 iter_lines，触发重连
                self._close_resp()

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
                    timeout=(_SSE_CONNECT_TIMEOUT, _SSE_READ_TIMEOUT),
                    proxies={"http": None, "https": None},
                    stream=True,
                )
                with self._resp_lock:
                    self._resp = resp
                
                if self._resp.status_code != 200:
                    print(f"[ntfy] ⚠️ SSE 连接失败：HTTP {self._resp.status_code}", file=sys.stderr)
                    self._close_resp()
                    self._notify_disconnected()
                    self._wait_with_backoff()
                    continue
                
                # 注意：不再在 HTTP 200 时立即触发 on_connected
                # 等收到 SSE event:open 才确认连接成功
                print("[ntfy] SSE HTTP 200，等待 event:open 确认...", file=sys.stderr)
                
                # 重置数据时间戳
                self._last_data_time = time.time()

                # 解析 SSE 事件流
                for line in self._resp.iter_lines():
                    if not self._running:
                        break
                    
                    # 每收到任何一行数据，更新时间戳
                    self._last_data_time = time.time()

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
                                # 收到 open 事件才确认连接成功
                                self._notify_connected()
                                self._reset_backoff()
                                print("[ntfy] SSE 已连接，等待消息...", file=sys.stderr)
                            elif event_type == "keepalive":
                                # 心跳消息，无需特殊处理
                                pass
                            elif event_type == "message":
                                # 收到新消息，触发回调
                                if self.on_message:
                                    self.on_message(msg)
                    
                    except json.JSONDecodeError:
                        pass  # 忽略非 JSON 行（如注释、空行）
                    except UnicodeDecodeError:
                        pass
                
                # SSE 连接断开
                self._close_resp()
                if not self._running:
                    break  # 主动停止，不重连
                self._notify_disconnected()
                print(f"[ntfy] SSE 连接断开，{self._retry_delay} 秒后重连...", file=sys.stderr)
                self._wait_with_backoff()
                
            except requests.exceptions.ConnectionError:
                self._close_resp()
                if not self._running:
                    break
                self._notify_disconnected()
                print(f"[ntfy] 网络连接失败，{self._retry_delay} 秒后重试...", file=sys.stderr)
                self._wait_with_backoff()
            except requests.exceptions.Timeout:
                self._close_resp()
                if not self._running:
                    break
                self._notify_disconnected()
                print(f"[ntfy] SSE 读取超时（{_SSE_READ_TIMEOUT}s），重连...", file=sys.stderr)
                # 读取超时时重置退避延迟
                self._retry_delay = _RETRY_DELAY_INIT
                time.sleep(1)
            except Exception as e:
                self._close_resp()
                if not self._running:  # 主动停止时的异常忽略
                    break
                self._notify_disconnected()
                print(f"[ntfy] SSE 错误：{type(e).__name__}: {e}", file=sys.stderr)
                self._wait_with_backoff()


# ── 便捷函数 ──────────────────────────────────────────────────────────────

def subscribe_ntfy(server: str, topic: str, username: str = "", password: str = "") -> NtfySSESubscriber:
    """
    创建并启动 ntfy SSE 订阅器。
    
    Args:
        server:   ntfy 服务器地址，例如 http://your-server:8080
        topic:    订阅话题，例如 sms
        username: 用户名（可选）
        password: 密码（可选）
    
    Returns:
        NtfySSESubscriber 实例
    
    Example:
        subscriber = subscribe_ntfy("http://your-server:8080", "sms", "your_username", "your_password")
        
        def on_message(msg):
            title = msg.get("title") or "ntfy 消息"
            message = msg.get("message") or str(msg)
            send_toast(title, message)
        
        subscriber.on_message = on_message
    """
    subscriber = NtfySSESubscriber(server, topic, username, password)
    subscriber.start()
    return subscriber
