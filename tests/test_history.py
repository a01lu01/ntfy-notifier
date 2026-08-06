import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from src import history


class HistoryTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        history.HISTORY_DIR = Path(self._tmp.name)
        history.HISTORY_FILE = history.HISTORY_DIR / "history.db"

    def tearDown(self):
        self._tmp.cleanup()

    def test_record_and_dedup(self):
        msg = {"id": "1", "topic": "test-topic", "title": "t", "message": "m"}
        self.assertTrue(history.record_message(msg))
        self.assertFalse(history.record_message(msg))
        rows = history.get_messages()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["message"], "m")

    def test_prunes_to_max_history(self):
        for i in range(1005):
            self.assertTrue(history.record_message(
                {"id": str(i), "topic": "test-topic", "title": f"t{i}", "message": f"m{i}"}
            ))
        rows = history.get_messages()
        self.assertLessEqual(len(rows), 1000)

    def test_clear(self):
        history.record_message({"id": "1", "topic": "test-topic", "title": "t", "message": "m"})
        self.assertTrue(history.clear_history())
        self.assertEqual(history.get_messages(), [])


if __name__ == "__main__":
    unittest.main()
