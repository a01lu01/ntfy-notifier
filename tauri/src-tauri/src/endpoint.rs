use reqwest::Url;
use std::net::IpAddr;

const MAX_TOPIC_LENGTH: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedEndpoint {
    pub server: Url,
    pub subscription: Url,
    pub loopback_http: bool,
}

pub(crate) fn validate_subscription_endpoint(
    server: &str,
    topic: &str,
    username: &str,
    password: &str,
    allow_insecure_http: bool,
) -> Result<ValidatedEndpoint, String> {
    if server != server.trim() {
        return Err("服务器地址前后不能包含空白字符".to_string());
    }
    let (raw_scheme, raw_authority) = explicit_authority(server)
        .ok_or_else(|| "服务器地址必须包含明确的协议和主机名".to_string())?;
    if !raw_scheme.eq_ignore_ascii_case("http") && !raw_scheme.eq_ignore_ascii_case("https") {
        return Err("服务器地址只支持 https:// 或 http://".to_string());
    }
    if server.contains('\\') {
        return Err("服务器地址不能包含反斜杠".to_string());
    }
    let server_url = Url::parse(server).map_err(|_| "服务器地址格式无效".to_string())?;
    if !matches!(server_url.scheme(), "http" | "https") {
        return Err("服务器地址只支持 https:// 或 http://".to_string());
    }
    if server_url.cannot_be_a_base() || server_url.host().is_none() {
        return Err("服务器地址必须包含有效主机名".to_string());
    }
    if raw_authority.contains('@')
        || !server_url.username().is_empty()
        || server_url.password().is_some()
    {
        return Err("服务器地址不能包含用户信息".to_string());
    }
    if server_url.query().is_some() {
        return Err("服务器地址不能包含查询参数".to_string());
    }
    if server_url.fragment().is_some() {
        return Err("服务器地址不能包含片段标识".to_string());
    }
    let loopback_http = server_url.scheme() == "http" && is_loopback_url(&server_url);
    if server_url.scheme() == "http" && !loopback_http && !allow_insecure_http {
        return Err("远程 HTTP 服务器必须明确允许不安全连接".to_string());
    }
    if !password.is_empty() && username.trim().is_empty() {
        return Err("填写密码时必须同时填写用户名".to_string());
    }
    validate_topic(topic)?;

    let mut subscription = server_url.clone();
    subscription
        .path_segments_mut()
        .map_err(|_| "服务器地址不能作为订阅基础地址".to_string())?
        .pop_if_empty()
        .push(topic)
        .push("sse");

    Ok(ValidatedEndpoint {
        server: server_url,
        subscription,
        loopback_http,
    })
}

pub(crate) fn requires_insecure_http_opt_in(server: &str) -> bool {
    let Ok(url) = Url::parse(server.trim()) else {
        return false;
    };
    url.scheme() == "http" && url.host().is_some() && !is_loopback_url(&url)
}

pub(crate) fn validate_redirect(
    previous: &[Url],
    target: &Url,
    allow_insecure_http: bool,
) -> Result<(), &'static str> {
    if previous.len() > 10 {
        return Err("重定向次数超过限制");
    }
    if !matches!(target.scheme(), "http" | "https") || target.host().is_none() {
        return Err("重定向目标协议或主机无效");
    }
    if authority_contains_userinfo(target.as_str())
        || !target.username().is_empty()
        || target.password().is_some()
    {
        return Err("重定向目标包含用户信息");
    }
    if previous
        .last()
        .is_some_and(|url| url.scheme() == "https" && target.scheme() != "https")
    {
        return Err("拒绝从 HTTPS 降级重定向");
    }
    if target.scheme() == "http" && !is_loopback_url(target) && !allow_insecure_http {
        return Err("重定向到远程 HTTP 需要明确允许");
    }
    Ok(())
}

fn validate_topic(topic: &str) -> Result<(), String> {
    if topic.is_empty() || topic.len() > MAX_TOPIC_LENGTH {
        return Err("主题长度必须为 1 到 64 个字符".to_string());
    }
    if !topic
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("主题只能包含字母、数字、连字符和下划线".to_string());
    }
    Ok(())
}

fn authority_contains_userinfo(raw_url: &str) -> bool {
    explicit_authority(raw_url).is_some_and(|(_, authority)| authority.contains('@'))
}

fn explicit_authority(raw_url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = raw_url.split_once("://")?;
    let authority = rest.split(['/', '?', '#', '\\']).next()?;
    (!authority.is_empty()).then_some((scheme, authority))
}

fn is_loopback_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    unbracketed
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(server: &str, topic: &str, allow_insecure_http: bool) -> Result<Url, String> {
        validate_subscription_endpoint(server, topic, "", "", allow_insecure_http)
            .map(|endpoint| endpoint.subscription)
    }

    #[test]
    fn accepts_https_and_builds_endpoint_from_base_path() {
        assert_eq!(
            validate("HTTPS://example.com/ntfy/", "Alerts_1-2", false)
                .unwrap()
                .as_str(),
            "https://example.com/ntfy/Alerts_1-2/sse"
        );
    }

    #[test]
    fn loopback_http_does_not_require_opt_in() {
        for server in [
            "http://localhost:8080",
            "HTTP://LOCALHOST:8080",
            "http://127.0.0.42",
            "http://127.1",
            "http://[::1]:8080",
            "http://[0:0:0:0:0:0:0:1]",
        ] {
            let endpoint = validate_subscription_endpoint(server, "alerts", "", "", false)
                .unwrap_or_else(|error| panic!("{server}: {error}"));
            assert!(endpoint.loopback_http, "{server}");
            assert!(!requires_insecure_http_opt_in(server), "{server}");
        }
    }

    #[test]
    fn remote_http_requires_explicit_opt_in() {
        for server in [
            "http://example.com",
            "HTTP://EXAMPLE.COM",
            "http://192.168.1.10",
            "http://0.0.0.0",
            "http://localhost.evil",
        ] {
            assert!(validate(server, "alerts", false).is_err(), "{server}");
            assert!(validate(server, "alerts", true).is_ok(), "{server}");
            assert!(requires_insecure_http_opt_in(server), "{server}");
        }
    }

    #[test]
    fn rejects_unsupported_or_ambiguous_server_urls() {
        for server in [
            "",
            "example.com",
            "https:example.com",
            "https:/example.com",
            "https:///example.com",
            "https:\\example.com",
            "https://\\example.com",
            "ftp://example.com",
            "file:///tmp/ntfy",
            "https://user@example.com",
            "https://user:pass@example.com",
            "https://@example.com",
            "https://example.com?token=secret",
            "https://example.com?",
            "https://example.com#fragment",
            "https://example.com#",
            " https://example.com",
        ] {
            assert!(validate(server, "alerts", true).is_err(), "{server}");
        }
    }

    #[test]
    fn enforces_ntfy_topic_shape_and_length() {
        assert!(validate("https://example.com", "a", false).is_ok());
        assert!(validate("https://example.com", &"a".repeat(64), false).is_ok());
        for topic in [
            "".to_string(),
            "a".repeat(65),
            "has.dot".to_string(),
            "has/slash".to_string(),
            "has space".to_string(),
            "中文".to_string(),
        ] {
            assert!(
                validate("https://example.com", &topic, false).is_err(),
                "{topic}"
            );
        }
    }

    #[test]
    fn password_requires_non_blank_username() {
        assert!(validate_subscription_endpoint(
            "https://example.com",
            "alerts",
            " ",
            "secret",
            false,
        )
        .is_err());
        assert!(validate_subscription_endpoint(
            "https://example.com",
            "alerts",
            "alice",
            "",
            false,
        )
        .is_ok());
    }

    #[test]
    fn redirect_policy_rejects_downgrades_and_unapproved_remote_http() {
        let https = Url::parse("https://example.com/topic/sse").unwrap();
        let other_https = Url::parse("https://cdn.example.com/topic/sse").unwrap();
        let remote_http = Url::parse("http://example.com/topic/sse").unwrap();
        let loopback_http = Url::parse("http://127.0.0.1/topic/sse").unwrap();

        assert!(validate_redirect(std::slice::from_ref(&https), &remote_http, true).is_err());
        assert!(validate_redirect(std::slice::from_ref(&https), &other_https, false).is_ok());
        assert!(
            validate_redirect(std::slice::from_ref(&loopback_http), &remote_http, false).is_err()
        );
        assert!(
            validate_redirect(std::slice::from_ref(&loopback_http), &remote_http, true).is_ok()
        );
        assert!(validate_redirect(std::slice::from_ref(&loopback_http), &https, false).is_ok());
    }

    #[test]
    fn redirect_policy_limits_chain_and_rejects_userinfo() {
        let target = Url::parse("https://example.com/topic/sse").unwrap();
        let allowed_previous = vec![target.clone(); 10];
        let previous = vec![target.clone(); 11];
        assert!(validate_redirect(&allowed_previous, &target, false).is_ok());
        assert!(validate_redirect(&previous, &target, false).is_err());

        let userinfo = Url::parse("https://user@example.com/topic/sse").unwrap();
        assert!(validate_redirect(&[], &userinfo, false).is_err());
    }
}
