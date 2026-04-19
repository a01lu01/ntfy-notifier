"""
系统托盘模块 - ntfy-Notifier
使用 pywin32 + ctypes 直接实现托盘图标
"""

import sys
from typing import Callable, Optional

# ── Win32 常量 ────────────────────────────────────────────────────────────
NIM_ADD    = 0
NIM_DELETE = 2
NIF_MESSAGE = 1
NIF_ICON    = 2
NIF_TIP     = 4
WM_USER     = 0x0400
WM_TRAY     = WM_USER + 1
ID_SETTINGS = 1
ID_QUIT     = 2

# ── 全局状态 ──────────────────────────────────────────────────────────────
_g_settings: Optional[Callable] = None
_g_quit:    Optional[Callable]  = None
_g_hwnd:    int = 0
_g_hmenu:   int = 0
_g_added:   bool = False
_g_hicon:   int = 0
_g_ok:      bool = False

# ── 图标创建 ──────────────────────────────────────────────────────────────
def _mkicon() -> int:
    try:
        from ctypes import windll
        S = 32
        hdc = windll.user32.GetDC(0)
        mdc = windll.gdi32.CreateCompatibleDC(hdc)
        hbm = windll.gdi32.CreateCompatibleBitmap(hdc, S, S)
        old = windll.gdi32.SelectObject(mdc, hbm)
        # 蓝色圆形图标
        br = windll.gdi32.CreateSolidBrush(0x00D47800)
        windll.gdi32.SelectObject(mdc, br)
        windll.gdi32.PatBlt(mdc, 0, 0, S, S, 0x00F00021)
        windll.gdi32.Ellipse(mdc, 1, 1, S-1, S-1)
        windll.gdi32.SelectObject(mdc, windll.gdi32.GetStockObject(5))
        windll.gdi32.Ellipse(mdc, 8, 8, S-8, S-8)
        windll.gdi32.SelectObject(mdc, old)
        hi = windll.user32.CreateIcon(
            windll.kernel32.GetModuleHandleW(None), S, S, 1, 32, 1, hbm, None)
        if not hi:
            hi = windll.user32.LoadImageW(0, 32512, 1, 16, 16, 0x8000 | 0x10)
        windll.gdi32.DeleteObject(hbm)
        windll.gdi32.DeleteObject(mdc)
        windll.gdi32.DeleteObject(br)
        windll.user32.ReleaseDC(0, hdc)
        return hi if hi else 0
    except:
        return 0

# ── 菜单 & 窗口过程 ───────────────────────────────────────────────────────
def _setup_menu():
    global _g_hmenu
    try:
        import win32gui
        _g_hmenu = win32gui.CreatePopupMenu()
        win32gui.AppendMenu(_g_hmenu, 0, ID_SETTINGS, "\u2699\ufe0f 设置...")
        win32gui.AppendMenu(_g_hmenu, 0, 0, "")
        win32gui.AppendMenu(_g_hmenu, 0, ID_QUIT, "\u274c\ufe0f 退出")
    except:
        pass

def _proc(hwnd, msg, wparam, lparam):
    global _g_settings, _g_quit, _g_hmenu
    import win32gui
    if msg == WM_TRAY:
        if lparam == 0x202:  # WM_LBUTTONUP
            if _g_settings:
                _g_settings()
        elif lparam == 0x205:  # WM_RBUTTONUP
            try:
                cur = win32gui.GetCursorPos()
                win32gui.SetForegroundWindow(hwnd)
                win32gui.TrackPopupMenu(_g_hmenu, 0, cur[0], cur[1], 0, hwnd, None)
                win32gui.PostMessage(hwnd, 0, 0, 0)
            except:
                pass
        elif lparam == 0x203:  # WM_LBUTTONDBLCLK
            if _g_settings:
                _g_settings()
    elif msg == 0x112 and wparam == ID_SETTINGS:  # WM_SYSCOMMAND
        if _g_settings:
            _g_settings()
    elif msg == 0x112 and wparam == ID_QUIT:
        if _g_quit:
            _g_quit()
    return win32gui.DefWindowProc(hwnd, msg, wparam, lparam)

def _make_class():
    try:
        import win32gui
        wc = win32gui.WNDCLASS()
        wc.lpfnWndProc = _proc
        wc.lpszClassName = "ntfy_TrayWin"
        wc.hInstance = win32gui.GetModuleHandle(None)
        result = win32gui.RegisterClass(wc)
        return result
    except Exception as e:
        return 0

def _make_tray_window():
    try:
        import win32gui
        cls = _make_class()
        hwnd = win32gui.CreateWindowEx(
            0, "ntfy_TrayWin", "ntfy Tray",
            0, 0, 0, 0, 0, 0, 0, None, None)
        return hwnd
    except Exception as e:
        return 0

# ── 添加 / 移除托盘图标 ────────────────────────────────────────────────────
def _add_tray():
    global _g_added
    if _g_added:
        return
    if not _g_hwnd:
        return
    try:
        from ctypes import Structure, c_uint, sizeof, byref, windll, POINTER, cast
        from ctypes.wintypes import HWND, HICON

        class NID(Structure):
            _fields_ = [
                ("cbSize",      c_uint),
                ("hwnd",        HWND),
                ("uID",         c_uint),
                ("uFlags",      c_uint),
                ("uCallback",   c_uint),
                ("hIcon",       HICON),
                ("szTip",       c_uint * 64),
                ("dwState",     c_uint),
                ("dwStateMask", c_uint),
                ("szInfo",      c_uint * 128),
                ("uTimeout",    c_uint),
                ("szInfoTitle", c_uint * 32),
                ("dwInfoFlags", c_uint),
            ]

        n = NID()
        cb = sizeof(NID)
        if not cb:
            cb = 936
        n.cbSize     = cb
        n.hwnd       = _g_hwnd
        n.uID        = 1
        n.uFlags     = NIF_MESSAGE | NIF_ICON | NIF_TIP
        n.uCallback  = WM_TRAY
        n.hIcon      = _g_hicon
        tip = "ntfy-Notifier"
        tip_arr = (c_uint * 64).from_buffer_copy(b"\x00" * 256)
        for i, ch in enumerate(tip):
            if i >= 64: break
            tip_arr[i] = ord(ch)
        n.szTip = tip_arr
        n.dwState      = 0
        n.dwStateMask  = 0
        n.uTimeout     = 0
        n.dwInfoFlags  = 0

        ret = windll.shell32.Shell_NotifyIconW(NIM_ADD, byref(n))
        if ret:
            _g_added = True
        else:
            err = windll.kernel32.GetLastError()
    except Exception as e:

def _remove_tray():
    global _g_added
    if not _g_added:
        return
    try:
        from ctypes import Structure, c_uint, sizeof, byref, windll
        from ctypes.wintypes import HWND, HICON

        class NID(Structure):
            _fields_ = [
                ("cbSize",      c_uint),
                ("hwnd",        HWND),
                ("uID",         c_uint),
                ("uFlags",      c_uint),
                ("uCallback",   c_uint),
                ("hIcon",       HICON),
                ("szTip",       c_uint * 64),
                ("dwState",     c_uint),
                ("dwStateMask", c_uint),
                ("szInfo",      c_uint * 128),
                ("uTimeout",    c_uint),
                ("szInfoTitle", c_uint * 32),
                ("dwInfoFlags", c_uint),
            ]

        n = NID()
        n.cbSize = sizeof(NID) or 936
        n.hwnd   = _g_hwnd
        n.uID    = 1
        windll.shell32.Shell_NotifyIconW(NIM_DELETE, byref(n))
        _g_added = False
    except:
        pass

def _pump():
    try:
        import win32gui
        from ctypes import windll, byref, c_int, c_ulong, Structure
        class MSG(Structure):
            _fields_ = [
                ("hwnd",    c_int),
                ("message", c_int),
                ("wParam",  c_int),
                ("lParam",  c_int),
                ("time",    c_int),
                ("pt_x",    c_int),
                ("pt_y",    c_int),
            ]
        msg = MSG()
        while _g_ok:
            while windll.user32.PeekMessageW(byref(msg), 0, 0, 0, 1):
                if not _g_ok:
                    return
                win32gui.TranslateMessage(byref(msg))
                win32gui.DispatchMessageW(byref(msg))
    except:
        pass

class TrayIcon:
    def __init__(self, on_settings=None, on_quit=None):
        global _g_settings, _g_quit, _g_hwnd, _g_hicon, _g_ok
        _g_settings = on_settings
        _g_quit = on_quit
        try:
            import win32gui
            _g_hicon = _mkicon()
            _setup_menu()
            _g_hwnd = _make_tray_window()
        except Exception as e:

    def start(self, connected=False) -> bool:
        global _g_ok
        try:
            import threading

            def pump_with_tray():
                _add_tray()
                _pump()

            t = threading.Thread(target=pump_with_tray, daemon=True)
            t.daemon = True
            t.start()
            _g_ok = True
            return True
        except:
            return False

    def update(self, connected: bool):
        pass  # stub for future connection status indicator

    def stop(self):
        global _g_ok, _g_added
        _g_ok = False
        try:
            import threading, time
            _remove_tray()
            time.sleep(0.1)
            if _g_hwnd:
                import win32gui
                win32gui.DestroyWindow(_g_hwnd)
        except:
            pass
