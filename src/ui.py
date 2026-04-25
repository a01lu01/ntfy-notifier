"""
设置窗口 UI - ntfy-Notifier
基于 Tkinter 实现 Windows Fluent 风格设置窗口
参考 Fluent Design：圆角、轻量阴影、Segoe UI Font、#0078D4 主色
"""

import sys
import tkinter as tk
from tkinter import ttk
from typing import Callable, Optional


# ── Fluent Design 色彩常量 ────────────────────────────────────────────────────
_FLUENT_BG            = "#FFFFFF"
_FLUENT_SURFACE       = "#F3F3F3"
_FLUENT_BORDER        = "#E0E0E0"
_FLUENT_TEXT          = "#1A1A1A"
_FLUENT_SUBTEXT       = "#606060"
_FLUENT_PLACEHOLDER   = "#999999"
_FLUENT_ACCENT        = "#0078D4"
_FLUENT_ACCENT_HOVER  = "#106EBE"
_FLUENT_DANGER        = "#C42B1C"
_FLUENT_INPUT_BG      = "#FFFFFF"
_FLUENT_INPUT_BORDER  = "#CCCCCC"
_FLUENT_INPUT_FOCUS   = "#0078D4"

# 窗口尺寸
_WIN_W = 460
_WIN_H = 440


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
        on_cancel: Optional[Callable] = None,
        master: Optional[tk.Tk] = None,
    ):
        self._current = dict(current_config)
        self._on_save = on_save
        self._on_cancel = on_cancel or (lambda: None)
        self._master = master
        self._win: Optional[tk.Toplevel] = None
        self._entries: dict[str, tk.Entry] = {}
        self._var_auto_start = tk.BooleanVar(value=False)
        self._closed = False
        self._pwd_visible = False

    # ── 公开接口 ─────────────────────────────────────────────────────────────

    def show(self):
        """非阻塞：在 _master 上显示设置窗口。"""
        root = self._master or tk.Tk()
        if not self._master:
            root.withdraw()

        win = tk.Toplevel(root)
        self._win = win
        win.title("ntfy-Notifier 设置")
        win.geometry(f"{_WIN_W}x{_WIN_H}")
        win.resizable(False, False)
        win.configure(bg=_FLUENT_BG)

        # ★ 关键：先 pack 底部 footer，再 pack 上方内容，确保按钮永远可见
        self._build_footer(win)
        self._build_content(win)

        win.protocol("WM_DELETE_WINDOW", self._cancel)

        # 居中
        win.update_idletasks()
        sw, sh = win.winfo_screenwidth(), win.winfo_screenheight()
        win.geometry(f"{_WIN_W}x{_WIN_H}+{(sw - _WIN_W) // 2}+{(sh - _WIN_H) // 2}")
        win.deiconify()
        win.after(0, lambda: (win.lift(), win.focus_force()))

        if not self._master:
            while not self._closed and win.winfo_exists():
                win.update()
                win.update_idletasks()

    def show_and_wait(self):
        """兼容性别名，内部调用 show()。"""
        self.show()

    # ── UI 构建 ─────────────────────────────────────────────────────────────

    def _build_content(self, parent: tk.Widget):
        """构建标题 + 表单 + 开机自启区域。"""
        content = tk.Frame(parent, bg=_FLUENT_BG)
        content.pack(fill="both", expand=True, padx=32, pady=(20, 0))

        # ── 标题 ──
        tk.Label(
            content, text="设置", font=("Segoe UI", 16, "bold"),
            fg=_FLUENT_TEXT, bg=_FLUENT_BG, anchor="w",
        ).pack(anchor="w")

        tk.Label(
            content, text="配置 ntfy-Notifier 连接参数", font=("Segoe UI", 9),
            fg=_FLUENT_SUBTEXT, bg=_FLUENT_BG, anchor="w",
        ).pack(anchor="w", pady=(2, 16))

        # ── 表单字段（上下排列：标签在上，输入框在下）──
        self._build_input_block(content, "服务器地址", "server", placeholder="http://...")

        # 密码字段行：用户名 + 密码并排
        pair = tk.Frame(content, bg=_FLUENT_BG)
        pair.pack(fill="x", pady=(0, 0))
        pair.columnconfigure(0, weight=1)
        pair.columnconfigure(1, weight=1)

        left = tk.Frame(pair, bg=_FLUENT_BG)
        left.grid(row=0, column=0, sticky="nsew", padx=(0, 6))
        self._build_input_block(left, "用户名", "username")

        right = tk.Frame(pair, bg=_FLUENT_BG)
        right.grid(row=0, column=1, sticky="nsew", padx=(6, 0))
        self._build_input_block(right, "密码", "password", is_password=True)

        self._build_input_block(content, "主题", "topic", placeholder="sms")

        # ── 开机自启 ──
        self._var_auto_start.set(self._current.get("auto_start", False))
        cb = tk.Checkbutton(
            content,
            text="  开机自启动",
            font=("Segoe UI", 9),
            variable=self._var_auto_start,
            bg=_FLUENT_BG, fg=_FLUENT_TEXT,
            activebackground=_FLUENT_BG, activeforeground=_FLUENT_TEXT,
            selectcolor=_FLUENT_BG,
            cursor="hand2",
        )
        cb.pack(anchor="w", pady=(8, 0))

    def _build_input_block(
        self, parent: tk.Widget, label: str, key: str,
        *, is_password: bool = False, placeholder: str = "",
    ):
        """构建标签 + 输入框的纵向块（密码框带独立的眼睛按钮）。"""
        block = tk.Frame(parent, bg=_FLUENT_BG)
        block.pack(fill="x", pady=(0, 10))

        # 标签
        tk.Label(
            block, text=label, font=("Segoe UI", 9),
            fg=_FLUENT_SUBTEXT, bg=_FLUENT_BG, anchor="w",
        ).pack(anchor="w")

        # 输入行
        input_row = tk.Frame(block, bg=_FLUENT_INPUT_BORDER, bd=0)
        input_row.pack(fill="x", pady=(4, 0))

        # 带边框效果的外框
        border_canvas = tk.Frame(
            input_row, bg=_FLUENT_INPUT_BORDER, padx=1, pady=1,
        )
        border_canvas.pack(fill="x")

        inner = tk.Frame(border_canvas, bg=_FLUENT_INPUT_BG)
        inner.pack(fill="x")

        # Entry
        entry_kw = dict(
            font=("Segoe UI", 10),
            bg=_FLUENT_INPUT_BG, fg=_FLUENT_TEXT,
            insertbackground=_FLUENT_TEXT,
            bd=0, relief="flat",
            highlightthickness=0,
        )
        if is_password:
            entry_kw["show"] = "•"

        entry = tk.Entry(inner, **entry_kw)
        entry.pack(side="left", fill="x", expand=True, ipady=5, padx=(8, 0))
        entry.insert(0, self._current.get(key, ""))
        self._entries[key] = entry

        # 密码切换按钮：独立在 Entry 右侧，不挤占输入框
        if is_password:
            eye_btn = tk.Label(
                inner, text="👁", font=("Segoe UI Emoji", 10),
                bg=_FLUENT_INPUT_BG, fg=_FLUENT_SUBTEXT,
                cursor="hand2",
            )
            eye_btn.pack(side="right", padx=(4, 6), pady=4)
            # 绑定点击
            eye_btn.bind("<Button-1>", lambda _e: self._toggle_password(entry, eye_btn))
            # hover 效果
            eye_btn.bind("<Enter>", lambda _e: eye_btn.config(fg=_FLUENT_ACCENT))
            eye_btn.bind("<Leave>", lambda _e: eye_btn.config(fg=_FLUENT_SUBTEXT))

        # 焦点高亮
        def _on_focus_in(_e, e=entry, bc=border_canvas):
            bc.config(bg=_FLUENT_INPUT_FOCUS)

        def _on_focus_out(_e, e=entry, bc=border_canvas):
            bc.config(bg=_FLUENT_INPUT_BORDER)

        entry.bind("<FocusIn>", _on_focus_in)
        entry.bind("<FocusOut>", _on_focus_out)

    def _toggle_password(self, entry: tk.Entry, eye_btn: tk.Label):
        """切换密码可见性。"""
        if self._pwd_visible:
            entry.config(show="•")
            eye_btn.config(text="👁")
            self._pwd_visible = False
        else:
            entry.config(show="")
            eye_btn.config(text="👁‍🗨")
            self._pwd_visible = True

    def _build_footer(self, parent: tk.Widget):
        """构建底部按钮栏（pack(side=BOTTOM) 确保永远可见）。"""
        footer = tk.Frame(parent, bg=_FLUENT_BG)
        footer.pack(side="bottom", fill="x")

        # 分隔线
        tk.Frame(footer, height=1, bg=_FLUENT_BORDER).pack(fill="x")

        btn_row = tk.Frame(footer, bg=_FLUENT_BG)
        btn_row.pack(fill="x", padx=32, pady=(12, 20))

        # 保存按钮
        save_btn = tk.Button(
            btn_row, text="  保存  ", font=("Segoe UI", 9, "bold"),
            fg="#FFFFFF", bg=_FLUENT_ACCENT,
            activeforeground="#FFFFFF", activebackground=_FLUENT_ACCENT_HOVER,
            bd=0, cursor="hand2", relief="flat",
            command=self._save,
        )
        save_btn.pack(side="right", ipady=4)

        # 取消按钮
        cancel_btn = tk.Button(
            btn_row, text="  取消  ", font=("Segoe UI", 9),
            fg=_FLUENT_TEXT, bg=_FLUENT_SURFACE,
            activeforeground=_FLUENT_TEXT, activebackground=_FLUENT_BORDER,
            bd=0, cursor="hand2", relief="flat",
            command=self._cancel,
        )
        cancel_btn.pack(side="right", padx=(0, 10), ipady=4)

    # ── 交互逻辑 ────────────────────────────────────────────────────────────

    def _collect_config(self) -> dict:
        """从表单收集当前配置值。"""
        cfg = {}
        for key, entry in self._entries.items():
            cfg[key] = entry.get()
        cfg["auto_start"] = bool(self._var_auto_start.get())
        return cfg

    def _save(self):
        """保存配置并关闭窗口。"""
        cfg = self._collect_config()
        try:
            self._on_save(cfg)
        except Exception as e:
            print(f"[ntfy] 保存失败: {e}", file=sys.stderr)
        finally:
            self._close()

    def _cancel(self):
        """取消并关闭窗口。"""
        if self._on_cancel:
            try:
                self._on_cancel()
            except Exception:
                pass
        self._close()

    def _close(self):
        """关闭设置窗口。"""
        self._closed = True
        if self._win and self._win.winfo_exists():
            self._win.destroy()


if __name__ == "__main__":
    # 独立测试入口
    test_cfg = {
        "server": "http://114.55.43.156:8080",
        "username": "iPhone",
        "password": "",
        "topic": "sms",
        "auto_start": False,
    }

    def on_save(cfg):
        print("保存的配置:", cfg)

    root = tk.Tk()
    root.withdraw()
    win = SettingsWindow(test_cfg, on_save=on_save)
    win.show_and_wait()
    print("窗口已关闭")
