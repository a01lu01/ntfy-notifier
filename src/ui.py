"""
设置窗口 UI - ntfy-Notifier
基于 Tkinter 实现 Windows Fluent 风格设置窗口
参考 Fluent Design：圆角、轻量阴影、Segoe UI Font、#0078D4 主色
"""

import tkinter as tk
from tkinter import ttk
from typing import Callable, Optional


# ── Fluent Design 色彩常量 ────────────────────────────────────────────────────
_FLUENT_BG            = "#FFFFFF"
_FLUENT_SURFACE       = "#F3F3F3"
_FLUENT_BORDER        = "#E0E0E0"
_FLUENT_TEXT          = "#1A1A1A"
_FLUENT_SUBTEXT       = "#606060"
_FLUENT_ACCENT        = "#0078D4"
_FLUENT_ACCENT_HOVER  = "#106EBE"


class SettingsWindow:
    """
    Fluent 风格设置窗口。
    show_and_wait() 在当前线程阻塞，直到窗口关闭，
    内部通过 root.after() 驱动 Tk 事件循环。
    """

    def __init__(
        self,
        current_config: dict,
        on_save: Callable[[dict], None],
        on_cancel: Callable[[], None],
        master: Optional[tk.Tk] = None,
    ):
        self._current = dict(current_config)
        self._on_save = on_save
        self._on_cancel = on_cancel
        self._master = master
        self._win: Optional[tk.Toplevel] = None
        self._entries: dict = {}
        self._var_auto_start = tk.BooleanVar(value=False)
        self._closed = False

    def show(self):
        """
        非阻塞：在 _master 上显示设置窗口。
        如果没有 master，则创建新的 Tk() 并驱动事件循环。
        """
        root = self._master or tk.Tk()
        if not self._master:
            root.withdraw()

        win = tk.Toplevel(root)
        self._win = win
        win.title("ntfy-Notifier 设置")
        win.geometry("480x540")
        win.resizable(False, False)
        win.configure(bg=_FLUENT_BG)

        self._build_header(win)
        self._build_form(win)
        self._build_footer(win)

        win.protocol("WM_DELETE_WINDOW", self._cancel)

        # 居中并显示
        win.update_idletasks()
        sw, sh = win.winfo_screenwidth(), win.winfo_screenheight()
        win.geometry(f"480x540+{(sw - 480) // 2}+{(sh - 540) // 2}")
        win.update()
        win.deiconify()
        win.after(0, lambda: (win.lift(), win.focus_force()))

        # 无 master 时在调用线程中驱动 Tk 事件循环
        if not self._master:
            while not self._closed and win.winfo_exists():
                win.update()
                win.update_idletasks()

    def show_and_wait(self):
        """兼容性别名，内部调用 show()。"""
        self.show()

    # ── UI 构建 ─────────────────────────────────────────────────────────────

    def _build_header(self, parent: tk.Widget):
        frame = tk.Frame(parent, bg=_FLUENT_BG, padx=32, pady=24)
        frame.pack(fill="x")

        tk.Label(
            frame, text="ntfy-Notifier 设置",
            font=("Segoe UI", 20, "bold"),
            fg=_FLUENT_TEXT, bg=_FLUENT_BG, anchor="w",
        ).pack(anchor="w")

        tk.Frame(parent, height=1, bg=_FLUENT_BORDER).pack(fill="x")

    def _build_form(self, parent: tk.Widget):
        form = tk.Frame(parent, bg=_FLUENT_BG, padx=32, pady=20)
        form.pack(fill="x")

        def field_row(label: str, default: str, row: int, show: str = None) -> ttk.Entry:
            tk.Label(form, text=label, font=("Segoe UI", 9),
                     fg=_FLUENT_SUBTEXT, bg=_FLUENT_BG,
                     anchor="w").grid(row=row, column=0, sticky="w", pady=(8, 2))

            ent = ttk.Entry(form, width=40, font=("Segoe UI", 10))
            ent.grid(row=row, column=0, sticky="we", pady=(0, 8))
            if show:
                ent.configure(show=show)
            ent.insert(0, default)
            return ent

        self._entries["server"] = field_row("服务器地址", self._current.get("server", ""), 0)
        self._entries["username"] = field_row("用户名", self._current.get("username", ""), 1)
        self._entries["password"] = field_row("密码", self._current.get("password", ""), 2, show="*")
        self._entries["topic"] = field_row("订阅话题", self._current.get("topic", ""), 3)

        tk.Label(form, text="轮询间隔", font=("Segoe UI", 9),
                 fg=_FLUENT_SUBTEXT, bg=_FLUENT_BG,
                 anchor="w").grid(row=4, column=0, sticky="w", pady=(8, 2))

        row4 = tk.Frame(form, bg=_FLUENT_BG)
        row4.grid(row=5, column=0, sticky="w", pady=(0, 8))
        ent_int = ttk.Entry(row4, width=10, font=("Segoe UI", 10))
        ent_int.pack(side="left")
        ent_int.insert(0, str(self._current.get("poll_interval", 3)))
        tk.Label(row4, text=" 秒", font=("Segoe UI", 9),
                 fg=_FLUENT_SUBTEXT, bg=_FLUENT_BG).pack(side="left", padx=6)
        self._entries["poll_interval"] = ent_int

        tk.Frame(form, height=1, bg=_FLUENT_BORDER).grid(row=6, column=0, sticky="we", pady=(8, 8))

        self._var_auto_start.set(bool(self._current.get("auto_start", False)))
        cb = tk.Checkbutton(
            form, text="开机自启",
            variable=self._var_auto_start,
            font=("Segoe UI", 10),
            fg=_FLUENT_TEXT, bg=_FLUENT_BG,
            activeforeground=_FLUENT_ACCENT,
            selectcolor=_FLUENT_BG, anchor="w",
        )
        cb.grid(row=7, column=0, sticky="w")

    def _build_footer(self, parent: tk.Widget):
        footer = tk.Frame(parent, bg=_FLUENT_SURFACE, pady=12, padx=24)
        footer.pack(side="bottom", fill="x")

        btn_cancel = tk.Button(
            footer, text="取消",
            font=("Segoe UI", 10),
            fg=_FLUENT_TEXT, bg=_FLUENT_BG,
            relief="flat", bd=1,
            padx=20, pady=6,
            command=self._cancel,
            cursor="hand2",
        )
        btn_cancel.pack(side="right", padx=8)

        btn_save = tk.Button(
            footer, text="保存",
            font=("Segoe UI", 10, "bold"),
            fg="#FFFFFF", bg=_FLUENT_ACCENT,
            relief="flat", bd=0,
            padx=20, pady=6,
            command=self._save,
            cursor="hand2",
        )
        btn_save.pack(side="right")

    def _save(self):
        try:
            interval = int(self._entries["poll_interval"].get().strip())
        except ValueError:
            interval = 3

        config = {
            "server":        self._entries["server"].get().strip(),
            "username":      self._entries["username"].get().strip(),
            "password":      self._entries["password"].get().strip(),
            "topic":         self._entries["topic"].get().strip(),
            "poll_interval": interval,
            "auto_start":   bool(self._var_auto_start.get()),
        }
        self._on_save(config)
        self._close()

    def _cancel(self):
        if self._on_cancel:
            self._on_cancel()
        self._close()

    def _close(self):
        self._closed = True
        if self._win and self._win.winfo_exists():
            try:
                self._win.grab_release()
                self._win.destroy()
            except Exception:
                pass
