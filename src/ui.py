"""
设置窗口 UI - ntfy-Notifier
基于 Tkinter 实现 Windows 11 Fluent Design 风格
跟随系统明/暗模式，使用 Windows 原生色彩
"""

import tkinter as tk
from tkinter import ttk
from typing import Callable, Optional


# ── Windows Fluent Design 色彩系统 ────────────────────────────────────────────

class _Theme:
    """跟随 Windows 系统明/暗模式的主题色管理器。"""

    _REG_KEY  = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
    _REG_VAL  = "AppsUseLightTheme"
    _POLL_MS  = 1000   # 主题检查轮询间隔（ms）

    @property
    def is_dark(self) -> bool:
        return self._is_dark

    def __init__(self):
        self._is_dark: bool = self._read_registry()
        self._cbs: list[Callable[[bool], None]] = []
        self._timer: str | None = None
        self._win: Optional[tk.Tk | tk.Toplevel] = None

    # ── 颜色表 ─────────────────────────────────────────────────────────────

    def colors(self, dark: bool | None = None) -> dict[str, str]:
        d = dark if dark is not None else self._is_dark
        if d:
            return dict(
                bg           = "#1F1F1F",
                surface      = "#2D2D2D",
                border       = "#3D3D3D",
                text         = "#FFFFFF",
                subtext      = "#9D9D9D",
                accent       = "#60CDFF",
                accent_hover = "#7AD4FF",
                accent_press = "#4DB8E8",
                btn_save_bg  = "#0078D4",
                btn_save_fg  = "#FFFFFF",
                btn_cancel_bg= "#2D2D2D",
                btn_cancel_fg= "#FFFFFF",
                input_bg     = "#252525",
                input_fg     = "#FFFFFF",
                input_border = "#555555",
                input_sel    = "#3A3A3A",
                scroll_bg    = "#2D2D2D",
                scroll_thumb = "#555555",
                title_bar    = "#1F1F1F",
            )
        else:
            return dict(
                bg           = "#F3F3F3",
                surface      = "#FFFFFF",
                border       = "#E5E5E5",
                text         = "#1A1A1A",
                subtext      = "#606060",
                accent       = "#0078D4",
                accent_hover = "#106EBE",
                accent_press = "#005A9E",
                btn_save_bg  = "#0078D4",
                btn_save_fg  = "#FFFFFF",
                btn_cancel_bg= "#FFFFFF",
                btn_cancel_fg= "#1A1A1A",
                input_bg     = "#FFFFFF",
                input_fg     = "#1A1A1A",
                input_border = "#CCCCCC",
                input_sel    = "#E5F3FF",
                scroll_bg    = "#F0F0F0",
                scroll_thumb = "#C8C8C8",
                title_bar    = "#F3F3F3",
            )

    # ── 注册表读取 ──────────────────────────────────────────────────────────

    def _read_registry(self) -> bool:
        try:
            import winreg
            key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, self._REG_KEY, 0, winreg.KEY_READ)
            try:
                val, _ = winreg.QueryValueEx(key, self._REG_VAL)
                return val == 0
            finally:
                winreg.CloseKey(key)
        except Exception:
            return False

    def _check_change(self):
        new = self._read_registry()
        if new != self._is_dark:
            self._is_dark = new
            for cb in self._cbs:
                try: cb(new)
                except Exception: pass
        if self._win and self._win.winfo_exists():
            self._timer = self._win.after(self._POLL_MS, self._check_change)

    def watch(self, win: tk.Tk | tk.Toplevel, on_change: Callable[[bool], None]):
        self._win = win
        self._cbs.append(on_change)
        self._timer = win.after(self._POLL_MS, self._check_change)

    def unwatch(self):
        if self._timer and self._win:
            try: self._win.after_cancel(self._timer)
            except Exception: pass
        self._timer = None


_theme = _Theme()


# ── ttk 样式（跟随主题） ─────────────────────────────────────────────────────

def _style_ttk(is_dark: bool):
    s = ttk.Style()
    s.theme_use("clam")
    c = _theme.colors(is_dark)
    s.configure(".", background=c["bg"], foreground=c["text"], fieldbackground=c["input_bg"])
    # TEntry
    s.configure("TEntry",
                fieldbackground=c["input_bg"],
                foreground=c["input_fg"],
                bordercolor=c["input_border"],
                lightcolor=c["input_border"],
                darkcolor=c["input_border"])
    s.map("TEntry",
          fieldbackground=[("focus", c["input_bg"])],
          bordercolor=[("focus", c["accent"])],
          lightcolor=[("focus", c["accent"])],
          darkcolor=[("focus", c["accent"])])
    # TCheckbutton
    s.configure("TCheckbutton",
                background=c["bg"], foreground=c["text"])
    s.map("TCheckbutton",
          background=[("active", c["bg"])],
          indicatorcolor=[("selected", c["accent"]), ("!selected", c["surface"])])


# ── SettingsWindow ───────────────────────────────────────────────────────────

class SettingsWindow:

    def __init__(
        self,
        current_config: dict,
        on_save: Callable[[dict], None],
        on_cancel: Callable[[], None] | None,
        master: Optional[tk.Tk] = None,
    ):
        self._cfg    = dict(current_config)
        self._save   = on_save
        self._cancel = on_cancel
        self._master = master
        self._win: tk.Toplevel | None = None
        self._entries: dict[str, tk.Widget] = {}
        self._var_autostart = tk.BooleanVar(value=bool(current_config.get("auto_start", False)))
        self._closed = False
        self._dark   = _theme.is_dark

    # ── 窗口创建 ─────────────────────────────────────────────────────────────

    def show(self):
        root = self._master or tk.Tk()
        if not self._master:
            root.withdraw()

        win = tk.Toplevel(root)
        self._win = win
        self._dark = _theme.is_dark
        _style_ttk(self._dark)

        win.title("ntfy-Notifier 设置")
        win.geometry("460x540")
        win.resizable(False, False)
        win.transient(root)
        win.protocol("WM_DELETE_WINDOW", self._cancel_or_close)

        self._build_all(win)
        self._recolor(win)

        # 居中并显示
        win.update_idletasks()
        sw, sh = win.winfo_screenwidth(), win.winfo_screenheight()
        ww, wh = 460, 540
        win.geometry(f"{ww}x{wh}+{(sw-ww)//2}+{(sh-wh)//2}")

        win.update()
        win.deiconify()
        win.after(0, lambda: (win.lift(), win.focus_force()))

        # 监听系统主题变化
        _theme.watch(win, self._on_theme_changed)

        if not self._master:
            while not self._closed and win.winfo_exists():
                win.update()
                win.update_idletasks()

    def show_and_wait(self):
        self.show()

    # ── 主题切换回调 ────────────────────────────────────────────────────────

    def _on_theme_changed(self, is_dark: bool):
        if self._dark == is_dark:
            return
        self._dark = is_dark
        _style_ttk(is_dark)
        if self._win:
            self._recolor(self._win)

    # ── 颜色重应用 ─────────────────────────────────────────────────────────

    def _recolor(self, win: tk.Toplevel):
        c = _theme.colors(self._dark)
        win.configure(bg=c["bg"])

        def walk(w: tk.Widget):
            cls = w.winfo_class()
            try:
                opts = dict(w.configure())
                cur_bg = opts.get("background", c["bg"])
                is_surface = str(cur_bg) in ("#FFFFFF", "#2D2D2D", "#F3F3F3", "#1F1F1F",
                                               c["surface"], c["bg"], "surface", "bg")
                is_header  = str(cur_bg) in (c["bg"], "bg")

                if cls == "Frame":
                    w.configure(bg=c["bg"])
                elif cls == "Label":
                    fg = c["subtext"]
                    for child in w.pack_slaves() if hasattr(w, 'pack_slaves') else []:
                        pass
                    # 标题行（大字）
                    font_str = str(opts.get("font", ""))
                    if "Segoe UI" in font_str:
                        size = int(font_str.split()[-2]) if len(font_str.split()) >= 2 else 0
                        if size >= 16:
                            fg = c["text"]
                    w.configure(fg=fg)
                elif cls == "Entry":
                    w.configure(
                        bg=c["input_bg"], fg=c["input_fg"],
                        insertcolor=c["accent"],
                        selectbackground=c["input_sel"],
                        highlightbackground=c["input_border"],
                    )
                elif cls == "Checkbutton":
                    w.configure(
                        fg=c["text"], bg=c["bg"],
                        activeforeground=c["accent"],
                        selectcolor=c["bg"],
                    )
            except tk.TclError:
                pass
            try:
                for ch in w.winfo_children():
                    walk(ch)
            except Exception:
                pass

        walk(win)
        # 单独处理 ttk Entry 的前景/背景
        for w in win.winfo_children():
            _recolor_ttk(w, self._dark)

    # ── UI 构建 ─────────────────────────────────────────────────────────────

    def _build_all(self, parent: tk.Widget):
        c = _theme.colors(self._dark)

        # 标题栏
        title_bar = tk.Frame(parent, bg=c["bg"], height=52, pady=0)
        title_bar.pack(fill="x")
        title_bar.pack_propagate(False)

        title_lbl = tk.Label(
            title_bar, text="ntfy-Notifier 设置",
            font=("Segoe UI Variable", 16, "bold"),
            fg=c["text"], bg=c["bg"], anchor="w", padx=24,
        )
        title_lbl.pack(side="left", fill="both", expand=True, pady=16)

        # 分隔线
        tk.Frame(parent, height=1, bg=c["border"]).pack(fill="x")

        # 表单区域（可滚动）
        canvas = tk.Canvas(parent, bg=c["bg"], highlightthickness=0, bd=0)
        scroll = ttk.Scrollbar(parent, orient="vertical", command=canvas.yview)
        form_wrap = tk.Frame(canvas, bg=c["bg"], padx=24, pady=8)

        canvas.create_window((0, 0), window=form_wrap, anchor="nw")
        form_wrap.bind(
            "<Configure>",
            lambda _: canvas.configure(scrollregion=canvas.bbox("all")),
        )
        canvas.configure(yscrollcommand=scroll.set)

        canvas.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")

        # 配置表单字段
        fields = [
            ("服务器地址",   "server",     "http://example.com",  False),
            ("用户名",       "username",   "",                    False),
            ("密码",         "password",   "",                    True),
            ("订阅话题",     "topic",      "sms",                 False),
        ]
        for lbl_text, key, default, is_password in fields:
            self._field_row(form_wrap, lbl_text, key, default, is_password)

        # 轮询间隔
        self._interval_row(form_wrap)

        # 分隔线
        tk.Frame(form_wrap, height=1, bg=c["border"]).pack(
            fill="x", pady=16, ipady=0,
        )

        # 开机自启
        cb_frame = tk.Frame(form_wrap, bg=c["bg"])
        cb_frame.pack(fill="x", pady=4)
        cb = ttk.Checkbutton(
            cb_frame, text="开机自动启动",
            variable=self._var_autostart,
            style="TCheckbutton",
        )
        cb.pack(side="left")

        # 分隔线
        tk.Frame(parent, height=1, bg=c["border"]).pack(fill="x", side="bottom")

        # 底部按钮栏
        footer = tk.Frame(parent, bg=c["surface"], padx=24, pady=12)
        footer.pack(fill="x", side="bottom")

        btn_cancel = tk.Button(
            footer, text="取消",
            font=("Segoe UI Variable", 9),
            fg=c["btn_cancel_fg"], bg=c["btn_cancel_bg"],
            relief="flat", bd=0,
            padx=16, pady=6,
            command=self._cancel_or_close,
            cursor="hand2",
            activebackground=c["border"],
            activeforeground=c["btn_cancel_fg"],
        )
        btn_cancel.pack(side="right", padx=(8, 0))

        btn_save = tk.Button(
            footer, text="保存",
            font=("Segoe UI Variable", 9, "bold"),
            fg=c["btn_save_fg"], bg=c["btn_save_bg"],
            relief="flat", bd=0,
            padx=20, pady=6,
            command=self._on_save,
            cursor="hand2",
            activebackground=c["accent_press"],
            activeforeground=c["btn_save_fg"],
        )
        btn_save.pack(side="right")

    def _field_row(self, parent: tk.Widget, label: str, key: str,
                   default: str = "", is_password: bool = False):
        """一行：上方标签 + 下方输入框。"""
        c = _theme.colors(self._dark)

        row = tk.Frame(parent, bg=c["bg"])
        row.pack(fill="x", pady=(12, 0))

        # 标签
        lbl = tk.Label(
            row, text=label,
            font=("Segoe UI Variable", 9),
            fg=c["subtext"], bg=c["bg"], anchor="w",
        )
        lbl.pack(anchor="w", pady=(0, 4))

        # 输入框容器（带边框）
        entry_wrap = tk.Frame(row, bg=c["input_border"], padx=1, pady=1)
        entry_wrap.pack(fill="x")

        show_char = "*" if is_password else None
        ent = tk.Entry(
            entry_wrap,
            font=("Segoe UI Variable", 10),
            bg=c["input_bg"], fg=c["input_fg"],
            insertcolor=c["accent"],
            selectbackground=c["input_sel"],
            relief="flat", bd=0,
            highlightthickness=0,
            textvariable=tk.StringVar(value=self._cfg.get(key, default)),
        )
        if show_char:
            ent.configure(show=show_char)
        ent.pack(fill="x", ipady=6, ipadx=8)

        self._entries[key] = ent
        return row

    def _interval_row(self, parent: tk.Widget):
        """轮询间隔行：标签 + 输入框 + 单位。"""
        c = _theme.colors(self._dark)

        row = tk.Frame(parent, bg=c["bg"])
        row.pack(fill="x", pady=(12, 0))

        lbl = tk.Label(
            row, text="轮询间隔",
            font=("Segoe UI Variable", 9),
            fg=c["subtext"], bg=c["bg"], anchor="w",
        )
        lbl.pack(anchor="w", pady=(0, 4))

        # 输入框 + 单位 横向排列
        pair = tk.Frame(row, bg=c["bg"])
        pair.pack(anchor="w")

        entry_wrap = tk.Frame(pair, bg=c["input_border"], padx=1, pady=1)
        entry_wrap.pack(side="left")

        default_interval = str(self._cfg.get("poll_interval", 3))
        ent = tk.Entry(
            entry_wrap,
            font=("Segoe UI Variable", 10),
            bg=c["input_bg"], fg=c["input_fg"],
            insertcolor=c["accent"],
            selectbackground=c["input_sel"],
            relief="flat", bd=0,
            highlightthickness=0,
            width=8,
            textvariable=tk.StringVar(value=default_interval),
        )
        ent.pack(side="left", ipady=6, ipadx=8)

        unit = tk.Label(
            pair, text=" 秒",
            font=("Segoe UI Variable", 10),
            fg=c["subtext"], bg=c["bg"],
        )
        unit.pack(side="left", padx=(8, 0))

        self._entries["poll_interval"] = ent

    def _on_save(self):
        c = _theme.colors(self._dark)
        try:
            interval = int(self._entries["poll_interval"].get())
            if interval < 1:
                interval = 3
        except Exception:
            interval = 3

        self._save({
            "server":       self._entries["server"].get(),
            "username":     self._entries["username"].get(),
            "password":     self._entries["password"].get(),
            "topic":        self._entries["topic"].get(),
            "poll_interval": interval,
            "auto_start":   self._var_autostart.get(),
        })
        self._close()

    def _cancel_or_close(self):
        if self._cancel:
            self._cancel()
        self._close()

    def _close(self):
        _theme.unwatch()
        self._closed = True
        if self._win:
            try:
                self._win.destroy()
            except Exception:
                pass


# ── ttk widget 递归重着色 ─────────────────────────────────────────────────

def _recolor_ttk(widget: tk.Widget, is_dark: bool):
    """递归地将 ttk 控件应用到当前主题色。"""
    c = _theme.colors(is_dark)
    try:
        opts = dict(widget.configure())
    except tk.TclError:
        return

    cls = widget.winfo_class()

    if cls == "TEntry":
        try:
            widget.configure(
                fieldbackground=c["input_bg"],
                foreground=c["input_fg"],
                insertcolor=c["accent"],
            )
        except tk.TclError:
            pass
    elif cls == "TCheckbutton":
        try:
            widget.configure(foreground=c["text"])
        except tk.TclError:
            pass

    try:
        for ch in widget.winfo_children():
            _recolor_ttk(ch, is_dark)
    except Exception:
        pass