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
        win.geometry("480x520")
        win.resizable(False, False)
        win.configure(bg=_FLUENT_BG)

        # ── 标题区 ────────────────────────────────────────────────────────
        self._build_header(win)

        # ── 表单区 ────────────────────────────────────────────────────────
        form_frame = tk.Frame(win, bg=_FLUENT_BG)
        form_frame.pack(fill="x", padx=32, pady=(0, 16))

        self._build_field(form_frame, "服务器地址 (Server)", "server", row=0)
        self._build_field(form_frame, "用户名 (Username)", "username", row=1)
        self._build_password_field(form_frame, "密码 (Password)", "password", row=2)
        self._build_field(form_frame, "主题 (Topic)", "topic", row=3)

        # ── 开机自启 ──────────────────────────────────────────────────────
        startup_frame = tk.Frame(win, bg=_FLUENT_BG)
        startup_frame.pack(fill="x", padx=32, pady=(8, 0))

        self._var_auto_start.set(self._current.get("auto_start", False))
        cb = ttk.Checkbutton(
            startup_frame,
            text="开机自启动 (Auto Start)",
            variable=self._var_auto_start,
            command=self._on_startup_toggle,
        )
        cb.pack(anchor="w")

        # ── 底部按钮 ──────────────────────────────────────────────────────
        self._build_footer(win)

        win.protocol("WM_DELETE_WINDOW", self._cancel)

        # 居中并显示
        win.update_idletasks()
        sw, sh = win.winfo_screenwidth(), win.winfo_screenheight()
        win.geometry(f"480x520+{(sw - 480) // 2}+{(sh - 520) // 2}")
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

        title_label = tk.Label(
            frame,
            text="⚙️  设置",
            font=("Segoe UI", 18, "bold"),
            fg=_FLUENT_TEXT,
            bg=_FLUENT_BG,
            anchor="w",
        )
        title_label.pack(anchor="w")

        subtitle = tk.Label(
            frame,
            text="配置 ntfy-Notifier 连接参数",
            font=("Segoe UI", 10),
            fg=_FLUENT_SUBTEXT,
            bg=_FLUENT_BG,
            anchor="w",
        )
        subtitle.pack(anchor="w", pady=(4, 0))

    def _build_field(self, parent: tk.Widget, label_text: str, config_key: str, row: int):
        """构建单行输入字段（标签 + Entry）。"""
        row_frame = tk.Frame(parent, bg=_FLUENT_BG)
        row_frame.pack(fill="x", pady=(8 if row > 0 else 0, 4))

        lbl = tk.Label(
            row_frame, text=label_text, font=("Segoe UI", 9),
            fg=_FLUENT_TEXT, bg=_FLUENT_BG, anchor="w", width=16,
        )
        lbl.pack(side="left")

        entry = tk.Entry(
            row_frame, font=("Segoe UI", 9),
            bd=1, relief="solid", bg="#FAFAFA", fg=_FLUENT_TEXT,
            highlightthickness=0,
        )
        entry.pack(side="right", fill="x", expand=True, padx=(8, 0))
        entry.insert(0, self._current.get(config_key, ""))
        self._entries[config_key] = entry

    def _build_password_field(self, parent: tk.Widget, label_text: str, config_key: str, row: int):
        """构建密码输入字段（带显示/隐藏切换按钮）。"""
        row_frame = tk.Frame(parent, bg=_FLUENT_BG)
        row_frame.pack(fill="x", pady=(8 if row > 0 else 0, 4))

        lbl = tk.Label(
            row_frame, text=label_text, font=("Segoe UI", 9),
            fg=_FLUENT_TEXT, bg=_FLUENT_BG, anchor="w", width=16,
        )
        lbl.pack(side="left")

        # Entry + 眼睛按钮容器
        inner = tk.Frame(row_frame, bg=_FLUENT_BG)
        inner.pack(side="right", fill="x", expand=True, padx=(8, 0))

        entry = tk.Entry(
            inner, font=("Segoe UI", 9), show="•",
            bd=1, relief="solid", bg="#FAFAFA", fg=_FLUENT_TEXT,
            highlightthickness=0, width=20,
        )
        entry.pack(side="left", fill="x", expand=True)
        self._entries[config_key] = entry

        # 眼睛切换按钮
        self._pwd_visible = False
        eye_btn = tk.Button(
            inner, text="👁️", font=("Segoe UI", 8),
            bg=_FLUENT_BG, fg=_FLUENT_SUBTEXT, bd=0, cursor="hand2",
            width=2, padx=4,
            command=lambda: self._toggle_password_visibility(entry),
        )
        eye_btn.pack(side="right")

    def _toggle_password_visibility(self, entry: tk.Entry):
        """切换密码可见性。"""
        if self._pwd_visible:
            entry.config(show="•")
            self._pwd_visible = False
        else:
            entry.config(show="")
            self._pwd_visible = True

    def _build_footer(self, parent: tk.Widget):
        """构建底部按钮栏。"""
        footer_frame = tk.Frame(parent, bg=_FLUENT_BG)
        footer_frame.pack(fill="x", padx=32, pady=(16, 24))

        # 分隔线
        sep = tk.Frame(footer_frame, height=1, bg=_FLUENT_BORDER)
        sep.pack(fill="x", pady=(0, 12))

        btn_frame = tk.Frame(footer_frame, bg=_FLUENT_BG)
        btn_frame.pack(fill="x")

        # 取消按钮
        cancel_btn = tk.Button(
            btn_frame, text="取消", font=("Segoe UI", 9),
            fg=_FLUENT_TEXT, bg="transparent", bd=0, cursor="hand2",
            width=10, command=self._cancel,
        )
        cancel_btn.pack(side="right", padx=(8, 0))

        # 保存按钮（Accent 色）
        save_btn = tk.Button(
            btn_frame, text="保存", font=("Segoe UI", 9, "bold"),
            fg="#FFFFFF", bg=_FLUENT_ACCENT, bd=0, cursor="hand2",
            width=10, command=self._save,
        )
        save_btn.pack(side="right")

    # ── 交互逻辑 ────────────────────────────────────────────────────────────

    def _on_startup_toggle(self):
        """开机自启复选框切换回调。"""
        pass  # 保存时统一处理

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
            # 简单错误提示（不阻塞）
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
