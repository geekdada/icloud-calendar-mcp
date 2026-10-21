//! Port of EventHandleTest: opaque event handle encoding, decoding and the SSRF guard.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use icloud_calendar_mcp::service::handle::{EventHandle, PREFIX, PREFIX_V2, encode, looks_like_handle};

fn forge(prefix: &str, payload: &str) -> String {
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(payload))
}

fn forge_handle(href: &str) -> String {
    forge(PREFIX, &format!("{href}\n\"e\""))
}

fn is_url_safe(body: &str) -> bool {
    body.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[test]
fn round_trips_href_and_etag() {
    let decoded = EventHandle::decode(&encode("/cal/abc.ics", Some("\"etag-1\""), None)).unwrap();
    assert_eq!(decoded.href, "/cal/abc.ics");
    assert_eq!(decoded.etag.as_deref(), Some("\"etag-1\""));
}

#[test]
fn round_trips_null_etag() {
    let decoded = EventHandle::decode(&encode("/cal/abc.ics", None, None)).unwrap();
    assert_eq!(decoded.href, "/cal/abc.ics");
    assert_eq!(decoded.etag, None, "empty etag decodes back to None");
}

#[test]
fn encoded_token_carries_version_prefix() {
    assert!(encode("/cal/abc.ics", Some("\"e\""), None).starts_with(PREFIX));
}

#[test]
fn encoded_token_uses_url_safe_alphabet() {
    let handle = encode("https://p180-caldav.icloud.com:443/x/y z.ics", Some("\"e/+=\""), None);
    let body = handle.strip_prefix(PREFIX).unwrap();
    assert!(is_url_safe(body), "{body}");
}

#[test]
fn normalizes_regional_href_on_encode() {
    let handle = encode("https://p42-caldav.icloud.com/12345/calendars/home/e.ics", Some("\"e\""), None);
    assert_eq!(EventHandle::decode(&handle).unwrap().href, "https://caldav.icloud.com/12345/calendars/home/e.ics");
}

#[test]
fn looks_like_handle_distinguishes_bare_uids() {
    assert!(looks_like_handle(&encode("/x.ics", None, None)));
    assert!(!looks_like_handle("plain-uid-1234"));
    assert!(!looks_like_handle(""));
}

#[test]
fn decode_returns_none_for_bare_uid_and_bad_base64() {
    assert_eq!(EventHandle::decode("plain-uid-1234"), None);
    assert_eq!(EventHandle::decode(&format!("{PREFIX}!!!not base64!!!")), None);
}

#[test]
fn decode_returns_none_without_separator_or_with_blank_href() {
    assert_eq!(EventHandle::decode(&forge(PREFIX, "no-newline-here")), None);
    assert_eq!(EventHandle::decode(&forge(PREFIX, "\n\"etag\"")), None);
}

#[test]
fn decode_rejects_off_host_absolute_hrefs() {
    assert_eq!(EventHandle::decode(&forge_handle("https://evil.com/steal.ics")), None);
    assert_eq!(EventHandle::decode(&forge_handle("http://169.254.169.254/latest/meta-data")), None);
}

#[test]
fn decode_rejects_suffix_and_substring_spoofed_hosts() {
    assert_eq!(EventHandle::decode(&forge_handle("https://icloud.com.attacker.com/steal.ics")), None);
    assert_eq!(EventHandle::decode(&forge_handle("https://evil.com/path?x=icloud.com")), None);
    assert_eq!(EventHandle::decode(&forge_handle("https://evilicloud.com/x.ics")), None);
}

#[test]
fn decode_rejects_userinfo_spoofed_authority() {
    assert_eq!(EventHandle::decode(&forge_handle("https://caldav.icloud.com@attacker.com/x.ics")), None);
}

#[test]
fn decode_rejects_non_http_schemes() {
    assert_eq!(EventHandle::decode(&forge_handle("file:///etc/passwd")), None);
    assert_eq!(EventHandle::decode(&forge_handle("gopher://icloud.com/x")), None);
}

#[test]
fn decode_rejects_single_slash_and_schemeless_authorities() {
    assert_eq!(EventHandle::decode(&forge_handle("https:/evil.com/steal.ics")), None);
    assert_eq!(EventHandle::decode(&forge_handle("https:evil.com/steal.ics")), None);
}

#[test]
fn decode_accepts_protocol_relative_href() {
    // No scheme, so it is joined to the trusted base URL, keeping the iCloud host.
    assert_eq!(EventHandle::decode(&forge_handle("//evil.com/x.ics")).unwrap().href, "//evil.com/x.ics");
}

#[test]
fn decode_accepts_relative_canonical_and_regional_hrefs() {
    for href in [
        "/123/calendars/home/e.ics",
        "https://caldav.icloud.com/123/calendars/home/e.ics",
        "https://p180-caldav.icloud.com/123/e.ics",
    ] {
        assert_eq!(EventHandle::decode(&forge_handle(href)).unwrap().href, href);
    }
}

#[test]
fn round_trips_unsafe_and_unicode_href() {
    let weird = "/cal/a b+c/=d/é-events.ics";
    let decoded = EventHandle::decode(&encode(weird, Some("\"e/+=\""), None)).unwrap();
    assert_eq!(decoded.href, weird);
    assert_eq!(decoded.etag.as_deref(), Some("\"e/+=\""));
}

#[test]
fn round_trips_weak_etag() {
    let decoded = EventHandle::decode(&encode("/cal/x.ics", Some("W/\"abc\""), None)).unwrap();
    assert_eq!(decoded.etag.as_deref(), Some("W/\"abc\""));
}

#[test]
fn handle_is_stable_across_partitions() {
    assert_eq!(
        encode("https://p1-caldav.icloud.com/9/e.ics", Some("\"e\""), None),
        encode("https://p77-caldav.icloud.com/9/e.ics", Some("\"e\""), None)
    );
}

#[test]
fn decode_returns_none_for_bare_prefix_and_blank_input() {
    assert_eq!(EventHandle::decode(PREFIX), None);
    assert_eq!(EventHandle::decode(""), None);
    assert_eq!(EventHandle::decode("   "), None);
}

#[test]
fn round_trips_occurrence_handle() {
    let handle = encode("/cal/series.ics", Some("\"etag-1\""), Some("20260818T140000Z"));
    assert!(handle.starts_with(PREFIX_V2));
    let decoded = EventHandle::decode(&handle).unwrap();
    assert_eq!(decoded.href, "/cal/series.ics");
    assert_eq!(decoded.etag.as_deref(), Some("\"etag-1\""));
    assert_eq!(decoded.recurrence_id.as_deref(), Some("20260818T140000Z"));
    assert!(decoded.is_occurrence_ref());
}

#[test]
fn round_trips_all_day_occurrence_and_null_etag() {
    let all_day = EventHandle::decode(&encode("/cal/series.ics", Some("\"e\""), Some("20260818"))).unwrap();
    assert_eq!(all_day.recurrence_id.as_deref(), Some("20260818"));
    let no_etag = EventHandle::decode(&encode("/cal/series.ics", None, Some("20260818T140000Z"))).unwrap();
    assert_eq!(no_etag.etag, None);
    assert_eq!(no_etag.recurrence_id.as_deref(), Some("20260818T140000Z"));
}

#[test]
fn master_handle_has_no_recurrence_id() {
    let decoded = EventHandle::decode(&encode("/cal/x.ics", Some("\"e\""), None)).unwrap();
    assert_eq!(decoded.recurrence_id, None);
    assert!(!decoded.is_occurrence_ref());
    let struct_form = EventHandle { href: "/cal/x.ics".into(), etag: Some("\"e\"".into()), recurrence_id: None };
    assert_eq!(struct_form.encode(), encode("/cal/x.ics", Some("\"e\""), None));
}

#[test]
fn occurrence_handle_body_is_url_safe_and_recognized() {
    let handle = encode("https://p180-caldav.icloud.com/x/y z.ics", Some("\"e/+=\""), Some("20260818T140000Z"));
    assert!(is_url_safe(handle.strip_prefix(PREFIX_V2).unwrap()));
    assert!(looks_like_handle(&handle));
}

#[test]
fn occurrence_handle_keeps_ssrf_guard_and_normalization() {
    assert_eq!(EventHandle::decode(&forge(PREFIX_V2, "https://evil.com/steal.ics\n\"e\"\n20260818T140000Z")), None);
    assert_eq!(
        encode("https://p1-caldav.icloud.com/9/e.ics", Some("\"e\""), Some("20260818T140000Z")),
        encode("https://p77-caldav.icloud.com/9/e.ics", Some("\"e\""), Some("20260818T140000Z"))
    );
}

#[test]
fn malformed_occurrence_handles_decode_to_none() {
    assert_eq!(EventHandle::decode(&forge(PREFIX_V2, "/cal/x.ics\n\"e\"")), None);
    assert_eq!(EventHandle::decode(&forge(PREFIX_V2, "/cal/x.ics\n\"e\"\n")), None);
}
