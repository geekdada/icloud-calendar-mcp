//! Self-contained, opaque references to CalDAV events.
//!
//! A bare UID can only be resolved through the in-memory cache, which is empty
//! in a fresh process and expires after five minutes. A handle instead carries
//! the coordinates the server needs (the partition-normalized href and the
//! last-known ETag) in one URL-safe token, so any process can act on the event.
//!
//! Encoding (stable, so clients and persisted handles can rely on it):
//! - master: `evt1_` + base64url-no-pad(`href \n etag`)
//! - occurrence: `evt2_` + base64url-no-pad(`href \n etag \n recurrenceId`),
//!   where recurrenceId is the RECURRENCE-ID wire form (`20260818T140000Z` or
//!   `20260818`) of one instance of a recurring series.

use std::sync::LazyLock;

use base64::Engine;
use base64::alphabet::URL_SAFE;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use regex::Regex;
use reqwest::Url;

use crate::caldav::normalize_icloud_url;

/// Prefix of a master (whole-event) handle.
pub const PREFIX: &str = "evt1_";
/// Prefix of an occurrence handle.
pub const PREFIX_V2: &str = "evt2_";

/// Encodes without padding; decodes with or without it.
const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new().with_encode_padding(false).with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"` (RFC 3986 §3.1).
static SCHEME_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:").expect("valid regex"));

/// A decoded event reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventHandle {
    /// Partition-normalized href of the event resource.
    pub href: String,
    /// Last-known ETag.
    pub etag: Option<String>,
    /// RECURRENCE-ID of one occurrence; `None` for a whole-event reference.
    pub recurrence_id: Option<String>,
}

impl EventHandle {
    /// True when this handle targets one occurrence of a series.
    pub fn is_occurrence_ref(&self) -> bool {
        self.recurrence_id.is_some()
    }

    /// The opaque token for this handle.
    pub fn encode(&self) -> String {
        encode(&self.href, self.etag.as_deref(), self.recurrence_id.as_deref())
    }

    /// Decode a token, or `None` when it is not a well-formed handle (callers
    /// then treat it as a legacy bare UID).
    ///
    /// The href feeds an authenticated GET/PUT/DELETE, so a handle must never
    /// steer credentials at another host: a relative path is fine (it is
    /// joined to the trusted base URL), but anything carrying a URI scheme must
    /// be an http(s) URL whose host is iCloud. The scheme test is deliberately
    /// broader than "contains ://" so a single-slash authority such as
    /// `https:/evil.com/x` cannot slip through.
    pub fn decode(token: &str) -> Option<EventHandle> {
        let (is_v2, b64) = if let Some(rest) = token.strip_prefix(PREFIX_V2) {
            (true, rest)
        } else {
            (false, token.strip_prefix(PREFIX)?)
        };
        let bytes = ENGINE.decode(b64).ok()?;
        let decoded = String::from_utf8_lossy(&bytes);
        let (href, rest) = decoded.split_once('\n')?;
        if href.trim().is_empty() || (has_scheme(href) && !has_icloud_host(href)) {
            return None;
        }
        let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_string());
        if !is_v2 {
            return Some(EventHandle { href: href.to_string(), etag: non_empty(rest), recurrence_id: None });
        }
        let (etag, recurrence_id) = rest.split_once('\n')?;
        if recurrence_id.trim().is_empty() {
            return None;
        }
        Some(EventHandle { href: href.to_string(), etag: non_empty(etag), recurrence_id: Some(recurrence_id.to_string()) })
    }
}

/// Encode a reference: an `evt1_` master handle when `recurrence_id` is
/// `None`, else an `evt2_` occurrence handle.
pub fn encode(href: &str, etag: Option<&str>, recurrence_id: Option<&str>) -> String {
    let href = normalize_icloud_url(href);
    let etag = etag.unwrap_or("");
    match recurrence_id {
        None => format!("{PREFIX}{}", ENGINE.encode(format!("{href}\n{etag}"))),
        Some(recurrence_id) => format!("{PREFIX_V2}{}", ENGINE.encode(format!("{href}\n{etag}\n{recurrence_id}"))),
    }
}

/// True when `token` carries a handle prefix (vs. a legacy bare UID).
pub fn looks_like_handle(token: &str) -> bool {
    token.starts_with(PREFIX) || token.starts_with(PREFIX_V2)
}

fn has_scheme(href: &str) -> bool {
    SCHEME_PREFIX.is_match(href)
}

/// http(s) only, and the host must be `icloud.com` or a subdomain. Host-based
/// on purpose: substring checks accept `icloud.com.attacker.com` and userinfo
/// tricks like `https://caldav.icloud.com@attacker.com`.
fn has_icloud_host(href: &str) -> bool {
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    host == "icloud.com" || host.ends_with(".icloud.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_token(prefix: &str, payload: &str) -> String {
        format!("{prefix}{}", ENGINE.encode(payload))
    }

    #[test]
    fn master_round_trip_and_exact_encoding() {
        let token = encode("/123/calendars/home/abc.ics", Some("etag-1"), None);
        assert!(token.starts_with("evt1_"));
        assert_eq!(token, raw_token(PREFIX, "/123/calendars/home/abc.ics\netag-1"));
        assert!(!token.contains('='));
        let handle = EventHandle::decode(&token).unwrap();
        assert_eq!(handle.href, "/123/calendars/home/abc.ics");
        assert_eq!(handle.etag.as_deref(), Some("etag-1"));
        assert!(!handle.is_occurrence_ref());
        assert_eq!(handle.encode(), token);
    }

    #[test]
    fn occurrence_round_trip() {
        let token = encode("/c/h/x.ics", None, Some("20260818T140000Z"));
        assert!(token.starts_with("evt2_"));
        let handle = EventHandle::decode(&token).unwrap();
        assert_eq!(handle.etag, None);
        assert_eq!(handle.recurrence_id.as_deref(), Some("20260818T140000Z"));
        assert!(handle.is_occurrence_ref());
    }

    #[test]
    fn regional_partitions_normalize() {
        let a = encode("https://p180-caldav.icloud.com:443/1/calendars/home/x.ics", Some("e"), None);
        let b = encode("https://caldav.icloud.com/1/calendars/home/x.ics", Some("e"), None);
        assert_eq!(a, b);
    }

    #[test]
    fn alphabet_is_url_safe() {
        let token = encode("/a?b=c&d=~~~~/\u{00e9}.ics", Some("\"W/xyz\""), None);
        assert!(token[5..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn padding_is_optional_on_decode() {
        let padded = format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE.encode("/x.ics\ne"));
        assert_eq!(EventHandle::decode(&padded).unwrap().href, "/x.ics");
    }

    #[test]
    fn malformed_tokens_decode_to_none() {
        assert_eq!(EventHandle::decode("some-uid@icloud"), None);
        assert_eq!(EventHandle::decode("evt1_!!!not-base64!!!"), None);
        assert_eq!(EventHandle::decode(&raw_token(PREFIX, "no separator")), None);
        assert_eq!(EventHandle::decode(&raw_token(PREFIX, "  \netag")), None);
        assert_eq!(EventHandle::decode(&raw_token(PREFIX_V2, "/x.ics\netag")), None);
        assert_eq!(EventHandle::decode(&raw_token(PREFIX_V2, "/x.ics\netag\n  ")), None);
        assert!(!looks_like_handle("uid"));
        assert!(looks_like_handle("evt2_abc"));
    }

    #[test]
    fn ssrf_hosts_are_rejected() {
        for href in [
            "https://evil.com/x.ics",
            "https://icloud.com.attacker.com/x.ics",
            "https://caldav.icloud.com@attacker.com/x.ics",
            "https://evil.com/?icloud.com",
            "https:/evil.com/x",
            "gopher://caldav.icloud.com/x",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert_eq!(EventHandle::decode(&raw_token(PREFIX, &format!("{href}\ne"))), None, "{href}");
        }
        for href in ["https://caldav.icloud.com/1/x.ics", "https://icloud.com/x.ics", "/relative/x.ics"] {
            assert!(EventHandle::decode(&raw_token(PREFIX, &format!("{href}\ne"))).is_some(), "{href}");
        }
    }
}
