import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from src.theme import ALL_TOKEN_KEYS, DARK, LIGHT, ThemeManager


class ThemeTokensTest(unittest.TestCase):
    def test_light_and_dark_have_same_keys(self):
        self.assertEqual(set(LIGHT), set(DARK))
        self.assertEqual(ALL_TOKEN_KEYS, frozenset(LIGHT.keys()))


class ThemeResolveTest(unittest.TestCase):
    @mock.patch("src.theme._system_is_dark", return_value=True)
    def test_system_resolves_dark(self, _mocked):
        self.assertEqual(ThemeManager("system").resolve(), "dark")

    @mock.patch("src.theme._system_is_dark", return_value=False)
    def test_system_resolves_light(self, _mocked):
        self.assertEqual(ThemeManager("system").resolve(), "light")

    def test_manual_override(self):
        self.assertEqual(ThemeManager("light").resolve(), "light")
        self.assertEqual(ThemeManager("dark").resolve(), "dark")


if __name__ == "__main__":
    unittest.main()
