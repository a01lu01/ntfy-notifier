/// 关键词后查找数字段的窗口大小（字符数）。
pub const KEYWORD_WINDOW: usize = 30;

/// 在文本中查找第一个长度落在 [min_len, max_len] 内的独立数字段。
/// 独立数字段 = 连续数字且前后不是数字（或文本边界）。
pub fn extract_digit_run(text: &str, min_len: usize, max_len: usize) -> Option<String> {
    let mut start: Option<usize> = None;
    let mut count = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            if start.is_none() {
                start = Some(idx);
            }
            count += 1;
        } else if let Some(s) = start.take() {
            if (min_len..=max_len).contains(&count) {
                return Some(text[s..idx].to_string());
            }
            count = 0;
        }
    }
    if let Some(s) = start {
        if (min_len..=max_len).contains(&count) {
            return Some(text[s..].to_string());
        }
    }
    None
}

/// 在关键词后 `window` 字符内查找第一个长度合规的独立数字段。
/// 多个关键词按出现位置从左到右依次尝试，任一命中即返回。
pub fn extract_after_keyword(
    text: &str,
    keywords: &[String],
    min_len: usize,
    max_len: usize,
    window: usize,
) -> Option<String> {
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for kw in keywords {
        if kw.is_empty() {
            continue;
        }
        for (idx, _) in text.match_indices(kw) {
            hits.push((idx, idx + kw.len()));
        }
    }
    hits.sort_unstable();
    for (_, end) in hits {
        let to = std::cmp::min(end + window, text.len());
        if let Some(otp) = extract_digit_run(&text[end..to], min_len, max_len) {
            return Some(otp);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_run_extracts_first_qualifying_run() {
        assert_eq!(
            extract_digit_run("您的验证码是123456，5分钟内有效", 4, 8).as_deref(),
            Some("123456")
        );
    }

    #[test]
    fn digit_run_matches_embedded_run() {
        assert_eq!(extract_digit_run("abc1234xyz", 4, 8).as_deref(), Some("1234"));
    }

    #[test]
    fn digit_run_rejects_too_long_run() {
        assert_eq!(extract_digit_run("验证码是1234567890，已失效", 4, 8), None);
    }

    #[test]
    fn digit_run_rejects_too_short_for_min() {
        assert_eq!(extract_digit_run("验证码8888", 6, 8), None);
    }

    #[test]
    fn digit_run_honors_fixed_length() {
        assert_eq!(extract_digit_run("验证码123456", 6, 6).as_deref(), Some("123456"));
    }

    #[test]
    fn digit_run_no_digits() {
        assert_eq!(extract_digit_run("本次没有验证码", 4, 8), None);
    }

    #[test]
    fn digit_run_empty_text() {
        assert_eq!(extract_digit_run("", 4, 8), None);
    }

    #[test]
    fn after_keyword_ignores_leading_number() {
        assert_eq!(
            extract_after_keyword(
                "订单号20260805，验证码112233",
                &["验证码".to_string()],
                4,
                8,
                30
            )
            .as_deref(),
            Some("112233")
        );
    }

    #[test]
    fn after_keyword_honors_window() {
        let text = format!("验证码{}123456", "x".repeat(33));
        assert_eq!(extract_after_keyword(&text, &["验证码".to_string()], 4, 8, 30), None);
        assert_eq!(
            extract_after_keyword(&text, &["验证码".to_string()], 4, 8, 40).as_deref(),
            Some("123456")
        );
    }

    #[test]
    fn after_keyword_leftmost_occurrence_wins() {
        assert_eq!(
            extract_after_keyword(
                "验证码1234 动态码5678",
                &["验证码".to_string(), "动态码".to_string()],
                4,
                8,
                30
            )
            .as_deref(),
            Some("1234")
        );
    }

    #[test]
    fn after_keyword_no_keyword() {
        assert_eq!(extract_after_keyword("123456", &["验证码".to_string()], 4, 8, 30), None);
    }

    #[test]
    fn after_keyword_rejects_too_long_run() {
        assert_eq!(
            extract_after_keyword("验证码是1234567890", &["验证码".to_string()], 4, 8, 30),
            None
        );
    }
}
