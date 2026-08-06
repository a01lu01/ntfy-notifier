import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from src.ui_state import (
    DEFAULT_COLUMN_ORDER,
    DEFAULT_COLUMN_WIDTHS,
    MIN_COLUMN_WIDTHS,
    ColumnStateStore,
)


class ColumnStateStoreTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self._path = Path(self._tmp.name) / "ui_state.json"
        self._store = ColumnStateStore(self._path)

    def tearDown(self):
        self._tmp.cleanup()

    def test_default_when_missing(self):
        state = self._store.load()
        self.assertEqual(state["column_order"], DEFAULT_COLUMN_ORDER)
        self.assertEqual(state["column_widths"], DEFAULT_COLUMN_WIDTHS)

    def test_roundtrip(self):
        self.assertTrue(self._store.save(
            ["message", "time", "title"],
            {"time": 150, "title": 200, "message": 500},
        ))
        state = self._store.load()
        self.assertEqual(state["column_order"], ["message", "time", "title"])
        self.assertEqual(state["column_widths"]["time"], 150)

    def test_corrupt_file_falls_back(self):
        self._path.write_text("{not valid json", encoding="utf-8")
        state = self._store.load()
        self.assertEqual(state["column_order"], DEFAULT_COLUMN_ORDER)
        self.assertEqual(state["column_widths"], DEFAULT_COLUMN_WIDTHS)

    def test_widths_clamped_to_minimum(self):
        self.assertTrue(self._store.save(
            DEFAULT_COLUMN_ORDER,
            {"time": 10, "title": 50, "message": 99999},
        ))
        state = self._store.load()
        self.assertEqual(state["column_widths"]["time"], MIN_COLUMN_WIDTHS["time"])
        self.assertEqual(state["column_widths"]["title"], MIN_COLUMN_WIDTHS["title"])
        self.assertEqual(state["column_widths"]["message"], 99999)


if __name__ == "__main__":
    unittest.main()
