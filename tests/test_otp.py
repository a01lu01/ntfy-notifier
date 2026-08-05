import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from src.notifier import _extract_otp


class OtpExtractionTest(unittest.TestCase):
    def test_google_code_message_returns_pure_digits(self):
        self.assertEqual(_extract_otp("G-000000是您的Google验证码"), "000000")

    def test_keyword_before_digits(self):
        self.assertEqual(_extract_otp("您的验证码是123456，5分钟内有效"), "123456")

    def test_keyword_colon(self):
        self.assertEqual(_extract_otp("Google验证码：888888"), "888888")

    def test_keyword_after_digits(self):
        self.assertEqual(_extract_otp("654321是您的动态码，请勿泄露"), "654321")

    def test_keyword_priority_over_earlier_number(self):
        self.assertEqual(_extract_otp("订单号20260805，验证码112233"), "112233")

    def test_embedded_digits_in_word(self):
        self.assertEqual(_extract_otp("abc1234xyz"), "1234")

    def test_too_long_number_not_matched(self):
        self.assertIsNone(_extract_otp("您的验证码是1234567890，已失效"))

    def test_no_digits(self):
        self.assertIsNone(_extract_otp("本次没有验证码"))

    def test_empty(self):
        self.assertIsNone(_extract_otp(""))
        self.assertIsNone(_extract_otp(None))


if __name__ == "__main__":
    unittest.main()
