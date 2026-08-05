import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from src import notifier


class SendToastFallbackTest(unittest.TestCase):
    def setUp(self):
        self._originals = (
            notifier._WINOTIFY_AVAILABLE,
            notifier._PLYER_AVAILABLE,
            notifier._WINRT_AVAILABLE,
            notifier._WIN32GUI_AVAILABLE,
            notifier._send_winotify_toast,
            notifier._send_plyer_toast,
        )
        notifier._WINOTIFY_AVAILABLE = False
        notifier._PLYER_AVAILABLE = False
        notifier._WINRT_AVAILABLE = False
        notifier._WIN32GUI_AVAILABLE = False

    def tearDown(self):
        (
            notifier._WINOTIFY_AVAILABLE,
            notifier._PLYER_AVAILABLE,
            notifier._WINRT_AVAILABLE,
            notifier._WIN32GUI_AVAILABLE,
            notifier._send_winotify_toast,
            notifier._send_plyer_toast,
        ) = self._originals

    def test_falls_through_when_winotify_fails(self):
        notifier._WINOTIFY_AVAILABLE = True
        notifier._send_winotify_toast = lambda *a, **k: False
        notifier._PLYER_AVAILABLE = True
        notifier._send_plyer_toast = lambda *a, **k: True
        self.assertTrue(notifier.send_toast("t", "m"))

    def test_short_circuits_on_winotify_success(self):
        notifier._WINOTIFY_AVAILABLE = True
        notifier._send_winotify_toast = lambda *a, **k: True

        def should_not_be_called(*a, **k):
            raise AssertionError("plyer 不应在 winotify 成功后被调用")

        notifier._PLYER_AVAILABLE = True
        notifier._send_plyer_toast = should_not_be_called
        self.assertTrue(notifier.send_toast("t", "m"))

    def test_all_backends_fail_returns_false(self):
        notifier._WINOTIFY_AVAILABLE = True
        notifier._send_winotify_toast = lambda *a, **k: False
        notifier._PLYER_AVAILABLE = True
        notifier._send_plyer_toast = lambda *a, **k: False
        self.assertFalse(notifier.send_toast("t", "m"))


if __name__ == "__main__":
    unittest.main()
