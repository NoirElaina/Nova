//! Panic-safe secret redaction and UTF-8 truncation helpers shared across Nova crates.

const REDACTED: &str = "[REDACTED]";

const SECRET_PREFIXES: &[&str] = &[
    "sk-proj-",
    "sk-ant-",
    "sk-svcacct-",
    "sk-",
    "sk_",
    "rk_",
    "pk_live_",
    "pk_test_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "AIza",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xoxs-",
    "AKIA",
    "ASIA",
    "nova:v2:",
    "nova:dpapi:v1:",
];

const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "auth_token",
    "bearer_token",
    "session_token",
    "id_token",
    "secret_key",
    "master_key",
    "client_secret",
    "private_key",
    "encryption_key",
    "signing_key",
    "authorization",
    "x-api-key",
    "password",
    "passwd",
    "secret",
    "token",
];

/// Truncates `input` cleanly at a UTF-8 character boundary within `max_bytes`.
pub fn truncate_utf8(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

/// Extracts, secret-scrubs, flattens newlines, and truncates a panic location + message summary.
pub fn format_panic_summary(info: &std::panic::PanicHookInfo<'_>, max_bytes: usize) -> (String, String) {
    let location = info
        .location()
        .map(|loc| format!("{}:{}", loc.file(), loc.line()))
        .unwrap_or_else(|| "unknown".to_string());

    let payload = if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    };

    let scrubbed = redact_secrets(&payload);
    let summary = truncate_utf8(&scrubbed, max_bytes).replace(['\r', '\n'], " ");
    (location, summary)
}

/// Scrubs common API tokens, credentials, URL passwords, PEM private keys, and
/// keyed secrets (`api_key=...`, `"token": "..."`, `Authorization: Bearer ...`)
/// from log and panic strings without panicking or splitting UTF-8 characters.
pub fn redact_secrets(input: &str) -> String {
    let stage1 = scrub_pem_blocks(input);
    let stage2 = scrub_url_userinfo(&stage1);
    let stage3 = scrub_keyed_values(&stage2);
    scrub_prefixed_tokens(&stage3)
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect();
    if matches!(
        normalized.as_str(),
        "xapikey"
            | "xgoogapikey"
            | "apikey"
            | "authorization"
            | "accesstoken"
            | "refreshtoken"
            | "authtoken"
            | "bearertoken"
            | "sessiontoken"
            | "idtoken"
            | "secretkey"
            | "masterkey"
            | "clientsecret"
            | "privatekey"
            | "encryptionkey"
            | "signingkey"
            | "password"
            | "passwd"
            | "secret"
            | "token"
            | "cookietoken"
            | "cookie"
            | "setcookie"
    ) {
        return true;
    }
    SENSITIVE_KEYS.iter().any(|k| {
        k.chars()
            .filter(|c| *c != '-' && *c != '_')
            .collect::<String>()
            == normalized
    })
}

/// Recursively scrubs secret strings and sensitive JSON object keys while
/// preserving JSON structure for safe wire diagnostics.
pub fn redact_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact_secrets(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_json_key(k) {
                    let scrubbed = match v {
                        serde_json::Value::Null => serde_json::Value::Null,
                        serde_json::Value::String(s) if s.is_empty() => {
                            serde_json::Value::String(String::new())
                        }
                        _ => serde_json::Value::String(REDACTED.to_string()),
                    };
                    out.insert(k.clone(), scrubbed);
                } else {
                    out.insert(k.clone(), redact_json_value(v));
                }
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

fn scrub_pem_blocks(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(begin) = rest.find("-----BEGIN ") {
        let after_begin = &rest[begin..];
        if let Some(header_end) = after_begin.find("-----") {
            let header_tail = &after_begin[header_end + 5..];
            if let Some(second_marker) = header_tail.find("-----") {
                let header = &after_begin[..header_end + 5 + second_marker + 5];
                if header.contains("PRIVATE KEY") {
                    out.push_str(&rest[..begin]);
                    if let Some(end_pos) = after_begin[header.len()..].find("-----END ") {
                        let tail = &after_begin[header.len() + end_pos..];
                        if let Some(end_dash) = tail[9..].find("-----") {
                            out.push_str("[REDACTED PRIVATE KEY]");
                            rest = &tail[9 + end_dash + 5..];
                            continue;
                        }
                    }
                    out.push_str("[REDACTED PRIVATE KEY]");
                    return out;
                }
            }
        }
        out.push_str(&rest[..begin + 11]);
        rest = &rest[begin + 11..];
    }
    out.push_str(rest);
    out
}

fn scrub_url_userinfo(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        if let Some(rel) = rest.find("://") {
            let after_scheme = i + rel + 3;
            out.push_str(&input[i..after_scheme]);
            let authority_rest = &input[after_scheme..];
            let authority_end = authority_rest
                .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '"' | '\'' | ')'))
                .unwrap_or(authority_rest.len());
            let authority = &authority_rest[..authority_end];
            if let Some(at_pos) = authority.find('@') {
                if authority[..at_pos].contains(':') {
                    out.push_str(REDACTED);
                    out.push('@');
                    out.push_str(&authority[at_pos + 1..]);
                    i = after_scheme + authority_end;
                    continue;
                }
            }
            out.push_str(authority);
            i = after_scheme + authority_end;
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}

fn scrub_keyed_values(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        if let Some(after_auth) = match_auth_scheme(rest) {
            let scheme_len = rest.len() - after_auth.len();
            out.push_str(&rest[..scheme_len]);
            let token_len = after_auth
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | ')' | '}'))
                .unwrap_or(after_auth.len());
            if token_len >= 6 {
                out.push_str(REDACTED);
                i += scheme_len + token_len;
                continue;
            }
        }

        if is_token_boundary(input, i) {
            if let Some((prefix_len, value_offset, quote_char)) = match_sensitive_key(rest) {
                out.push_str(&rest[..prefix_len]);
                let after_sep = &rest[prefix_len + value_offset..];
                out.push_str(&rest[prefix_len..prefix_len + value_offset]);
                if let Some(q) = quote_char {
                    out.push(q);
                    let quoted = &after_sep[q.len_utf8()..];
                    if let Some(end_q) = find_closing_quote(quoted, q) {
                        out.push_str(REDACTED);
                        out.push(q);
                        i += prefix_len + value_offset + q.len_utf8() + end_q + q.len_utf8();
                        continue;
                    }
                    out.push_str(REDACTED);
                    return out;
                } else if let Some((scheme_len, token_len)) = auth_scheme_span(after_sep) {
                    out.push_str(&after_sep[..scheme_len]);
                    out.push_str(REDACTED);
                    i += prefix_len + value_offset + scheme_len + token_len;
                    continue;
                } else {
                    let val_len = after_sep
                        .find(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ')' | ']' | '}' | '"' | '\''))
                        .unwrap_or(after_sep.len());
                    if val_len > 0 {
                        out.push_str(REDACTED);
                        i += prefix_len + value_offset + val_len;
                        continue;
                    }
                }
            }
        }

        if let Some(ch) = rest.chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn auth_scheme_span(rest: &str) -> Option<(usize, usize)> {
    let after_auth = match_auth_scheme(rest)?;
    let scheme_len = rest.len() - after_auth.len();
    let token_len = after_auth
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | ')' | '}'))
        .unwrap_or(after_auth.len());
    (token_len >= 6).then_some((scheme_len, token_len))
}

fn match_auth_scheme(rest: &str) -> Option<&str> {
    for scheme in ["Bearer ", "bearer ", "BEARER ", "Basic ", "basic ", "BASIC "] {
        if let Some(after) = rest.strip_prefix(scheme) {
            return Some(after);
        }
    }
    None
}

fn is_token_boundary(full: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    match full[..idx].chars().next_back() {
        Some(c) => !c.is_ascii_alphanumeric() && c != '_' && c != '-',
        None => true,
    }
}

fn match_sensitive_key(rest: &str) -> Option<(usize, usize, Option<char>)> {
    let (quote_offset, key_quote, after_open) = if let Some(s) = rest.strip_prefix('"') {
        (1, Some('"'), s)
    } else if let Some(s) = rest.strip_prefix('\'') {
        (1, Some('\''), s)
    } else {
        (0, None, rest)
    };

    for &key in SENSITIVE_KEYS {
        if after_open.len() < key.len() {
            continue;
        }
        let prefix = after_open.get(..key.len())?;
        if !prefix.eq_ignore_ascii_case(key) {
            continue;
        }
        let mut cursor = quote_offset + key.len();
        let after_key = &rest[cursor..];
        let after_close_quote = if let Some(q) = key_quote {
            let Some(stripped) = after_key.strip_prefix(q) else {
                continue;
            };
            cursor += q.len_utf8();
            stripped
        } else {
            // Ensure full identifier matched before separator.
            if after_key
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                continue;
            }
            after_key
        };

        let trimmed_ws = after_close_quote.trim_start_matches(' ');
        let ws_before_sep = after_close_quote.len() - trimmed_ws.len();
        let Some(sep_char) = trimmed_ws.chars().next() else {
            continue;
        };
        if sep_char != '=' && sep_char != ':' {
            continue;
        }
        let sep_end = cursor + ws_before_sep + 1;
        let after_sep = &rest[sep_end..];
        let ws_after_sep = after_sep.len() - after_sep.trim_start_matches(' ').len();
        let val_start = &rest[sep_end + ws_after_sep..];
        let val_quote = val_start
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'');
        return Some((sep_end, ws_after_sep, val_quote));
    }
    None
}

fn find_closing_quote(s: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            return Some(idx);
        }
    }
    None
}

fn scrub_prefixed_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        let mut matched_len = None;
        if is_prefix_boundary(input, i) {
            for &prefix in SECRET_PREFIXES {
                if let Some(after) = rest.strip_prefix(prefix) {
                    let body_len = after
                        .find(|c: char| !is_token_char(c))
                        .unwrap_or(after.len());
                    let min_body = if prefix == "AIza" {
                        16
                    } else if prefix == "AKIA" || prefix == "ASIA" {
                        12
                    } else {
                        6
                    };
                    if body_len >= min_body {
                        matched_len = Some(prefix.len() + body_len);
                        break;
                    }
                }
            }
        }

        if let Some(total) = matched_len {
            out.push_str(REDACTED);
            i += total;
        } else if let Some(ch) = rest.chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn is_prefix_boundary(full: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    match full[..idx].chars().next_back() {
        Some(c) => !c.is_ascii_alphanumeric() && c != '_',
        None => true,
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

#[cfg(test)]
mod tests {
    use super::{redact_secrets, truncate_utf8};

    #[test]
    fn redacts_api_tokens_and_headers() {
        let raw = "failed calling https://user:topsecret@api.example.com/v1 with Bearer eyJhbGciOiJIUzI1NiJ9.abc.xyz and sk-proj-1234567890abcdef";
        let scrubbed = redact_secrets(raw);
        assert!(!scrubbed.contains("topsecret"));
        assert!(!scrubbed.contains("eyJhbGci"));
        assert!(!scrubbed.contains("sk-proj-1234567890abcdef"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_json_and_kv_secrets_while_preserving_normal_text() {
        let raw = r#"{"api_key": "secret-val-123", "model": "gpt-5"} password=my_pw token='tok_abc'"#;
        let scrubbed = redact_secrets(raw);
        assert!(scrubbed.contains(r#""model": "gpt-5""#));
        assert!(!scrubbed.contains("secret-val-123"));
        assert!(!scrubbed.contains("my_pw"));
        assert!(!scrubbed.contains("tok_abc"));
    }

    #[test]
    fn redacts_master_key_prefix_and_pem() {
        let raw = "key=nova:v2:YWJjZGVmMTIzNDU2 -----BEGIN PRIVATE KEY----- MIIEvgIBADANBgk -----END PRIVATE KEY-----";
        let scrubbed = redact_secrets(raw);
        assert!(!scrubbed.contains("YWJjZGVmMTIzNDU2"));
        assert!(!scrubbed.contains("MIIEvgIBADANBgk"));
        assert!(scrubbed.contains("[REDACTED PRIVATE KEY]"));
    }

    #[test]
    fn truncate_utf8_respects_char_boundaries() {
        let text = "错误原因：密钥失效";
        let cut = truncate_utf8(text, 7);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
        assert!(cut.len() <= 7);
    }

    #[test]
    fn redact_json_value_scrubs_headers_and_nested_secrets() {
        let raw = serde_json::json!({
            "model": "claude-sonnet",
            "headers": {
                "x-api-key": "custom-secret-value",
                "x-goog-api-key": "AIzaSyA1234567890123456",
                "content-type": "application/json"
            },
            "messages": [
                { "role": "user", "content": "connect to https://u:pass123@db.local/app with sk-proj-abcdef123456" }
            ]
        });
        let redacted = super::redact_json_value(&raw);
        let serialized = redacted.to_string();
        assert!(!serialized.contains("custom-secret-value"));
        assert!(!serialized.contains("AIzaSy"));
        assert!(!serialized.contains("pass123"));
        assert!(!serialized.contains("sk-proj-abcdef123456"));
        assert_eq!(redacted["model"], "claude-sonnet");
        assert_eq!(redacted["headers"]["content-type"], "application/json");
        assert_eq!(redacted["headers"]["x-api-key"], "[REDACTED]");
    }
}
