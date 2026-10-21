//! CalDAV model behavior: error factories and classification, calendar/event
//! value semantics, href id extraction, color normalization and credentials.

use icloud_calendar_mcp::caldav::{
    CalDavCalendar, CalDavCredentials, CalDavError, CalDavEvent, extract_calendar_id_from_href, normalize_color,
};

fn calendar(ctag: Option<&str>) -> CalDavCalendar {
    CalDavCalendar {
        id: "home".into(),
        href: "/1234567/calendars/home/".into(),
        url: "https://caldav.icloud.com/1234567/calendars/home/".into(),
        display_name: "Personal".into(),
        color: Some("#FF5733".into()),
        ctag: ctag.map(str::to_string),
        is_read_only: false,
        supported_components: vec!["VEVENT".into()],
    }
}

fn event(etag: Option<&str>, data: &str) -> CalDavEvent {
    CalDavEvent {
        uid: "same-uid".into(),
        href: "/evt.ics".into(),
        url: "https://ex.com/evt.ics".into(),
        etag: etag.map(str::to_string),
        ical_data: data.into(),
    }
}

#[test]
fn error_carries_code_message_and_retryability() {
    let error = CalDavError::new(500, "Server error", true);
    assert_eq!(error.code, 500);
    assert_eq!(error.message, "Server error");
    assert!(error.retryable);
    assert!(!CalDavError::new(500, "x", false).retryable);
}

#[test]
fn error_factories() {
    let cases = [
        (CalDavError::auth(), 401, false),
        (CalDavError::forbidden(), 403, false),
        (CalDavError::not_found("Resource not found"), 404, false),
        (CalDavError::timeout("Request timed out"), 408, true),
        (CalDavError::conflict(), 412, false),
        (CalDavError::payload_too_large("Response too large"), 413, false),
        (CalDavError::rate_limited(), 429, true),
        (CalDavError::network("Connection refused"), 0, true),
        (CalDavError::ssl("Certificate invalid"), 0, false),
        (CalDavError::server(503, "Service Unavailable"), 503, true),
        (CalDavError::bad_request("Missing UID"), 400, false),
    ];
    for (error, code, retryable) in cases {
        assert_eq!(error.code, code, "{error:?}");
        assert_eq!(error.retryable, retryable, "{error:?}");
    }
    assert_eq!(CalDavError::network("Connection refused").message, "Connection refused");
    assert_eq!(CalDavError::bad_request("Missing UID").message, "Missing UID");
}

#[test]
fn error_classification() {
    assert!(CalDavError::new(401, "x", false).is_auth_error());
    assert!(CalDavError::new(403, "x", false).is_auth_error());
    assert!(!CalDavError::new(404, "x", false).is_auth_error());
    assert!(CalDavError::new(404, "x", false).is_not_found());
    for code in [500, 502, 503, 599] {
        assert!(CalDavError::new(code, "x", true).is_server_error());
    }
    assert!(!CalDavError::new(499, "x", false).is_server_error());
    assert!(CalDavError::new(412, "x", false).is_conflict());
    assert!(CalDavError::new(409, "x", false).is_conflict());
    for code in [400, 401, 404, 500] {
        assert!(!CalDavError::new(code, "x", false).is_conflict());
    }
}

#[test]
fn error_display_includes_message_and_code() {
    assert_eq!(CalDavError::conflict().to_string(), "Precondition failed (resource modified) (412)");
}

#[test]
fn calendar_value_semantics() {
    let cal = calendar(Some("abc123"));
    assert_eq!(cal.display_name, "Personal");
    assert_eq!(cal, calendar(Some("abc123")));
    assert_ne!(cal, calendar(Some("different")));
    assert_eq!(calendar(None).ctag, None);
}

#[test]
fn event_has_etag() {
    assert!(event(Some("\"123\""), "...").has_etag());
    assert!(!event(None, "...").has_etag());
    assert!(!event(Some("   "), "...").has_etag());
}

#[test]
fn events_compare_all_fields() {
    assert_ne!(event(Some("\"a\""), "VERSION:1"), event(Some("\"b\""), "VERSION:2"));
    assert_eq!(event(Some("\"a\""), "VERSION:1"), event(Some("\"a\""), "VERSION:1"));
}

#[test]
fn calendar_id_extraction() {
    assert_eq!(extract_calendar_id_from_href("/1234567/calendars/home/"), "home");
    assert_eq!(extract_calendar_id_from_href("/1234567/calendars/work"), "work");
    assert_eq!(extract_calendar_id_from_href("/calendars/"), "calendars");
    assert_eq!(extract_calendar_id_from_href("/home/"), "home");
    assert_eq!(
        extract_calendar_id_from_href("/user/calendars/550e8400-e29b-41d4-a716-446655440000/"),
        "550e8400-e29b-41d4-a716-446655440000"
    );
}

#[test]
fn color_normalization() {
    assert_eq!(normalize_color(None), None);
    assert_eq!(normalize_color(Some("")), None);
    assert_eq!(normalize_color(Some("   ")), None);
    assert_eq!(normalize_color(Some("#FF5733")).as_deref(), Some("#FF5733"));
    assert_eq!(normalize_color(Some("#ffffff")).as_deref(), Some("#ffffff"));
    assert_eq!(normalize_color(Some("FF5733")).as_deref(), Some("#FF5733"));
    assert_eq!(normalize_color(Some("abc123")).as_deref(), Some("#abc123"));
    assert_eq!(normalize_color(Some("#FF5733FF")).as_deref(), Some("#FF5733"));
    assert_eq!(normalize_color(Some("#12345678")).as_deref(), Some("#123456"));
    assert_eq!(normalize_color(Some("red")), None);
    assert_eq!(normalize_color(Some("#GGG")), None);
    assert_eq!(normalize_color(Some("rgb(255,0,0)")), None);
}

#[test]
fn credentials_store_values_and_mask_password() {
    let creds = CalDavCredentials::new("user@icloud.com", "secret-password-123");
    assert_eq!(creds.username, "user@icloud.com");
    assert_eq!(creds.password, "secret-password-123");
    let shown = format!("{creds:?}");
    assert!(shown.contains("user@icloud.com"));
    assert!(shown.contains("***"));
    assert!(!shown.contains("secret-password-123"));
}

#[test]
fn credentials_validity() {
    assert!(CalDavCredentials::new("user@icloud.com", "password").is_valid());
    assert!(!CalDavCredentials::new("", "password").is_valid());
    assert!(!CalDavCredentials::new("user@icloud.com", "").is_valid());
    assert!(!CalDavCredentials::new("   ", "   ").is_valid());
}
