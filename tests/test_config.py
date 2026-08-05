import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from src import config


class ConfigTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        config.CONFIG_DIR = Path(self._tmp.name)
        config.CONFIG_FILE = config.CONFIG_DIR / "config.json"

    def tearDown(self):
        self._tmp.cleanup()

    def test_dpapi_roundtrip(self):
        if not config._DPAPI_AVAILABLE:
            self.skipTest("win32crypt 不可用")
        encrypted = config._encrypt_password("secret123")
        self.assertEqual(config._decrypt_password(encrypted), "secret123")

    def test_save_never_writes_plaintext_password(self):
        config.save_config({
            "server": "https://example.com",
            "username": "u",
            "password": "secret123",
            "topic": "sms",
            "auto_start": False,
            "auto_copy_otp": False,
        })
        with open(config.CONFIG_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
        self.assertNotIn("password", data)
        self.assertIn("password_encrypted", data)

    def test_plaintext_password_migrated_on_load(self):
        if not config._DPAPI_AVAILABLE:
            self.skipTest("win32crypt 不可用")
        config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        with open(config.CONFIG_FILE, "w", encoding="utf-8") as f:
            json.dump({"server": "https://example.com", "username": "u",
                       "password": "secret123", "topic": "sms"}, f)
        cfg, first, corrupt = config.load_config()
        self.assertFalse(first)
        self.assertFalse(corrupt)
        self.assertEqual(cfg["password"], "secret123")
        with open(config.CONFIG_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
        self.assertNotIn("password", data)
        self.assertIn("password_encrypted", data)

    def test_corrupt_file_backed_up_and_reset(self):
        config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        with open(config.CONFIG_FILE, "w", encoding="utf-8") as f:
            f.write("{not valid json")
        cfg, first, corrupt = config.load_config()
        self.assertTrue(corrupt)
        self.assertTrue(first)
        self.assertEqual(cfg["server"], "")
        backups = list(config.CONFIG_DIR.glob("config.json.corrupt-*"))
        self.assertEqual(len(backups), 1)
        self.assertTrue(config.CONFIG_FILE.exists())


if __name__ == "__main__":
    unittest.main()
