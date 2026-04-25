"""
配置管理模块 - ntfy-Notifier
配置存储在 %APPDATA%/ntfy-notifier/config.json
"""

import json
import os
from pathlib import Path

CONFIG_DIR = Path(os.environ["APPDATA"]) / "ntfy-notifier"
CONFIG_FILE = CONFIG_DIR / "config.json"

DEFAULT_CONFIG = {
    "server": "http://114.55.43.156:8080",
    "username": "iPhone",
    "password": "",  # 请在设置中填入你的 ntfy 访问密码
    "topic": "sms",
    "poll_interval": 60,
    "auto_start": False,
}


def load_config() -> tuple[dict, bool]:
    """
    加载配置，如无配置文件则返回默认配置并写入。
    返回 (config_dict, is_first_run)
    """
    if CONFIG_FILE.exists():
        try:
            with open(CONFIG_FILE, "r", encoding="utf-8") as f:
                return json.load(f), False
        except (json.JSONDecodeError, IOError):
            pass
    # 首次运行：写入默认配置并返回
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(DEFAULT_CONFIG.copy(), f, indent=4, ensure_ascii=False)
    return DEFAULT_CONFIG.copy(), True


def save_config(config: dict) -> None:
    """保存配置到 JSON 文件。"""
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(config, f, indent=4, ensure_ascii=False)


def get_config_path() -> Path:
    """返回配置文件路径。"""
    return CONFIG_FILE
