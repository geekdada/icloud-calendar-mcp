//! Validation flows, error formats and sanitization through the real tool
//! handlers. Port of `ToolIntegrationTest.kt`: the Kotlin suite replayed each
//! handler's validator calls by hand; these drive the handlers themselves and
//! check that invalid input never reaches CalDAV.

mod support;

use std::sync::Arc;

use icloud_calendar_mcp::error;
use icloud_calendar_mcp::ics::{NewEvent, build_event, parse_events};
use icloud_calendar_mcp::logging::{McpLogger, NullSink};
use icloud_calendar_mcp::ratelimit::RateLimiter;
use icloud_calendar_mcp::server::CalendarServer;
use icloud_calendar_mcp::validation::sanitize_for_ics;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use support::*;

fn setup() -> (Arc<MockCalDavClient>, CalendarServer) {
    let mock = MockCalDavClient::with_calendars(vec![calendar("cal-123", "Work")]);
    let (server, _) = server(&mock);
    (mock, server)
}

async fn call(server: &CalendarServer, name: &str, args: Value) -> CallToolResult {
    server.call_tool_by_name(name, args.as_object().cloned()).await.unwrap()
}

fn error_body(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true), "expected an error: {}", result_text(result));
    result_json(result)
}

/// The single message, or every message joined in `details`.
fn messages(result: &CallToolResult) -> String {
    let body = error_body(result);
    let error = &body["error"];
    format!("{} {}", error["message"].as_str().unwrap_or_default(), error["details"].as_str().unwrap_or_default())
}

fn no_caldav_calls(mock: &MockCalDavClient) -> bool {
    let s = mock.state();
    s.get_events_called + s.create_event_called + s.update_event_called + s.delete_event_called + s.get_event_called
        == 0
}

// ───────────────────────────── get_events ─────────────────────────────

#[tokio::test]
async fn get_events_validates_all_required_parameters() {
    let (mock, server) = setup();
    let result = call(&server, "get_events", json!({})).await;
    let body = error_body(&result);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert_eq!(body["error"]["message"], "Multiple validation errors");
    let details = body["error"]["details"].as_str().unwrap();
    for field in ["calendar_id", "start_date", "end_date"] {
        assert!(details.contains(field), "{details}");
    }
    assert_eq!(details.matches("is required").count(), 3);
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn get_events_accepts_valid_parameters() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": "cal-123", "start_date": "2025-01-15", "end_date": "2025-01-20"}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let s = mock.state();
    assert_eq!(s.last_calendar_id.as_deref(), Some("cal-123"));
    assert_eq!(s.last_start_date.as_deref(), Some("2025-01-15"));
    assert_eq!(s.last_end_date.as_deref(), Some("2025-01-20"));
}

#[tokio::test]
async fn get_events_rejects_malicious_calendar_id() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": "../../../etc/passwd", "start_date": "2025-01-15", "end_date": "2025-01-20"}),
    )
    .await;
    let body = error_body(&result);
    assert_eq!(body["error"]["message"], "calendar_id contains invalid characters");
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn get_events_rejects_invalid_date_format() {
    let (_, server) = setup();
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": "cal-123", "start_date": "01-15-2025", "end_date": "2025-01-20"}),
    )
    .await;
    assert!(messages(&result).contains("YYYY-MM-DD"), "{}", messages(&result));
}

#[tokio::test]
async fn get_events_rejects_impossible_dates() {
    let (_, server) = setup();
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": "cal-123", "start_date": "2025-02-30", "end_date": "2025-03-01"}),
    )
    .await;
    assert!(messages(&result).contains("not a valid date"), "{}", messages(&result));
}

// ───────────────────────────── create_event ─────────────────────────────

#[tokio::test]
async fn create_event_validates_all_required_parameters() {
    let (mock, server) = setup();
    let result = call(&server, "create_event", json!({})).await;
    let details = error_body(&result)["error"]["details"].as_str().unwrap().to_string();
    assert_eq!(details.split("; ").count(), 4, "calendar_id, title, start_time, end_time: {details}");
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn create_event_validates_time_range() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Meeting", "start_time": "2025-01-15T10:00:00", "end_time": "2025-01-15T09:00:00"}),
    )
    .await;
    let body = error_body(&result);
    assert_eq!(body["error"]["message"], "End time must be after start time");
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn create_event_orders_mixed_naive_and_zoned_endpoints_in_the_write_zone() {
    let (_, server) = setup();
    // 09:00 in New York is 14:00Z, so a 13:00Z end precedes it.
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "M", "start_time": "2025-01-15T09:00:00", "end_time": "2025-01-15T13:00:00Z", "timezone": "America/New_York"}),
    )
    .await;
    assert!(messages(&result).contains("End time must be after start time"), "{}", messages(&result));
}

#[tokio::test]
async fn create_event_validates_optional_fields() {
    let (_, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Meeting", "start_time": "2025-01-15T09:00:00", "end_time": "2025-01-15T10:00:00",
               "location": "a".repeat(501), "description": "Valid description"}),
    )
    .await;
    let body = error_body(&result);
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("location") && message.contains("too long"), "{message}");
}

#[tokio::test]
async fn create_event_validates_timezone_rdates_and_alarms() {
    let (_, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Meeting", "start_time": "2025-01-15T09:00:00", "end_time": "2025-01-15T10:00:00",
               "timezone": "Mars/Olympus", "rdates": ["2025-01-16T10:00:00+02:00"], "alarms": [{"action": "DISPLAY"}]}),
    )
    .await;
    let text = messages(&result);
    assert!(text.contains("timezone is not a recognized IANA timezone"), "{text}");
    assert!(text.contains("rdates[0]"), "{text}");
    assert!(text.contains("alarms[0].trigger is required"), "{text}");
}

#[tokio::test]
async fn create_event_accepts_valid_full_parameters() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Team Meeting", "start_time": "2025-01-15T09:00:00", "end_time": "2025-01-15T10:00:00",
               "location": "Conference Room A", "description": "Weekly sync meeting"}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let ics = mock.state().last_created_ics.clone().unwrap();
    assert!(ics.contains("LOCATION:Conference Room A"));
    assert!(ics.contains("DESCRIPTION:Weekly sync meeting"));
    assert!(ics.contains("DTSTART:20250115T090000Z"), "naive times without timezone are UTC");
}

#[tokio::test]
async fn all_day_create_requires_both_dates() {
    let (_, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Trip", "is_all_day": true, "start_date": "2025-01-15"}),
    )
    .await;
    assert!(messages(&result).contains("end_date is required"), "{}", messages(&result));
}

#[tokio::test]
async fn all_day_create_writes_date_values_and_ignores_times() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Trip", "is_all_day": true, "start_date": "2025-01-15", "end_date": "2025-01-17",
               "start_time": "garbage"}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let ics = mock.state().last_created_ics.clone().unwrap();
    assert!(ics.contains("DTSTART;VALUE=DATE:20250115"));
    assert!(ics.contains("DTEND;VALUE=DATE:20250118"), "inclusive end date becomes an exclusive DTEND");
}

#[tokio::test]
async fn create_event_reports_read_only_and_missing_calendars() {
    let mock = MockCalDavClient::with_calendars(vec![read_only_calendar("shared", "Shared")]);
    let (server, _) = server(&mock);
    let args = |id: &str| json!({"calendar_id": id, "title": "M", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"});
    let read_only = error_body(&call(&server, "create_event", args("shared")).await);
    assert_eq!(read_only["error"]["code"], "FORBIDDEN");
    assert!(read_only["error"]["message"].as_str().unwrap().contains("read-only"));
    let missing = error_body(&call(&server, "create_event", args("nope")).await);
    assert_eq!(missing["error"]["code"], "NOT_FOUND");
    assert_eq!(mock.state().create_event_called, 0);
}

// ───────────────────────────── update_event / delete_event ─────────────────────────────

#[tokio::test]
async fn update_event_requires_event_id() {
    let (_, server) = setup();
    let body = error_body(&call(&server, "update_event", json!({"title": "x"})).await);
    assert_eq!(body["error"]["message"], "event_id is required");
}

#[tokio::test]
async fn update_event_allows_partial_updates() {
    let (mock, server) = setup();
    let created = call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "Old", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z", "location": "Room 9"})).await;
    let handle = result_json(&created)["handle"].as_str().unwrap().to_string();
    let result = call(&server, "update_event", json!({"event_id": handle, "title": "New Title"})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:New Title"));
    assert!(ics.contains("LOCATION:Room 9"), "untouched fields survive");
}

#[tokio::test]
async fn update_event_accepts_a_recently_listed_bare_uid() {
    let (mock, server) = setup();
    let created = call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "Old", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"})).await;
    let uid = result_json(&created)["uid"].as_str().unwrap().to_string();
    let result = call(&server, "update_event", json!({"event_id": uid, "title": "Renamed"})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    assert!(mock.state().last_updated_ics.as_ref().unwrap().contains("SUMMARY:Renamed"));
}

#[tokio::test]
async fn update_event_with_unknown_uid_is_not_found() {
    let (_, server) = setup();
    let body = error_body(&call(&server, "update_event", json!({"event_id": "never-seen", "title": "x"})).await);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn update_event_validates_provided_fields_only() {
    let (_, server) = setup();
    let text = messages(
        &call(
            &server,
            "update_event",
            json!({"event_id": "evt-123", "title": "", "start_time": "tomorrow", "start_date": "2025-13-01", "scope": "everything"}),
        )
        .await,
    );
    assert!(text.contains("title is required"), "{text}");
    assert!(text.contains("start_time must be in ISO 8601 format"), "{text}");
    assert!(text.contains("start_date is not a valid date"), "{text}");
    assert!(text.contains("scope must be one of this_occurrence, this_and_future, all_events"), "{text}");
}

#[tokio::test]
async fn update_event_validates_time_range_when_both_ends_given() {
    let (_, server) = setup();
    let body = error_body(
        &call(
            &server,
            "update_event",
            json!({"event_id": "evt-123", "start_time": "2025-01-15T10:00:00Z", "end_time": "2025-01-15T10:00:00Z"}),
        )
        .await,
    );
    assert_eq!(body["error"]["message"], "End time must be different from start time");
}

#[tokio::test]
async fn delete_event_requires_event_id() {
    let (_, server) = setup();
    let body = error_body(&call(&server, "delete_event", json!({})).await);
    assert_eq!(body["error"]["message"], "event_id is required");
}

#[tokio::test]
async fn delete_event_rejects_malicious_event_id() {
    let (mock, server) = setup();
    let body = error_body(&call(&server, "delete_event", json!({"event_id": "evt; rm -rf /"})).await);
    assert!(body["error"]["message"].as_str().unwrap().contains("invalid"));
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn delete_by_handle_sends_the_handles_etag() {
    let (mock, server) = setup();
    let created = call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "Gone", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"})).await;
    let handle = result_json(&created)["handle"].as_str().unwrap().to_string();
    let result = call(&server, "delete_event", json!({"event_id": handle})).await;
    assert_eq!(result_json(&result)["message"], "Event deleted successfully");
    let s = mock.state();
    assert_eq!(s.delete_etags_seen, [Some("\"new-etag\"".to_string())]);
}

// ───────────────────────────── argument shapes ─────────────────────────────

#[tokio::test]
async fn null_arguments_count_as_absent() {
    let (_, server) = setup();
    let body = error_body(&call(&server, "update_event", json!({"event_id": null, "title": null})).await);
    assert_eq!(body["error"]["message"], "event_id is required");
}

#[tokio::test]
async fn non_scalar_arguments_are_validation_errors() {
    let (mock, server) = setup();
    let body = error_body(
        &call(
            &server,
            "get_events",
            json!({"calendar_id": {"$ne": null}, "start_date": "2025-01-01", "end_date": "2025-01-02"}),
        )
        .await,
    );
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert_eq!(body["error"]["message"], "calendar_id must be a string");
    let rdates = error_body(
        &call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "x", "rdates": "2025-01-01"})).await,
    );
    assert_eq!(rdates["error"]["message"], "rdates must be an array of strings");
    let alarms = error_body(
        &call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "x", "alarms": {"trigger": "-PT5M"}}))
            .await,
    );
    assert_eq!(alarms["error"]["message"], "alarms must be an array of alarm objects");
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn numeric_arguments_read_as_text() {
    let (_, server) = setup();
    // A numeric calendar id is read by its JSON text, like any scalar.
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": 12345, "start_date": "2025-01-01", "end_date": "2025-01-02"}),
    )
    .await;
    let body = error_body(&result);
    assert_eq!(body["error"]["code"], "NOT_FOUND", "the id reached the service as \"12345\"");
}

// ───────────────────────────── error formats ─────────────────────────────

#[test]
fn validation_error_response_is_valid_json() {
    let result = error::validation_error(&["Title is required".into(), "Start date is invalid".into()]);
    assert_eq!(result.is_error, Some(true));
    let text = result_text(&result);
    assert!(text.contains("\"success\": false"));
    assert!(text.contains("\"code\": \"VALIDATION_ERROR\""));
    assert!(text.contains("\"details\""));
    serde_json::from_str::<Value>(&text).unwrap();
}

#[test]
fn authentication_error_response_is_valid_json() {
    let result = error::authentication_error("iCloud credentials not configured");
    let text = result_text(&result);
    assert!(text.contains("\"code\": \"AUTH_ERROR\""));
    assert!(text.contains("iCloud credentials not configured"));
}

#[test]
fn internal_error_response_does_not_leak_details() {
    let text = result_text(&error::internal_error("RuntimeException"));
    assert!(text.contains("internal error"));
    assert!(!text.contains("RuntimeException"));
}

#[tokio::test]
async fn every_tool_reports_missing_credentials() {
    let server = CalendarServer::with_parts(
        None,
        Arc::new(RateLimiter::new(100, 100, 60_000)),
        Arc::new(McpLogger::new(Arc::new(NullSink))),
    );
    let calls = [
        ("list_calendars", json!({})),
        ("get_events", json!({"calendar_id": "c", "start_date": "2025-01-01", "end_date": "2025-01-02"})),
        (
            "create_event",
            json!({"calendar_id": "c", "title": "t", "start_time": "2025-01-01T10:00:00Z", "end_time": "2025-01-01T11:00:00Z"}),
        ),
        ("update_event", json!({"event_id": "e", "title": "t"})),
        ("delete_event", json!({"event_id": "e"})),
    ];
    for (name, args) in calls {
        let body = error_body(&call(&server, name, args).await);
        assert_eq!(body["error"]["code"], "AUTH_ERROR", "{name}");
        assert_eq!(body["error"]["message"], "iCloud credentials not configured", "{name}");
    }
}

#[tokio::test]
async fn validation_runs_before_the_credentials_check() {
    let server = CalendarServer::with_parts(
        None,
        Arc::new(RateLimiter::new(100, 100, 60_000)),
        Arc::new(McpLogger::new(Arc::new(NullSink))),
    );
    let body = error_body(&call(&server, "delete_event", json!({})).await);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn rate_limit_is_checked_first_and_split_by_read_and_write() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("cal-123", "Work")]);
    let server = CalendarServer::with_parts(
        Some(service(&mock)),
        Arc::new(RateLimiter::new(1, 1, 60_000)),
        Arc::new(McpLogger::new(Arc::new(NullSink))),
    );
    assert_ne!(call(&server, "list_calendars", json!({})).await.is_error, Some(true));
    let blocked = error_body(&call(&server, "get_events", json!({})).await);
    assert_eq!(blocked["error"]["code"], "RATE_LIMITED", "rate limit wins over validation");
    assert!(blocked["error"]["message"].as_str().unwrap().contains("Please retry in"));
    // The write budget is separate.
    let write = error_body(&call(&server, "delete_event", json!({})).await);
    assert_eq!(write["error"]["code"], "VALIDATION_ERROR");
    let write_blocked = error_body(&call(&server, "update_event", json!({})).await);
    assert_eq!(write_blocked["error"]["code"], "RATE_LIMITED");
}

// ───────────────────────────── ICS sanitization ─────────────────────────────

#[tokio::test]
async fn create_event_neutralizes_crlf_injection() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "cal-123", "title": "Meeting\nATTENDEE:mailto:evil@hacker.com", "start_time": "2025-01-15T09:00:00Z",
               "end_time": "2025-01-15T10:00:00Z", "location": "Room A\r\nORGANIZER:mailto:evil@hacker.com"}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let ics = mock.state().last_created_ics.clone().unwrap();
    assert!(!ics.contains("\r\nATTENDEE"), "{ics}");
    assert!(!ics.contains("\r\nORGANIZER"), "{ics}");
    assert!(ics.contains("SUMMARY:Meeting ATTENDEE:mailto:evil@hacker.com"));
}

#[test]
fn sanitize_keeps_semicolons_for_the_generator_to_escape() {
    let safe = sanitize_for_ics("Room A;ORGANIZER:mailto:evil@hacker.com");
    assert!(safe.contains(';'));
    assert!(!safe.contains('\n'));
}

#[test]
fn special_chars_survive_the_write_read_round_trip_without_double_escaping() {
    let raw_title = "Lunch, dinner; and a path C:\\Users\\Name";
    let raw_description = "warm intro, framed; not generic. path\\here";
    let ics = build_event(&NewEvent {
        uid: Some("roundtrip@test".into()),
        summary: sanitize_for_ics(raw_title),
        start_time: Some("2026-08-15T09:00:00".into()),
        end_time: Some("2026-08-15T10:00:00".into()),
        timezone: Some("UTC".into()),
        description: Some(sanitize_for_ics(raw_description)),
        ..NewEvent::default()
    })
    .unwrap();
    let parsed = parse_events(&ics);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].summary, raw_title);
    assert_eq!(parsed[0].description.as_deref(), Some(raw_description));
}

#[tokio::test]
async fn special_chars_survive_create_then_get_events() {
    let (mock, server) = setup();
    let title = "Lunch, dinner; and C:\\path";
    call(&server, "create_event", json!({"calendar_id": "cal-123", "title": title, "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"})).await;
    let created = mock.state().registered_events.values().next().cloned().unwrap();
    mock.state().events_response = vec![created];
    let listed = call(
        &server,
        "get_events",
        json!({"calendar_id": "cal-123", "start_date": "2025-01-15", "end_date": "2025-01-15"}),
    )
    .await;
    assert_eq!(result_json(&listed)["events"][0]["summary"], title);
}

#[tokio::test]
async fn full_validation_flow_catches_all_security_issues() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": "../../../etc/passwd", "title": "Meeting", "start_time": "invalid", "end_time": "2025-01-15T10:00:00",
               "location": "Room\nX-HEADER:injection"}),
    )
    .await;
    let details = error_body(&result)["error"]["details"].as_str().unwrap().to_string();
    assert!(details.contains("calendar_id contains invalid characters"), "{details}");
    assert!(details.contains("start_time"), "{details}");
    assert!(!details.contains("location"), "location is valid input (sanitized on write): {details}");
    assert!(no_caldav_calls(&mock));
}

#[tokio::test]
async fn successful_writes_are_audited_without_leaking_secrets() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("cal-123", "Work")]);
    let (server, sink) = server(&mock);
    call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "M", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"})).await;
    let records = sink.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data["message"], "Event created");
    assert_eq!(records[0].data["tool"], "create_event");
    assert_eq!(records[0].data["calendar"], "cal-123");
    assert_eq!(records[0].logger, "icloud-calendar-mcp");

    mock.state().error_to_return = Some(icloud_calendar_mcp::caldav::CalDavError::forbidden());
    call(&server, "delete_event", json!({"event_id": "someone@example.com"})).await;
    let records = sink.records();
    let failure = records.last().unwrap();
    assert_eq!(failure.data["message"], "Event deletion failed");
    assert_eq!(failure.data["code"], 404, "an unknown bare UID is a 404 before any CalDAV call");
    assert!(!failure.data.to_string().contains("someone@example.com"), "emails are redacted: {}", failure.data);
}

#[tokio::test]
async fn stale_etag_after_one_retry_is_reported_as_conflict() {
    let (mock, server) = setup();
    let created = call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "Old", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"})).await;
    let handle = result_json(&created)["handle"].as_str().unwrap().to_string();
    mock.state().fail_412_updates_remaining = 2;
    let body = error_body(&call(&server, "update_event", json!({"event_id": handle, "title": "New"})).await);
    assert_eq!(body["error"]["code"], "CONFLICT", "callers re-list and retry on a conflict");
    assert_eq!(mock.state().update_event_called, 2, "exactly one refetch-and-retry");
}

#[tokio::test]
async fn single_stale_etag_is_absorbed_by_the_retry() {
    let (mock, server) = setup();
    let created = call(&server, "create_event", json!({"calendar_id": "cal-123", "title": "Old", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T10:00:00Z"})).await;
    let handle = result_json(&created)["handle"].as_str().unwrap().to_string();
    mock.state().fail_412_deletes_remaining = 1;
    let result = call(&server, "delete_event", json!({"event_id": handle})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    assert_eq!(mock.state().delete_event_called, 2);
}
