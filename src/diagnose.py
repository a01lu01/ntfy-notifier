"""诊断工具 - 验证打包后的 ntfy-Notifier 各模块是否正常"""
import sys
import os
import traceback
from pathlib import Path

LOG_FILE = Path(os.environ["APPDATA"]) / "ntfy-notifier" / "diagnose.log"

def log(msg):
    print(msg, flush=True)
    try:
        LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
        with open(LOG_FILE, "a", encoding="utf-8") as f:
            f.write(msg + "\n")
    except Exception:
        pass

def check_module(name, import_fn):
    try:
        import_fn()
        log(f"[OK]  {name}")
        return True
    except Exception as e:
        log(f"[FAIL] {name}: {type(e).__name__}: {e}")
        traceback.print_exc(file=open(LOG_FILE, "a", encoding="utf-8") if False else sys.stdout)
        return False

def main():
    log("=" * 50)
    log("ntfy-Notifier 诊断报告")
    log(f"时间: {os.popen('date /t && time /t').read().strip()}")
    log(f"Python: {sys.version}")
    log(f"EXE: {sys.executable}")
    log(f"Frozen: {getattr(sys, 'frozen', False)}")
    if hasattr(sys, '_MEIPASS'):
        log(f"_MEIPASS: {sys._MEIPASS}")
    log("=" * 50)

    all_ok = True

    # 1. 核心模块导入
    log("\n--- 模块导入 ---")
    all_ok &= check_module("requests", lambda: __import__("requests"))
    all_ok &= check_module("pystray", lambda: __import__("pystray"))
    all_ok &= check_module("PIL (Pillow)", lambda: __import__("PIL"))
    all_ok &= check_module("plyer", lambda: __import__("plyer"))
    all_ok &= check_module("win32gui", lambda: __import__("win32gui"))
    all_ok &= check_module("tkinter", lambda: __import__("tkinter"))

    # 2. SSL 库
    log("\n--- SSL 库 ---")
    try:
        import ssl
        ctx = ssl.create_default_context()
        log(f"[OK]  ssl.create_default_context - 支持 TLS: {ssl.OPENSSL_VERSION}")
    except Exception as e:
        log(f"[FAIL] ssl: {e}")
        all_ok = False

    # 3. requests 实际连接测试
    log("\n--- 网络连接测试 ---")
    try:
        import requests
        # 尝试连接 ntfy 服务器
        from src.config import load_config
        cfg, _ = load_config()
        server = cfg.get("server", "http://114.55.43.156:8080")
        topic = cfg.get("topic", "sms")
        url = f"{server.rstrip('/')}/{topic}/sse"
        log(f"目标: {url}")

        resp = requests.get(
            url,
            auth=(cfg.get("username", ""), cfg.get("password", "")) or None,
            timeout=10,
            proxies={"http": None, "https": None},
            stream=True,
        )
        log(f"状态码: {resp.status_code}")
        log(f"Content-Type: {resp.headers.get('Content-Type', 'N/A')}")
        # 读取第一批数据
        data = next(resp.iter_lines(decode_unicode=True), None)
        log(f"首条 SSE 数据: {data}")
        resp.close()
    except Exception as e:
        log(f"[FAIL] 网络请求: {type(e).__name__}: {e}")
        traceback.print_exc()
        all_ok = False

    # 4. 通知后端测试
    log("\n--- 通知测试 ---")
    try:
        from src.notifier import (
            _WINRT_AVAILABLE, _PLYER_AVAILABLE,
            _WIN32GUI_AVAILABLE, _WINOTIFY_AVAILABLE,
        )
        log(f"winotify 可用: {_WINOTIFY_AVAILABLE}")
        log(f"winrt 可用: {_WINRT_AVAILABLE}")
        log(f"plyer 可用: {_PLYER_AVAILABLE}")
        log(f"win32gui 可用: {_WIN32GUI_AVAILABLE}")
    except Exception as e:
        log(f"[FAIL] notifier 导入: {type(e).__name__}: {e}")
        traceback.print_exc()

    # winotify 通知测试（主程序优先使用此通道）
    try:
        from winotify import Notification, audio as wa
        toast = Notification(
            app_id="ntfy-Notifier",
            title="诊断通知 (winotify)",
            msg="ntfy-Notifier winotify 通知测试",
            duration="short",
        )
        toast.set_audio(wa.Default, loop=False)
        toast.show()
        log("[OK] winotify 通知发送成功")
        all_ok &= True
    except Exception as e:
        log(f"[FAIL] winotify 通知: {type(e).__name__}: {e}")
        traceback.print_exc()
        all_ok = False

    if _PLYER_AVAILABLE:
        try:
            from plyer import notification
            notification.notify(title="诊断通知 (plyer)", message="ntfy-Notifier plyer 通知测试", timeout=5)
            log("[OK] plyer 通知发送成功")
        except Exception as e:
            log(f"[FAIL] plyer 通知: {e}")
            traceback.print_exc()

    if _WIN32GUI_AVAILABLE:
        try:
            import win32gui
            # win32gui.MessageBox(0, "测试", "ntfy-Notifier 诊断", 0x40)
            log(f"[OK] win32gui 可用（手动测试 MessageBox）")
        except Exception as e:
            log(f"[FAIL] win32gui: {e}")

    # 5. 配置文件
    log("\n--- 配置文件 ---")
    from src.config import CONFIG_FILE, load_config
    log(f"配置路径: {CONFIG_FILE}")
    log(f"配置文件存在: {CONFIG_FILE.exists()}")
    cfg, first = load_config()
    log(f"首次运行: {first}")
    log(f"当前配置: server={cfg.get('server')}, topic={cfg.get('topic')}, username={cfg.get('username')}")

    # 6. 图标文件
    log("\n--- 图标文件 ---")
    # tray.py 的路径逻辑
    if getattr(sys, 'frozen', False):
        base = Path(sys._MEIPASS)
    else:
        base = Path(__file__).parent.parent
    for name in ["connected.ico", "disconnected.ico"]:
        p = base / name
        log(f"{name}: {p} {'存在' if p.exists() else '不存在'} ({p.stat().st_size if p.exists() else 0} bytes)")

    log("\n" + "=" * 50)
    if all_ok:
        log("结论: 所有检查通过 ✅")
    else:
        log("结论: 存在问题，请查看上方 [FAIL] 项")
    log(f"日志文件: {LOG_FILE}")
    log("=" * 50)

    input("\n按回车退出...")

if __name__ == "__main__":
    main()
