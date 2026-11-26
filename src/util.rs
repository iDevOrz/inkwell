use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, FixedOffset, Utc};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::fill(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

pub fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    left.len() == right.len() && left.ct_eq(right).into()
}

pub fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Escapes text for XML and drops characters XML 1.0 forbids (most C0
/// controls, U+FFFE/U+FFFF); a single one would make a whole feed unparsable.
pub fn xml_escape(value: &str) -> String {
    let allowed: String = value.chars().filter(|ch| is_xml_char(*ch)).collect();
    html_escape(&allowed)
}

fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r') || (ch >= ' ' && ch != '\u{FFFE}' && ch != '\u{FFFF}')
}

/// Characters left as-is in a URL path segment (RFC 3986 unreserved).
const PATH_SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Percent-encodes a slug for use in absolute URLs such as sitemap `<loc>`,
/// which must be valid URIs (Chinese slugs are otherwise emitted raw).
pub fn path_segment(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, PATH_SEGMENT).to_string()
}

pub fn timezone(offset_minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(offset_minutes.saturating_mul(60))
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC offset is valid"))
}

pub fn format_date(timestamp: i64, offset_minutes: i32, format: &str) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|date| {
            date.with_timezone(&timezone(offset_minutes))
                .format(format)
                .to_string()
        })
        .unwrap_or_default()
}

pub fn format_datetime(timestamp: i64, offset_minutes: i32) -> String {
    format_date(timestamp, offset_minutes, "%Y-%m-%d %H:%M")
}

pub fn format_human_date(timestamp: i64, offset_minutes: i32) -> String {
    format_date(timestamp, offset_minutes, "%Y年%m月%d日")
}

pub fn format_iso(timestamp: i64, offset_minutes: i32) -> String {
    format_date(timestamp, offset_minutes, "%Y-%m-%dT%H:%M:%S%:z")
}

pub fn format_month(timestamp: i64, offset_minutes: i32) -> String {
    format_date(timestamp, offset_minutes, "%Y年%m月")
}

pub fn format_bytes(size: i64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut value = size.max(0) as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value as i64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (index, character) in value.chars().enumerate() {
        if index >= max_chars {
            result.push('…');
            break;
        }
        result.push(character);
    }
    result
}

pub fn slugify(input: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;

    for character in input.trim().chars() {
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                output.push(lower);
            }
            last_dash = false;
        } else if (character.is_whitespace() || matches!(character, '-' | '_' | '/' | '.' | ':'))
            && !last_dash
            && !output.is_empty()
        {
            output.push('-');
            last_dash = true;
        }
    }

    let output = output.trim_matches('-').to_string();
    let output = truncate_chars(&output, 120);
    let output = output.trim_end_matches('…').trim_matches('-').to_string();

    if output.is_empty() {
        "item".to_string()
    } else {
        output
    }
}

pub fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

pub fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn safe_next(value: Option<&str>) -> String {
    match value {
        Some(next)
            if next.starts_with('/')
                && !next.starts_with("//")
                && !next.contains('\\')
                // Browsers drop tabs/newlines while parsing Location, so any
                // control character could turn "/\t/host" into "//host".
                && !next.chars().any(char::is_control) =>
        {
            next.to_string()
        }
        _ => "/admin".to_string(),
    }
}

pub fn client_ip(
    headers: &axum::http::HeaderMap,
    peer: std::net::SocketAddr,
    trust_proxy_headers: bool,
) -> String {
    if trust_proxy_headers {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            // Only the right-most hop was written by the trusted proxy;
            // anything to its left is client-controlled.
            .and_then(|value| value.rsplit(',').next())
            .map(str::trim);
        let real_ip = headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .map(str::trim);

        if let Some(ip) = forwarded
            .or(real_ip)
            .and_then(|value| value.parse::<std::net::IpAddr>().ok())
        {
            return ip.to_string();
        }
    }

    peer.ip().to_string()
}

pub fn user_agent(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| truncate_chars(value, 300))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_chinese_and_spaces() {
        assert_eq!(slugify("Hello Rust World"), "hello-rust-world");
        assert_eq!(slugify("  Rust 笔记 / 第一篇  "), "rust-笔记-第一篇");
        assert_eq!(slugify("!!!"), "item");
    }

    #[test]
    fn escapes_html() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn xml_escape_drops_characters_invalid_in_xml() {
        assert_eq!(xml_escape("a\u{0B}b\u{0C}c\u{FFFF}<"), "abc&lt;");
        assert_eq!(xml_escape("tab\tline\n"), "tab\tline\n");
    }

    #[test]
    fn path_segment_encodes_non_ascii_and_reserved() {
        assert_eq!(path_segment("rust-笔记"), "rust-%E7%AC%94%E8%AE%B0");
        assert_eq!(path_segment("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(path_segment("ok-1_2.3~"), "ok-1_2.3~");
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("100%_ok"), "%100\\%\\_ok%");
    }

    #[test]
    fn safe_next_rejects_external_urls() {
        assert_eq!(safe_next(Some("/admin/posts")), "/admin/posts");
        assert_eq!(safe_next(Some("https://evil.example")), "/admin");
        assert_eq!(safe_next(Some("//evil.example")), "/admin");
    }

    #[test]
    fn safe_next_rejects_control_characters() {
        // Browsers strip tabs/newlines while parsing Location, turning
        // "/\t/evil.example" into the protocol-relative "//evil.example".
        assert_eq!(safe_next(Some("/\t/evil.example")), "/admin");
        assert_eq!(safe_next(Some("/\u{0}/evil.example")), "/admin");
        assert_eq!(
            safe_next(Some("/admin/posts?page=2")),
            "/admin/posts?page=2"
        );
    }

    #[test]
    fn forwarded_ip_is_ignored_unless_proxy_trust_is_enabled() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.77".parse().unwrap());
        let peer = "192.0.2.10:3000".parse().unwrap();

        assert_eq!(client_ip(&headers, peer, false), "192.0.2.10");
        assert_eq!(client_ip(&headers, peer, true), "198.51.100.77");
    }

    #[test]
    fn forwarded_ip_uses_hop_appended_by_trusted_proxy() {
        // A client can prepend arbitrary values; only the right-most entry was
        // written by the trusted proxy in front of the app.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.9, 198.51.100.77".parse().unwrap(),
        );
        let peer = "192.0.2.10:3000".parse().unwrap();

        assert_eq!(client_ip(&headers, peer, true), "198.51.100.77");
    }
}
