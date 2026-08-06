use regex::Regex;
use std::sync::OnceLock;

static KEYWORDS: OnceLock<Regex> = OnceLock::new();
fn keywords() -> &'static Regex {
    KEYWORDS.get_or_init(|| {
        Regex::new(r"(验证码|动态码|校验码|安全码|一次性密码|OTP)").unwrap()
    })
}

/// 查找第一个前后都不是数字的 4-8 位数字段（避免正则环视限制）。
fn find_digit_run(text: &str) -> Option<String> {
    let mut start: Option<usize> = None;
    let mut count = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            if start.is_none() {
                start = Some(idx);
            }
            count += 1;
        } else if let Some(s) = start.take() {
            if (4..=8).contains(&count) {
                return Some(text[s..idx].to_string());
            }
            count = 0;
        }
    }
    if let Some(s) = start {
        if (4..=8).contains(&count) {
            return Some(text[s..].to_string());
        }
    }
    None
}

/// 提取 4-8 位纯数字验证码：优先关键词后 30 字符内，再全文首个独立数字段。
pub fn extract_otp(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    for m in keywords().find_iter(text) {
        let end = std::cmp::min(m.end() + 30, text.len());
        if let Some(otp) = find_digit_run(&text[m.end()..end]) {
            return Some(otp);
        }
    }
    find_digit_run(text)
}

#[cfg(test)]
mod tests {
    use super::extract_otp;

    #[test]
    fn google_message_returns_pure_digits() {
        assert_eq!(extract_otp("G-000000是您的Google验证码").as_deref(), Some("000000"));
    }

    #[test]
    fn keyword_before_digits() {
        assert_eq!(extract_otp("您的验证码是123456，5分钟内有效").as_deref(), Some("123456"));
    }

    #[test]
    fn keyword_colon() {
        assert_eq!(extract_otp("Google验证码：888888").as_deref(), Some("888888"));
    }

    #[test]
    fn too_long_not_matched() {
        assert_eq!(extract_otp("您的验证码是1234567890，已失效"), None);
    }

    #[test]
    fn no_digits() {
        assert_eq!(extract_otp("本次没有验证码"), None);
    }
}
