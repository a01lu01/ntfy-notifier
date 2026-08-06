"""表格列状态记忆 - ntfy-Notifier

保存推送表格的列顺序与列宽到 ui_state.json，损坏时回退默认值。
"""

import json
import os
from pathlib import Path

DEFAULT_COLUMN_ORDER = ["time", "title", "message"]
DEFAULT_COLUMN_WIDTHS = {"time": 180, "title": 220, "message": 640}
MIN_COLUMN_WIDTHS = {"time": 120, "title": 80, "message": 160}


def _state_dir() -> Path:
    if os.environ.get("APPDATA"):
        return Path(os.environ["APPDATA"]) / "ntfy-notifier"
    return Path.home() / "AppData" / "Roaming" / "ntfy-notifier"


def _default_state() -> dict:
    return {
        "column_order": list(DEFAULT_COLUMN_ORDER),
        "column_widths": dict(DEFAULT_COLUMN_WIDTHS),
    }


class ColumnStateStore:
    """读写推送表格的列顺序与列宽。"""

    def __init__(self, path=None):
        self.path = Path(path) if path else _state_dir() / "ui_state.json"

    def load(self) -> dict:
        """加载列状态；文件缺失/损坏/字段非法时回退默认值。"""
        default = _default_state()
        try:
            with open(self.path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except Exception:
            return default

        order = data.get("column_order")
        if not isinstance(order, list) or not all(isinstance(c, str) for c in order):
            order = list(DEFAULT_COLUMN_ORDER)
        else:
            # 去重并补全缺失列
            seen = []
            for col in order:
                if col not in seen:
                    seen.append(col)
            for col in DEFAULT_COLUMN_ORDER:
                if col not in seen:
                    seen.append(col)
            order = seen

        raw_widths = data.get("column_widths")
        widths = {}
        if isinstance(raw_widths, dict):
            for col in DEFAULT_COLUMN_ORDER:
                try:
                    value = int(raw_widths.get(col, DEFAULT_COLUMN_WIDTHS[col]))
                except (TypeError, ValueError):
                    value = DEFAULT_COLUMN_WIDTHS[col]
                widths[col] = max(MIN_COLUMN_WIDTHS.get(col, 80), value)
        else:
            widths = dict(DEFAULT_COLUMN_WIDTHS)

        return {"column_order": order, "column_widths": widths}

    def save(self, order, widths) -> bool:
        """原子保存列顺序与列宽（逻辑单位，含最小宽度约束）。"""
        payload = {"column_order": list(order), "column_widths": {}}
        for col in DEFAULT_COLUMN_ORDER:
            try:
                value = int(widths.get(col, DEFAULT_COLUMN_WIDTHS[col]))
            except (TypeError, ValueError):
                value = DEFAULT_COLUMN_WIDTHS[col]
            payload["column_widths"][col] = max(MIN_COLUMN_WIDTHS.get(col, 80), value)
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_suffix(".tmp")
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump(payload, f, indent=2, ensure_ascii=False)
            os.replace(tmp, self.path)
            return True
        except Exception:
            return False
