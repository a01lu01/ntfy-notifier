use crate::otp::{extract_after_keyword, extract_digit_run, KEYWORD_WINDOW};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub keywords: Vec<String>,
    pub min_length: i64,
    pub max_length: i64,
    pub match_mode: String,
    pub enabled: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            keywords: Vec::new(),
            min_length: 4,
            max_length: 8,
            match_mode: "both".to_string(),
            enabled: true,
        }
    }
}

pub fn default_rules() -> Vec<Rule> {
    vec![Rule {
        id: "default".to_string(),
        name: "默认规则".to_string(),
        keywords: vec![
            "验证码".to_string(),
            "动态码".to_string(),
            "校验码".to_string(),
            "安全码".to_string(),
            "一次性密码".to_string(),
            "OTP".to_string(),
        ],
        min_length: 4,
        max_length: 8,
        match_mode: "both".to_string(),
        enabled: true,
    }]
}

fn rules_path() -> PathBuf {
    crate::appdata::resolve().join("rules.json")
}

pub fn load() -> Vec<Rule> {
    let path = rules_path();
    if !path.exists() {
        let rules = default_rules();
        let _ = save(&rules);
        return rules;
    }
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return default_rules(),
    };
    let parsed: Vec<Rule> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let backup = path.with_file_name(format!("rules.json.corrupt-{stamp}"));
            let _ = fs::rename(&path, &backup);
            let rules = default_rules();
            let _ = save(&rules);
            return rules;
        }
    };
    parsed.into_iter().map(sanitize_rule).collect()
}

fn sanitize_rule(mut rule: Rule) -> Rule {
    rule.min_length = rule.min_length.max(1);
    rule.max_length = rule.max_length.max(rule.min_length);
    if !matches!(rule.match_mode.as_str(), "keyword_only" | "whole_text" | "both") {
        rule.match_mode = "both".to_string();
    }
    rule
}

pub fn save(rules: &[Rule]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    let path = rules_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn match_rule(text: &str, rule: &Rule) -> Option<String> {
    let min = rule.min_length.max(1) as usize;
    let max = rule.max_length.max(rule.min_length) as usize;
    match rule.match_mode.as_str() {
        "keyword_only" => extract_after_keyword(text, &rule.keywords, min, max, KEYWORD_WINDOW),
        "whole_text" => extract_digit_run(text, min, max),
        _ => extract_after_keyword(text, &rule.keywords, min, max, KEYWORD_WINDOW)
            .or_else(|| extract_digit_run(text, min, max)),
    }
}

pub fn find_otp(text: &str, rules: &[Rule]) -> Option<String> {
    for rule in rules {
        if !rule.enabled {
            continue;
        }
        if let Some(otp) = match_rule(text, rule) {
            return Some(otp);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::MutexGuard;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_env() -> MutexGuard<'static, ()> {
        let guard = crate::appdata::test_lock().lock().unwrap();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ntfy-test-rules-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::appdata::set(dir);
        guard
    }

    fn rule(
        id: &str,
        name: &str,
        keywords: &[&str],
        min_len: i64,
        max_len: i64,
        mode: &str,
        enabled: bool,
    ) -> Rule {
        Rule {
            id: id.to_string(),
            name: name.to_string(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            min_length: min_len,
            max_length: max_len,
            match_mode: mode.to_string(),
            enabled,
        }
    }

    #[test]
    fn default_rules_shape() {
        let rules = default_rules();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].enabled);
        assert_eq!(rules[0].match_mode, "both");
        assert!(rules[0].keywords.contains(&"验证码".to_string()));
    }

    #[test]
    fn load_creates_default_rules_when_missing() {
        let _guard = unique_env();
        let rules = load();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "默认规则");
    }

    #[test]
    fn save_load_roundtrip() {
        let _guard = unique_env();
        let custom = vec![rule("a", "规则A", &["A码"], 4, 8, "both", false)];
        save(&custom).unwrap();
        let loaded = load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(loaded[0].enabled, false);
        assert_eq!(loaded[0].keywords, vec!["A码".to_string()]);
    }

    #[test]
    fn load_recovers_from_corrupt_file() {
        let _guard = unique_env();
        std::fs::create_dir_all(rules_path().parent().unwrap()).unwrap();
        std::fs::write(&rules_path(), "not json").unwrap();
        let rules = load();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "默认规则");
    }

    #[test]
    fn find_otp_uses_default_keywords() {
        let _guard = unique_env();
        assert_eq!(find_otp("您的验证码是123456", &load()).as_deref(), Some("123456"));
    }

    #[test]
    fn find_otp_first_enabled_match_wins_by_order() {
        let _guard = unique_env();
        let a = rule("a", "A", &["A码"], 4, 8, "both", true);
        let b = rule("b", "B", &["B码"], 4, 8, "both", true);
        let text = "A码1234 B码567890";
        assert_eq!(find_otp(text, &[a.clone(), b.clone()]).as_deref(), Some("1234"));
        assert_eq!(find_otp(text, &[b, a]).as_deref(), Some("567890"));
    }

    #[test]
    fn find_otp_skips_disabled_rules() {
        let _guard = unique_env();
        let disabled = rule("d", "D", &["A码"], 4, 8, "both", false);
        assert_eq!(find_otp("A码1234", &[disabled]), None);
    }

    #[test]
    fn match_rule_keyword_only_does_not_fallback() {
        let _guard = unique_env();
        let r = rule("k", "K", &["验证码"], 4, 8, "keyword_only", true);
        assert_eq!(match_rule("123456", &r), None);
        assert_eq!(match_rule("验证码123456", &r).as_deref(), Some("123456"));
    }

    #[test]
    fn match_rule_whole_text_ignores_keywords() {
        let _guard = unique_env();
        let r = rule("w", "W", &["验证码"], 4, 8, "whole_text", true);
        assert_eq!(match_rule("123456", &r).as_deref(), Some("123456"));
    }

    #[test]
    fn match_rule_both_falls_back_to_whole_text() {
        let _guard = unique_env();
        let r = rule("b", "B", &["验证码"], 4, 8, "both", true);
        assert_eq!(match_rule("123456", &r).as_deref(), Some("123456"));
    }

    #[test]
    fn match_rule_unknown_mode_treated_as_both() {
        let _guard = unique_env();
        let r = rule("u", "U", &["验证码"], 4, 8, "bogus", true);
        assert_eq!(match_rule("验证码123456", &r).as_deref(), Some("123456"));
        assert_eq!(match_rule("123456", &r).as_deref(), Some("123456"));
    }

    #[test]
    fn match_rule_clamps_invalid_lengths() {
        let _guard = unique_env();
        let r = rule("c", "C", &["验证码"], 10, 4, "both", true);
        assert_eq!(match_rule("验证码1234567890", &r).as_deref(), Some("1234567890"));
    }
}
