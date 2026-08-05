"""
推送历史模块 - ntfy-Notifier

使用 SQLite 存储收到的推送消息，保留最近 MAX_HISTORY 条。
数据库不可用时所有操作返回 None/空列表，不影响通知主流程。
"""

import os
import sqlite3
import sys
import threading
from datetime import datetime
from pathlib import Path

if os.environ.get("APPDATA"):
    HISTORY_DIR = Path(os.environ["APPDATA"]) / "ntfy-notifier"
else:
    HISTORY_DIR = Path.home() / "AppData" / "Roaming" / "ntfy-notifier"
HISTORY_FILE = HISTORY_DIR / "history.db"

MAX_HISTORY = 1000

_db_lock = threading.Lock()


def _connect() -> sqlite3.Connection:
    """打开数据库连接（WAL 模式，自动建表）。"""
    HISTORY_DIR.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(HISTORY_FILE), timeout=10)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS messages (
            id          TEXT PRIMARY KEY,
            received_at TEXT NOT NULL,
            topic       TEXT,
            title       TEXT,
            message     TEXT
        )
        """
    )
    conn.commit()
    return conn


def record_message(msg: dict):
    """记录一条推送消息。

    返回：
        True  - 新记录，已写入
        False - 该消息 id 已存在（重复）
        None  - 数据库不可用/写入失败
    """
    msg_id = str(msg.get("id", "") or "")
    if not msg_id:
        return None
    try:
        with _db_lock:
            conn = _connect()
            try:
                cur = conn.execute(
                    """
                    INSERT OR IGNORE INTO messages
                        (id, received_at, topic, title, message)
                    VALUES (?, ?, ?, ?, ?)
                    """,
                    (
                        msg_id,
                        datetime.now().isoformat(timespec="seconds"),
                        str(msg.get("topic", "") or ""),
                        str(msg.get("title", "") or ""),
                        str(msg.get("message", "") or ""),
                    ),
                )
                conn.commit()
                inserted = cur.rowcount > 0
                if inserted:
                    # 裁剪：只保留最近 MAX_HISTORY 条
                    conn.execute(
                        """
                        DELETE FROM messages
                        WHERE id NOT IN (
                            SELECT id FROM messages
                            ORDER BY received_at DESC, rowid DESC
                            LIMIT ?
                        )
                        """,
                        (MAX_HISTORY,),
                    )
                    conn.commit()
                return inserted
            finally:
                conn.close()
    except Exception as e:
        print(f"[history] 记录推送失败: {e}", file=sys.stderr)
        return None


def get_messages(limit: int = MAX_HISTORY) -> list:
    """按时间倒序返回最近的历史消息列表。"""
    try:
        with _db_lock:
            conn = _connect()
            try:
                rows = conn.execute(
                    """
                    SELECT received_at, title, message
                    FROM messages
                    ORDER BY received_at DESC, rowid DESC
                    LIMIT ?
                    """,
                    (limit,),
                ).fetchall()
                return [
                    {"time": r[0], "title": r[1] or "", "message": r[2] or ""}
                    for r in rows
                ]
            finally:
                conn.close()
    except Exception as e:
        print(f"[history] 读取历史失败: {e}", file=sys.stderr)
        return []


def clear_history() -> bool:
    """清空全部推送历史。"""
    try:
        with _db_lock:
            conn = _connect()
            try:
                conn.execute("DELETE FROM messages")
                conn.commit()
                return True
            finally:
                conn.close()
    except Exception as e:
        print(f"[history] 清空历史失败: {e}", file=sys.stderr)
        return False
