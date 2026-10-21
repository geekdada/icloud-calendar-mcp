//! OWASP MCP Top 10 risks not covered by `security_adversarial.rs`: tool
//! manipulation (MCP02), embedded resource injection (MCP03), credential
//! storage (MCP04), resource consumption (MCP08) and isolation (MCP09).
//! Port of `OwaspMcpSecurityTest.kt`.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use icloud_calendar_mcp::caldav::CalDavCredentials;
use icloud_calendar_mcp::credentials::{self, Credentials};
use icloud_calendar_mcp::error::{self, ErrorCode};
use icloud_calendar_mcp::ics::{NewEvent, build_event, parse_events};
use icloud_calendar_mcp::ratelimit::RateLimiter;
use icloud_calendar_mcp::service::CalendarService;
use icloud_calendar_mcp::validation::{validate_calendar_id, validate_title};
use serde_json::json;
use support::*;

fn invalid_id(id: &str) -> bool {
    validate_calendar_id(Some(id), "calendar_id").is_err()
}

fn all_day(summary: &str) -> NewEvent {
    NewEvent { summary: summary.into(), start_date: Some("2025-01-15".into()), is_all_day: true, ..NewEvent::default() }
}

/// Fail if `f` takes longer than `limit` (the Kotlin suite used @Timeout).
fn within<T>(limit: Duration, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = f();
    assert!(start.elapsed() < limit, "took {:?}", start.elapsed());
    value
}

// ───────────────────── MCP02: tool manipulation ─────────────────────

#[test]
fn mcp02_json_injection_in_parameters_is_rejected() {
    assert!(invalid_id(r#"normal-id", "admin": true, "bypass": "auth"#));
}

#[test]
fn mcp02_json_like_title_is_valid_and_escaped() {
    let injection = r#"{"nested": {"privilege": "admin"}}"#;
    assert!(validate_title(Some(injection), "title").is_ok());
    let ics = build_event(&all_day(injection)).unwrap();
    assert!(ics.contains(r#"SUMMARY:{"nested": {"privilege": "admin"}}"#));
    assert_eq!(parse_events(&ics)[0].summary, injection);
}

#[test]
fn mcp02_prototype_pollution_and_control_characters_are_rejected() {
    assert!(invalid_id("__proto__[admin]=1"));
    assert!(invalid_id("test\u{0}\u{1}\u{2}\u{3}\u{4}id"));
}

#[test]
fn mcp02_type_coercion_strings_are_plain_ids() {
    assert!(!invalid_id("12345"));
    assert!(!invalid_id("true"));
}

#[tokio::test]
async fn mcp02_extra_arguments_are_ignored() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let (server, _) = server(&mock);
    let result = server
        .call_tool_by_name(
            "get_events",
            json!({"calendar_id": "home", "start_date": "2025-01-01", "end_date": "2025-01-02", "admin": true, "href": "https://evil.com/x"})
                .as_object()
                .cloned(),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    assert_eq!(mock.state().last_calendar_id.as_deref(), Some("home"));
}

#[tokio::test]
async fn mcp02_forged_handle_cannot_redirect_writes() {
    use base64::Engine;
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let (server, _) = server(&mock);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("https://evil.com/steal.ics\netag");
    let forged = format!("evt1_{payload}");
    let result =
        server.call_tool_by_name("delete_event", json!({"event_id": forged}).as_object().cloned()).await.unwrap();
    assert_eq!(result.is_error, Some(true));
    let s = mock.state();
    assert_eq!(s.delete_event_called + s.get_event_called, 0, "a non-iCloud href never reaches CalDAV");
}

// ───────────────────── MCP03: embedded resource injection ─────────────────────

#[test]
fn mcp03_dangerous_uris_are_rejected() {
    for uri in [
        "file:///etc/passwd",
        "data:text/html,<script>alert('xss')</script>",
        "javascript:alert(document.domain)",
        "http://169.254.169.254/latest/meta-data/",
        "http://localhost:8080/admin",
        "http://127.0.0.1:22/ssh",
    ] {
        assert!(invalid_id(uri), "{uri}");
    }
}

#[test]
fn mcp03_x_properties_are_inert_data() {
    let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:poison@attacker.com\nSUMMARY:Normal Event\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nX-MALICIOUS:true\nX-INJECT:../../../sensitive-data\nEND:VEVENT\nEND:VCALENDAR";
    let events = parse_events(ics);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Normal Event");
}

#[tokio::test]
async fn mcp03_only_the_calendar_resource_is_readable() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let (server, _) = server(&mock);
    for uri in
        ["file:///etc/passwd", "calendar://calendars/../secrets", "calendar://calendars/home", "http://localhost/"]
    {
        assert!(server.read_resource_uri(uri).await.is_err(), "{uri}");
    }
}

// ───────────────────── MCP04: credential storage ─────────────────────

#[test]
fn mcp04_credentials_are_masked_when_formatted() {
    let password = "super-secret-app-password-1234";
    let creds = Credentials::new("user@icloud.com", password);
    let message = format!("Failed to connect with {creds}");
    assert!(!message.contains(password));
    assert!(!message.contains("super-secret"));
    let debug = format!("{creds:?}");
    assert!(!debug.contains("secret"));
    assert!(debug.contains("****"));
}

#[test]
fn mcp04_mask_handles_edge_cases() {
    assert_eq!(credentials::mask(Some(""), 3), "***");
    assert_eq!(credentials::mask(None, 3), "***");
    let masked = credentials::mask(Some(&"a".repeat(1000)), 3);
    assert_eq!(masked, "aaa***");
    assert!(masked.len() <= 10);
}

#[test]
fn mcp04_error_sanitizer_masks_varied_credential_formats() {
    for input in ["password=secret123", "password: secret123", "PASSWORD=SECRET123", "pass=mypass", "pwd=mypwd"] {
        let text = result_text(&error::create_error_result(ErrorCode::InternalError, input, None));
        assert!(!text.to_lowercase().contains("secret123"), "{input}: {text}");
        assert!(!text.contains("mypass"), "{input}: {text}");
        assert!(!text.contains("mypwd"), "{input}: {text}");
    }
}

#[test]
fn mcp04_caldav_credentials_debug_masks_password() {
    let shown = format!("{:?}", CalDavCredentials::new("user@example.com", "xxxx-yyyy-zzzz-1234"));
    assert!(!shown.contains("xxxx-yyyy-zzzz-1234"));
    assert!(shown.contains("***"));
}

#[test]
fn mcp04_credential_errors_name_the_rule_not_the_value() {
    let lookup = |key: &str| match key {
        "ICLOUD_USERNAME" => Some("not-an-email".to_string()),
        "ICLOUD_PASSWORD" => Some("short-secret".to_string()),
        _ => None,
    };
    let error = credentials::load_with(lookup).unwrap_err();
    assert!(!error.0.contains("short-secret"));
    assert!(!error.0.contains("not-an-email"));
}

// ───────────────────── MCP08: resource consumption ─────────────────────

#[tokio::test]
async fn mcp08_cache_stays_bounded_by_expiry() {
    let mock = MockCalDavClient::new();
    mock.state().events_response = (1..=100)
        .map(|i| {
            caldav_event(
                &format!("uid-{i}"),
                &vcalendar(&timed_vevent(
                    &format!("uid-{i}"),
                    &format!("Event {i}"),
                    "20250115T090000Z",
                    "20250115T100000Z",
                )),
            )
        })
        .collect();
    let service = CalendarService::with_cache(mock.clone(), Duration::from_secs(60), 10);
    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert!(service.cache_size() <= 100, "cache grew beyond the events seen: {}", service.cache_size());
}

#[tokio::test]
async fn mcp08_expired_entries_are_evicted_at_capacity() {
    let mock = MockCalDavClient::new();
    let events = |prefix: &str| {
        (1..=10)
            .map(|i| {
                let uid = format!("{prefix}-{i}");
                caldav_event(&uid, &vcalendar(&timed_vevent(&uid, "E", "20250115T090000Z", "20250115T100000Z")))
            })
            .collect::<Vec<_>>()
    };
    let service = CalendarService::with_cache(mock.clone(), Duration::from_millis(20), 10);
    mock.state().events_response = events("old");
    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert_eq!(service.cache_size(), 10);
    tokio::time::sleep(Duration::from_millis(40)).await;
    mock.state().events_response = events("new");
    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert!(service.cache_size() <= 11, "expired entries were evicted: {}", service.cache_size());
}

#[test]
fn mcp08_large_ics_parses_quickly() {
    let ics = format!(
        "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:large@test.com\nSUMMARY:Large Event\nDESCRIPTION:{}\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nEND:VEVENT\nEND:VCALENDAR",
        "X".repeat(1_000_000)
    );
    let events = within(Duration::from_secs(5), || parse_events(&ics));
    assert_eq!(events.len(), 1);
}

#[test]
fn mcp08_many_events_parse_quickly() {
    let mut ics = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
    for i in 0..1000 {
        ics.push_str(&format!(
            "BEGIN:VEVENT\nUID:event-{i}@test.com\nSUMMARY:Event {i}\nDTSTART:20250115T{:02}0000Z\nDTEND:20250115T{:02}0000Z\nEND:VEVENT\n",
            i % 24,
            (i + 1) % 24
        ));
    }
    ics.push_str("END:VCALENDAR\n");
    let events = within(Duration::from_secs(5), || parse_events(&ics));
    assert_eq!(events.len(), 1000);
}

#[test]
fn mcp08_deeply_nested_components_are_handled() {
    let mut ics = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
    ics.push_str(&"BEGIN:VEVENT\n".repeat(100));
    ics.push_str("UID:nested@test.com\nSUMMARY:Nested Test\nDTSTART:20250115T100000Z\n");
    ics.push_str(&"END:VEVENT\n".repeat(100));
    ics.push_str("END:VCALENDAR\n");
    within(Duration::from_secs(5), || parse_events(&ics));
}

#[test]
fn mcp08_rate_limiter_caps_at_limit() {
    let limiter = RateLimiter::new(10, 5, 1000);
    assert_eq!((0..1000).filter(|_| limiter.try_acquire_read()).count(), 10);
}

#[test]
fn mcp08_long_field_values_build_quickly() {
    let long = "A".repeat(10_000_000);
    let ics = within(Duration::from_secs(5), || {
        build_event(&NewEvent { description: Some(long[..5000].to_string()), ..all_day(&long[..500]) }).unwrap()
    });
    assert!(ics.contains("SUMMARY:"));
}

#[tokio::test]
async fn mcp08_oversized_tool_input_is_rejected_before_work() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let (server, _) = server(&mock);
    let result = server
        .call_tool_by_name(
            "create_event",
            json!({"calendar_id": "home", "title": "t", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z",
                   "description": "D".repeat(5001), "rdates": vec!["2025-01-16"; 367],
                   "alarms": vec![json!({"trigger": "-PT5M"}); 9]})
            .as_object()
            .cloned(),
        )
        .await
        .unwrap();
    let details = result_json(&result)["error"]["details"].as_str().unwrap().to_string();
    assert!(details.contains("description is too long"), "{details}");
    assert!(details.contains("rdates has too many entries"), "{details}");
    assert!(details.contains("alarms has too many entries"), "{details}");
    assert_eq!(mock.state().create_event_called, 0);
}

// ───────────────────── MCP09: isolation ─────────────────────

#[test]
fn mcp09_path_traversal_variants_are_rejected() {
    for attempt in [
        "../../../etc/passwd",
        "..\\..\\..\\windows\\system32",
        "....//....//....//etc/passwd",
        "%2e%2e%2f%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "..%252f..%252f..%252fetc%252fpasswd",
        "..%c0%af..%c0%af..%c0%afetc%c0%afpasswd",
        "/calendars/../../etc/passwd",
    ] {
        assert!(invalid_id(attempt), "{attempt}");
    }
}

#[tokio::test]
async fn mcp09_service_caches_are_isolated() {
    let mock = MockCalDavClient::new();
    mock.state().events_response = vec![caldav_event(
        "isolated-event",
        &vcalendar(&timed_vevent("isolated-event", "Isolated Event", "20250115T090000Z", "20250115T100000Z")),
    )];
    let first = CalendarService::new(mock.clone());
    let second = CalendarService::new(mock.clone());
    first.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert_eq!(second.cache_size(), 0);
    let _ = Arc::new(first);
}

#[test]
fn mcp09_env_glob_and_regex_injection_are_rejected() {
    for pattern in [
        "${HOME}/sensitive",
        "*",
        "**",
        "*.ics",
        "[a-z]*",
        "{a,b,c}",
        "?(pattern)",
        "(?=malicious)",
        ".*",
        ".+",
        "[^a-z]",
        "\\d+",
        "^start",
        "end$",
    ] {
        assert!(invalid_id(pattern), "{pattern}");
    }
}
