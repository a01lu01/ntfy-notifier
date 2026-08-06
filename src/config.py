"""
配置管理模块 - ntfy-Notifier
配置存储在 %APPDATA%/ntfy-notifier/config.json
密码使用 Windows DPAPI 加密存储（password_encrypted 字段）。
"""

import base64
import json
import os
import sys
import time
from pathlib import Path

if os.environ.get("APPDATA"):
    CONFIG_DIR = Path(os.environ["APPDATA"]) / "ntfy-notifier"
else:
    # APPDATA 缺失时回退到用户主目录（通常不会发生）
    CONFIG_DIR = Path.home() / "AppData" / "Roaming" / "ntfy-notifier"
CONFIG_FILE = CONFIG_DIR / "config.json"

DEFAULT_CONFIG = {
    "server": "",
    "username": "",
    "password": "",  # 请在设置中填入你的 ntfy 访问密码
    "topic": "",
    "theme_mode": "system",
    "auto_start": False,
    "auto_copy_otp": False,
}

try:
    import win32crypt
    _DPAPI_AVAILABLE = True
except ImportError:
    _DPAPI_AVAILABLE = False


def _encrypt_password(plain: str) -> str:
    """使用 Windows DPAPI 加密密码，返回 base64 字符串。"""
    if not plain or not _DPAPI_AVAILABLE:
        return plain
    try:
        blob = win32crypt.CryptProtectData(
            plain.encode("utf-16-le"), "ntfy-Notifier", None, None, None, 0
        )
        return base64.b64encode(blob).decode("ascii")
    except Exception as e:
        print(f"[config] 密码加密失败，将按明文存储: {e}", file=sys.stderr)
        return plain


def _decrypt_password(encoded: str) -> str:
    """解密 DPAPI 密码；失败返回空字符串。"""
    if not encoded or not _DPAPI_AVAILABLE:
        return encoded
    try:
        blob = base64.b64decode(encoded)
        _, data = win32crypt.CryptUnprotectData(blob, None, None, None, 0)
        if isinstance(data, str):
            return data
        return data.decode("utf-16-le")
    except Exception as e:
        print(f"[config] 密码解密失败，请重新在设置中填写密码: {e}", file=sys.stderr)
        return ""


def _prepare_for_save(config: dict) -> dict:
    """把运行时配置转换为落盘配置：密码加密存储，不写明文。"""
    out = {k: v for k, v in config.items() if k != "password_encrypted"}
    password = str(out.get("password", ""))
    out.pop("password", None)
    if _DPAPI_AVAILABLE:
        out["password_encrypted"] = _encrypt_password(password) if password else ""
    else:
        # DPAPI 不可用时的降级：明文存储并告警
        if password:
            out["password"] = password
            print("[config] ⚠️ DPAPI 不可用，密码将以明文保存在配置文件中", file=sys.stderr)
    return out


def _load_from_disk() -> dict:
    """读取磁盘上的 JSON 配置并完成密码迁移/解密。"""
    with open(CONFIG_FILE, "r", encoding="utf-8") as f:
        cfg = json.load(f)
    encrypted = str(cfg.get("password_encrypted", "") or "")
    plain = str(cfg.get("password", "") or "")
    if encrypted:
        cfg["password"] = _decrypt_password(encrypted)
    elif plain:
        # 旧版明文配置 → 迁移为加密存储
        cfg["password"] = plain
        print("[config] 检测到旧版明文密码，正在迁移为加密存储...", file=sys.stderr)
        save_config(cfg)
    else:
        cfg["password"] = ""
    return cfg


def load_config() -> tuple[dict, bool, bool]:
    """
    加载配置，如无配置文件则返回默认配置并写入。
    返回 (config_dict, is_first_run, config_was_corrupt)
    """
    corrupt = False
    if CONFIG_FILE.exists():
        try:
            return _load_from_disk(), False, False
        except (json.JSONDecodeError, IOError, KeyError) as e:
            corrupt = True
            print(f"[config] 配置文件损坏（{type(e).__name__}），已备份并重置", file=sys.stderr)
            backup = CONFIG_FILE.with_name(
                f"config.json.corrupt-{time.strftime('%Y%m%d-%H%M%S')}"
            )
            try:
                CONFIG_FILE.replace(backup)
            except OSError:
                pass
    # 首次运行或损坏重置：写入默认配置
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    save_config(DEFAULT_CONFIG.copy())
    return DEFAULT_CONFIG.copy(), True, corrupt


def save_config(config: dict) -> None:
    """保存配置到 JSON 文件（原子写入：临时文件 + replace）。"""
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    payload = _prepare_for_save(config)
    tmp = CONFIG_FILE.with_suffix(".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=4, ensure_ascii=False)
    os.replace(tmp, CONFIG_FILE)


def get_config_path() -> Path:
    """返回配置文件路径。"""
    return CONFIG_FILE
