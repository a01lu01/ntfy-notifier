"""Fluent 风格主窗口 - ntfy-Notifier

单主窗口 + 左侧导航（推送 / 设置 / 关于），支持深浅色主题。
"""

import sys
import tkinter as tk
import webbrowser
from tkinter import messagebox, ttk
from typing import Callable, Optional

from src.theme import DARK, LIGHT, ThemeManager
from src.ui_state import (
    DEFAULT_COLUMN_ORDER,
    DEFAULT_COLUMN_WIDTHS,
    MIN_COLUMN_WIDTHS,
    ColumnStateStore,
)

APP_VERSION = "1.0.0"
GITHUB_URL = "https://github.com/a01lu01/ntfy-notifier"

COLUMN_TITLES = {"time": "时间", "title": "标题", "message": "内容"}


def _font_family(root: tk.Misc) -> str:
    """优先 Segoe UI Variable，回退 Segoe UI。"""
    try:
        import tkinter.font as tkfont
        families = set(tkfont.families(root))
        if "Segoe UI Variable" in families:
            return "Segoe UI Variable"
    except Exception:
        pass
    return "Segoe UI"


def _walk_apply(widget: tk.Widget, tokens: dict):
    """递归给经典 tk 控件应用主题色（通过 _theme_role 识别语义角色）。"""
    for child in widget.winfo_children():
        role = getattr(child, "_theme_role", None)
        cls = child.winfo_class()
        if cls == "Frame":
            bg_map = {
                "card": "card_bg",
                "toolbar": "window_bg",
                "input_border": "input_border",
                "input_inner": "input_bg",
            }
            child.configure(bg=tokens.get(bg_map.get(role, "window_bg")))
        elif cls == "Label":
            fg = tokens["text"]
            if role in ("subtitle", "subtext"):
                fg = tokens["subtext"]
            elif role in ("accent", "link"):
                fg = tokens["accent_text"]
            child.configure(bg=tokens["window_bg"], fg=fg)
        elif cls == "Button":
            if role == "accent":
                child.configure(
                    bg=tokens["accent"], fg="#FFFFFF",
                    activebackground=tokens["accent"], activeforeground="#FFFFFF",
                )
            elif role == "danger":
                child.configure(
                    bg=tokens["accent"], fg="#FFFFFF",
                    activebackground=tokens["accent"], activeforeground="#FFFFFF",
                )
            else:
                child.configure(
                    bg=tokens["card_bg"], fg=tokens["text"],
                    activebackground=tokens["hover"], activeforeground=tokens["text"],
                )
        elif cls == "Entry":
            child.configure(
                bg=tokens["input_bg"], fg=tokens["text"],
                insertbackground=tokens["text"],
            )
        elif cls in ("Checkbutton", "Radiobutton"):
            child.configure(
                bg=tokens["window_bg"], fg=tokens["text"],
                activebackground=tokens["window_bg"],
                activeforeground=tokens["text"],
                selectcolor=tokens["card_bg"],
            )
        _walk_apply(child, tokens)


def _apply_window_style(win: tk.Toplevel, tokens: dict):
    """应用 pywinstyles 的 Win11 圆角与标题栏明暗；失败则忽略。"""
    try:
        import pywinstyles
        pywinstyles.apply_style(win, "win11")
        pywinstyles.apply_style(win, "dark" if tokens is DARK else "light")
        pywinstyles.change_header_color(win, tokens["window_bg"])
    except Exception:
        pass


class MainWindow:
    """主窗口：左侧导航 + 推送/设置/关于三个页面。"""

    def __init__(
        self,
        master: tk.Tk,
        config: dict,
        on_save: Callable[[dict], None],
        theme_manager: ThemeManager,
        scale: float = 1.0,
    ):
        self._master = master
        self._config = dict(config)
        self._on_save = on_save
        self._theme = theme_manager
        self._scale = scale
        self._win: Optional[tk.Toplevel] = None
        self._sidebar: Optional[tk.Frame] = None
        self._nav_items = {}
        self._pages = {}
        self._current_page = "push"
        self._font = _font_family(master)
        self._store = ColumnStateStore()
        self._build()

    def _sc(self, value: int) -> int:
        return max(1, int(round(value * self._scale)))

    def _build(self):
        win = tk.Toplevel(self._master)
        self._win = win
        win.title("ntfy-Notifier")
        win.configure(bg=DARK["window_bg"])
        win.geometry(f"{self._sc(1350)}x{self._sc(800)}")
        win.minsize(self._sc(900), self._sc(500))
        win.protocol("WM_DELETE_WINDOW", self.hide)

        self._sidebar = tk.Frame(win, width=self._sc(180), bg=DARK["window_bg"])
        self._sidebar.pack(side="left", fill="y")
        self._sidebar.pack_propagate(False)

        content = tk.Frame(win, bg=DARK["window_bg"])
        content.pack(side="left", fill="both", expand=True)

        self._build_nav()
        self._pages["push"] = PushPage(content, self._scale, self._font, self._store)
        self._pages["settings"] = SettingsPage(
            content, self._config, self._handle_save, self._theme, self._scale, self._font
        )
        self._pages["about"] = AboutPage(content, self._scale, self._font)
        for page in self._pages.values():
            page.frame.pack(fill="both", expand=True)
            page.frame.pack_forget()

        self.show_page("push")
        self.apply_theme()
        win.withdraw()

    def _handle_save(self, cfg: dict):
        """保存配置后同步设置页的“当前值”，并立即应用主题。"""
        self._config = dict(cfg)
        self._pages["settings"].update_config(cfg)
        self._theme.set_mode(cfg.get("theme_mode", "system"))
        self.apply_theme()
        if self._on_save:
            self._on_save(cfg)

    def _build_nav(self):
        for name, text in (("push", "推送"), ("settings", "设置"), ("about", "关于")):
            item = tk.Frame(self._sidebar, bg=DARK["window_bg"])
            item.pack(fill="x", padx=self._sc(8), pady=self._sc(2))
            item._theme_role = "nav_item"
            indicator = tk.Frame(item, width=self._sc(3), bg=DARK["window_bg"])
            indicator.pack(side="left", fill="y")
            label = tk.Label(
                item,
                text=text,
                font=(self._font, 11),
                bg=DARK["window_bg"],
                fg=DARK["text"],
                anchor="w",
                cursor="hand2",
                padx=self._sc(10),
                pady=self._sc(7),
            )
            label._theme_role = "nav"
            label.pack(side="left", fill="x", expand=True)
            label.bind("<Button-1>", lambda _e, n=name: self.show_page(n))
            label.bind("<Enter>", lambda _e, l=label: self._nav_hover(l, True))
            label.bind("<Leave>", lambda _e, l=label: self._nav_hover(l, False))
            self._nav_items[name] = (item, indicator, label)

    def _nav_hover(self, label: tk.Label, entering: bool):
        tokens = DARK if self._theme.current == "dark" else LIGHT
        name = next(n for n, (_, _, l) in self._nav_items.items() if l is label)
        if name == self._current_page:
            return
        label.configure(bg=tokens["hover"] if entering else tokens["window_bg"])

    def _select_nav(self, name: str):
        tokens = DARK if self._theme.current == "dark" else LIGHT
        self._current_page = name
        for n, (item, indicator, label) in self._nav_items.items():
            if n == name:
                item.configure(bg=tokens["selected"])
                indicator.configure(bg=tokens["accent"])
                label.configure(
                    bg=tokens["selected"], fg=tokens["accent_text"], font=(self._font, 11, "bold")
                )
            else:
                item.configure(bg=tokens["window_bg"])
                indicator.configure(bg=tokens["window_bg"])
                label.configure(
                    bg=tokens["window_bg"], fg=tokens["text"], font=(self._font, 11)
                )

    def show_page(self, name: str):
        if name not in self._pages:
            return
        for n, page in self._pages.items():
            if n == name:
                page.frame.pack(fill="both", expand=True)
            else:
                page.frame.pack_forget()
        self._select_nav(name)
        if name == "push":
            self._pages["push"].refresh()

    def show(self):
        if self._win is None:
            return
        self._win.update_idletasks()
        sw, sh = self._win.winfo_screenwidth(), self._win.winfo_screenheight()
        w, h = self._sc(1350), self._sc(800)
        self._win.geometry(
            f"{w}x{h}+{(sw - w) // 2}+{(sh - h) // 2}"
        )
        self._win.deiconify()
        self._win.lift()
        self._win.focus_force()

    def hide(self):
        if self._win is not None:
            self._win.withdraw()

    def apply_theme(self):
        tokens = DARK if self._theme.current == "dark" else LIGHT
        if self._win is None:
            return
        self._win.configure(bg=tokens["window_bg"])
        self._sidebar.configure(bg=tokens["window_bg"])
        _walk_apply(self._sidebar, tokens)
        for page in self._pages.values():
            page.apply_theme(tokens)
        self._select_nav(self._current_page)
        _apply_window_style(self._win, tokens)

    def destroy(self):
        if self._win is not None:
            self._win.destroy()
            self._win = None


class PushPage:
    """推送历史页面。"""

    def __init__(self, parent, scale, font, store: ColumnStateStore):
        self._scale = scale
        self._font = font
        self._store = store
        self._state = store.load()
        self._order = list(self._state["column_order"])
        self._drag_col = None

        self.frame = tk.Frame(parent, bg=DARK["window_bg"])
        self.frame._theme_role = "window"

        toolbar = tk.Frame(self.frame, bg=DARK["window_bg"])
        toolbar._theme_role = "toolbar"
        toolbar.pack(fill="x", padx=self._sc(16), pady=self._sc(12))
        self._btn_refresh = self._make_button(toolbar, "刷新", self.refresh)
        self._btn_refresh.pack(side="left")
        self._btn_clear = self._make_button(toolbar, "清空", self._clear)
        self._btn_clear.pack(side="left", padx=(self._sc(8), 0))

        table_frame = tk.Frame(self.frame, bg=DARK["window_bg"])
        table_frame.pack(fill="both", expand=True, padx=self._sc(16), pady=(0, self._sc(16)))

        style = ttk.Style(parent)
        try:
            style.theme_use("clam")
        except Exception:
            pass
        self._tree = ttk.Treeview(
            table_frame, columns=self._order, show="headings", selectmode="browse",
            style="Push.Treeview",
        )
        vscroll = ttk.Scrollbar(table_frame, orient="vertical", command=self._tree.yview)
        hscroll = ttk.Scrollbar(table_frame, orient="horizontal", command=self._tree.xview)
        self._tree.configure(yscrollcommand=vscroll.set, xscrollcommand=hscroll.set)
        self._tree.grid(row=0, column=0, sticky="nsew")
        vscroll.grid(row=0, column=1, sticky="ns")
        hscroll.grid(row=1, column=0, sticky="ew")
        table_frame.rowconfigure(0, weight=1)
        table_frame.columnconfigure(0, weight=1)

        self._empty = tk.Label(
            self.frame, text="暂无推送", font=(self._font, 12),
            bg=DARK["window_bg"], fg=DARK["subtext"],
        )
        self._empty._theme_role = "subtext"

        self._tree.bind("<Double-1>", self._copy_message)
        self._tree.bind("<ButtonPress-1>", self._on_heading_press)
        self._tree.bind("<B1-Motion>", self._on_heading_motion)
        self._tree.bind("<ButtonRelease-1>", self._on_heading_release)
        self._set_columns()
        self.refresh()

    def _sc(self, value: int) -> int:
        return max(1, int(round(value * self._scale)))

    def _make_button(self, parent, text, command) -> tk.Button:
        btn = tk.Button(
            parent, text=text, font=(self._font, 10),
            bd=0, relief="flat", cursor="hand2", command=command,
        )
        btn._theme_role = "secondary"
        return btn

    def _set_columns(self):
        self._tree["columns"] = self._order
        for col in self._order:
            self._tree.heading(col, text=COLUMN_TITLES[col])
            self._tree.column(
                col,
                width=self._sc(self._state["column_widths"].get(
                    col, DEFAULT_COLUMN_WIDTHS[col]
                )),
                minwidth=self._sc(MIN_COLUMN_WIDTHS[col]),
                stretch=(col == "message"),
                anchor="w",
            )

    def _on_heading_press(self, event):
        col = self._tree.identify_column(event.x)
        if col and col != "#0":
            self._drag_col = col

    def _on_heading_motion(self, event):
        if self._drag_col is None:
            return
        cur = self._tree.identify_column(event.x)
        if not cur or cur == "#0" or cur == self._drag_col:
            return
        a = self._order.index(self._drag_col)
        b = self._order.index(cur)
        self._order[a], self._order[b] = self._order[b], self._order[a]
        self._set_columns()
        self._drag_col = cur

    def _on_heading_release(self, event):
        if self._drag_col is None:
            # 也可能是列宽调整：保存宽度即可
            self._save_column_state()
            return
        self._drag_col = None
        self._save_column_state()

    def _save_column_state(self):
        widths = {}
        for col in self._order:
            logical = int(self._tree.column(col, "width") / self._scale)
            widths[col] = max(MIN_COLUMN_WIDTHS[col], logical)
        self._state = {"column_order": list(self._order), "column_widths": widths}
        self._store.save(self._order, widths)

    def refresh(self):
        from src.history import get_messages
        items = get_messages(limit=1000)
        for item in self._tree.get_children():
            self._tree.delete(item)
        for row in items:
            self._tree.insert(
                "", "end", values=(row["time"], row["title"], row["message"])
            )
        if items:
            self._empty.pack_forget()
        else:
            self._empty.pack(fill="x", pady=self._sc(24))

    def _clear(self):
        if not messagebox.askyesno(
            "清空历史", "确定清空全部推送历史？此操作不可恢复。",
            parent=self.frame.winfo_toplevel(),
        ):
            return
        from src.history import clear_history
        if clear_history():
            self.refresh()

    def _copy_message(self, event):
        selection = self._tree.selection()
        if not selection:
            return
        values = self._tree.item(selection[0], "values")
        if len(values) >= 3:
            win = self.frame.winfo_toplevel()
            win.clipboard_clear()
            win.clipboard_append(values[2])

    def apply_theme(self, tokens):
        self.frame.configure(bg=tokens["window_bg"])
        _walk_apply(self.frame, tokens)
        self._empty.configure(bg=tokens["window_bg"], fg=tokens["subtext"])
        style = ttk.Style(self.frame)
        style.configure(
            "Push.Treeview",
            background=tokens["card_bg"],
            fieldbackground=tokens["card_bg"],
            foreground=tokens["text"],
            borderwidth=0,
            rowheight=self._sc(30),
            font=(self._font, 10),
        )
        style.map(
            "Push.Treeview",
            background=[("selected", tokens["selected"])],
            foreground=[("selected", tokens["text"])],
        )
        style.configure(
            "Push.Treeview.Heading",
            background=tokens["hover"],
            foreground=tokens["text"],
            relief="flat",
            font=(self._font, 10, "bold"),
            padding=(self._sc(6), self._sc(4)),
        )
        style.map(
            "Push.Treeview.Heading",
            background=[("active", tokens["selected"])],
        )


class SettingsPage:
    """设置页面。"""

    def __init__(
        self,
        parent,
        config: dict,
        on_save: Callable[[dict], None],
        theme_manager: ThemeManager,
        scale: float,
        font: str,
    ):
        self._scale = scale
        self._font = font
        self._on_save = on_save
        self._theme = theme_manager
        self._current = dict(config)
        self._entries = {}
        self._pwd_visible = False

        self.frame = tk.Frame(parent, bg=DARK["window_bg"])
        self.frame._theme_role = "window"

        content = tk.Frame(self.frame, bg=DARK["window_bg"])
        content.pack(fill="both", expand=True, padx=self._sc(40), pady=self._sc(24))

        tk.Label(
            content, text="设置", font=(self._font, 15, "bold"),
            bg=DARK["window_bg"], fg=DARK["text"], anchor="w",
        ).pack(anchor="w")
        tk.Label(
            content, text="配置 ntfy-Notifier 连接参数", font=(self._font, 10),
            bg=DARK["window_bg"], fg=DARK["subtext"], anchor="w",
        ).pack(anchor="w", pady=(self._sc(2), self._sc(16)))

        self._build_input(content, "服务器地址", "server", placeholder="https://...")

        pair = tk.Frame(content, bg=DARK["window_bg"])
        pair.pack(fill="x")
        left = tk.Frame(pair, bg=DARK["window_bg"])
        left.pack(side="left", fill="x", expand=True, padx=(0, self._sc(6)))
        right = tk.Frame(pair, bg=DARK["window_bg"])
        right.pack(side="left", fill="x", expand=True, padx=(self._sc(6), 0))
        self._build_input(left, "用户名", "username")
        self._build_input(right, "密码", "password", is_password=True)

        self._build_input(content, "主题", "topic", placeholder="sms")

        # 界面主题
        theme_label = tk.Label(
            content, text="界面主题", font=(self._font, 10),
            bg=DARK["window_bg"], fg=DARK["subtext"], anchor="w",
        )
        theme_label._theme_role = "subtext"
        theme_label.pack(anchor="w", pady=(self._sc(12), self._sc(4)))
        self._var_theme = tk.StringVar(value=self._current.get("theme_mode", "system"))
        theme_row = tk.Frame(content, bg=DARK["window_bg"])
        theme_row.pack(anchor="w")
        for value, text in (("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")):
            tk.Radiobutton(
                theme_row, text=text, value=value, variable=self._var_theme,
                font=(self._font, 10), cursor="hand2",
                bg=DARK["window_bg"], fg=DARK["text"],
                activebackground=DARK["window_bg"], activeforeground=DARK["text"],
                selectcolor=DARK["card_bg"],
            ).pack(side="left", padx=(0, self._sc(16)))

        # 行为选项
        self._var_auto_start = tk.BooleanVar(value=self._current.get("auto_start", False))
        cb = tk.Checkbutton(
            content, text="开机自启动", font=(self._font, 10),
            variable=self._var_auto_start, cursor="hand2",
            bg=DARK["window_bg"], fg=DARK["text"],
            activebackground=DARK["window_bg"], activeforeground=DARK["text"],
            selectcolor=DARK["card_bg"],
        )
        cb.pack(anchor="w", pady=(self._sc(12), 0))
        self._var_auto_copy = tk.BooleanVar(value=self._current.get("auto_copy_otp", False))
        cb2 = tk.Checkbutton(
            content, text="收到短信时自动复制验证码到剪贴板", font=(self._font, 10),
            variable=self._var_auto_copy, cursor="hand2",
            bg=DARK["window_bg"], fg=DARK["text"],
            activebackground=DARK["window_bg"], activeforeground=DARK["text"],
            selectcolor=DARK["card_bg"],
        )
        cb2.pack(anchor="w", pady=(self._sc(6), 0))

        # 底部按钮
        footer = tk.Frame(self.frame, bg=DARK["window_bg"])
        footer.pack(side="bottom", fill="x", padx=self._sc(40), pady=self._sc(16))
        save_btn = tk.Button(
            footer, text="保存", font=(self._font, 10, "bold"),
            bd=0, relief="flat", cursor="hand2", command=self._save,
        )
        save_btn._theme_role = "accent"
        save_btn.pack(side="right", ipadx=self._sc(12), ipady=self._sc(5))
        cancel_btn = tk.Button(
            footer, text="取消", font=(self._font, 10),
            bd=0, relief="flat", cursor="hand2", command=self._reload,
        )
        cancel_btn._theme_role = "secondary"
        cancel_btn.pack(side="right", padx=(0, self._sc(10)), ipadx=self._sc(12), ipady=self._sc(5))

    def _sc(self, value: int) -> int:
        return max(1, int(round(value * self._scale)))

    def _build_input(
        self, parent, label_text, key, *, is_password=False, placeholder=""
    ):
        block = tk.Frame(parent, bg=DARK["window_bg"])
        block.pack(fill="x", pady=(0, self._sc(10)))
        tk.Label(
            block, text=label_text, font=(self._font, 10),
            bg=DARK["window_bg"], fg=DARK["subtext"], anchor="w",
        ).pack(anchor="w")

        border = tk.Frame(block, bg=DARK["input_border"], padx=1, pady=1)
        border._theme_role = "input_border"
        border.pack(fill="x", pady=(self._sc(4), 0))
        inner = tk.Frame(border, bg=DARK["input_bg"])
        inner._theme_role = "input_inner"
        inner.pack(fill="x")

        entry = tk.Entry(
            inner, font=(self._font, 11), bd=0, relief="flat",
            highlightthickness=0, bg=DARK["input_bg"], fg=DARK["text"],
            insertbackground=DARK["text"],
        )
        if is_password:
            entry.configure(show="•")
        entry.pack(side="left", fill="x", expand=True, ipady=self._sc(6), padx=(self._sc(8), 0))
        entry.insert(0, self._current.get(key, ""))
        self._entries[key] = entry

        if is_password:
            self._pwd_btn = tk.Button(
                inner, text="显示", font=(self._font, 9), bd=0, relief="flat",
                cursor="hand2", command=self._toggle_password,
            )
            self._pwd_btn._theme_role = "secondary"
            self._pwd_btn.pack(side="right", padx=(self._sc(4), self._sc(8)))

    def _toggle_password(self):
        entry = self._entries["password"]
        if self._pwd_visible:
            entry.configure(show="•")
            self._pwd_btn.configure(text="显示")
            self._pwd_visible = False
        else:
            entry.configure(show="")
            self._pwd_btn.configure(text="隐藏")
            self._pwd_visible = True

    def _collect(self) -> dict:
        cfg = {}
        for key, entry in self._entries.items():
            cfg[key] = entry.get()
        cfg["theme_mode"] = self._var_theme.get()
        cfg["auto_start"] = bool(self._var_auto_start.get())
        cfg["auto_copy_otp"] = bool(self._var_auto_copy.get())
        return cfg

    def _save(self):
        cfg = self._collect()
        server = str(cfg.get("server", ""))
        password = str(cfg.get("password", ""))
        if server.startswith("http://") and password:
            if not messagebox.askyesno(
                "安全提示",
                "当前服务器地址使用 http://，密码将以明文在网络上传输，"
                "建议改用 https://。仍要保存吗？",
                parent=self.frame.winfo_toplevel(),
            ):
                return
        try:
            self._on_save(cfg)
        except Exception as e:
            print(f"[ntfy] 保存失败: {e}", file=sys.stderr)

    def _reload(self):
        """取消：把表单恢复为最近一次已保存的配置。"""
        for key, entry in self._entries.items():
            entry.delete(0, "end")
            entry.insert(0, self._current.get(key, ""))
        self._var_theme.set(self._current.get("theme_mode", "system"))
        self._var_auto_start.set(self._current.get("auto_start", False))
        self._var_auto_copy.set(self._current.get("auto_copy_otp", False))
        if self._pwd_visible:
            self._toggle_password()

    def update_config(self, config: dict):
        self._current = dict(config)

    def apply_theme(self, tokens):
        self.frame.configure(bg=tokens["window_bg"])
        _walk_apply(self.frame, tokens)


class AboutPage:
    """关于页面。"""

    def __init__(self, parent, scale, font):
        self._scale = scale
        self._font = font
        self.frame = tk.Frame(parent, bg=DARK["window_bg"])
        self.frame._theme_role = "window"

        box = tk.Frame(self.frame, bg=DARK["window_bg"])
        box.pack(expand=True, padx=self._sc(60), pady=self._sc(40))
        tk.Label(
            box, text="ntfy-Notifier", font=(self._font, 15, "bold"),
            bg=DARK["window_bg"], fg=DARK["text"],
        ).pack(anchor="w")
        tk.Label(
            box, text=f"版本 {APP_VERSION}", font=(self._font, 10),
            bg=DARK["window_bg"], fg=DARK["subtext"],
        ).pack(anchor="w", pady=(self._sc(4), self._sc(12)))
        tk.Label(
            box, text="Windows 系统托盘工具，订阅 ntfy 消息并弹出系统通知。",
            font=(self._font, 10), bg=DARK["window_bg"], fg=DARK["text"], anchor="w",
        ).pack(anchor="w", pady=(0, self._sc(12)))

        link = tk.Label(
            box, text=GITHUB_URL, font=(self._font, 10),
            bg=DARK["window_bg"], fg=DARK["accent_text"], cursor="hand2",
        )
        link._theme_role = "link"
        link.pack(anchor="w")
        link.bind("<Button-1>", lambda _e: webbrowser.open(GITHUB_URL))

    def _sc(self, value: int) -> int:
        return max(1, int(round(value * self._scale)))

    def apply_theme(self, tokens):
        self.frame.configure(bg=tokens["window_bg"])
        _walk_apply(self.frame, tokens)
