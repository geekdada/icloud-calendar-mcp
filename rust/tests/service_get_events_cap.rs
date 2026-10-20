//! Port of GetEventsCapTest: the get_events result cap, the expansion work-bound
//! surfacing as a 413, and the client-side 2 MB 413 passing through unchanged.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use icloud_calendar_mcp::caldav::{CalDavError, CalDavEvent};
use icloud_calendar_mcp::error;
use icloud_calendar_mcp::service::{CalendarService, MAX_RETURNED_EVENTS};
use regex::Regex;
use support::*;

const CALENDAR_ID: &str = "cal-1";

fn setup() -> (Arc<MockCalDavClient>, CalendarService) {
    let mock = MockCalDavClient::with_calendars(vec![calendar(CALENDAR_ID, "Private")]);
    let service = CalendarService::new(mock.clone());
    (mock, service)
}

fn resource(uid: &str, body: &str) -> CalDavEvent {
    CalDavEvent {
        uid: uid.into(),
        href: format!("/caldav/cal-1/{uid}.ics"),
        url: format!("https://caldav.icloud.com/caldav/cal-1/{uid}.ics"),
        etag: Some(format!("\"etag-{uid}\"")),
        ical_data: format!("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\n{body}END:VCALENDAR"),
    }
}

/// `n` distinct non-recurring timed events, all inside the queried window.
fn serve_non_recurring(mock: &MockCalDavClient, n: usize) {
    mock.state().events_response = (0..n)
        .map(|i| {
            let uid = format!("uid-{i}");
            resource(
                &uid,
                &format!("BEGIN:VEVENT\nUID:{uid}\nDTSTART:20260601T100000Z\nDTEND:20260601T110000Z\nSUMMARY:Event {i}\nEND:VEVENT\n"),
            )
        })
        .collect();
}

/// One recurring master with `rrule`, starting inside the window.
fn serve_series(mock: &MockCalDavClient, rrule: &str) {
    mock.state().events_response = vec![resource(
        "series-1",
        &format!(
            "BEGIN:VEVENT\nUID:series-1\nDTSTART:20260601T000000Z\nDTEND:20260601T010000Z\nSUMMARY:Dense series\nRRULE:{rrule}\nEND:VEVENT\n"
        ),
    )];
}

#[tokio::test]
async fn exactly_1000_events_is_success() {
    let (mock, service) = setup();
    serve_non_recurring(&mock, MAX_RETURNED_EVENTS);
    let events = service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-01").await.unwrap();
    assert_eq!(events.len(), 1000);
}

#[tokio::test]
async fn events_over_the_cap_are_413() {
    let (mock, service) = setup();
    serve_non_recurring(&mock, MAX_RETURNED_EVENTS + 1);
    let err = service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-01").await.unwrap_err();
    assert_eq!(err.code, 413);
}

#[tokio::test]
async fn empty_result_is_success() {
    let (mock, service) = setup();
    serve_non_recurring(&mock, 0);
    assert!(service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-01").await.unwrap().is_empty());
}

#[tokio::test]
async fn dense_series_over_count_cap_is_413() {
    // ~1,440 minutely occurrences: over the 1,000 count cap, under the 10,000 expansion bound.
    let (mock, service) = setup();
    serve_series(&mock, "FREQ=MINUTELY");
    let err = service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-02").await.unwrap_err();
    assert_eq!(err.code, 413);
    assert!(err.message.contains("Too many events"), "{}", err.message);
}

#[tokio::test]
async fn series_tripping_the_expansion_bound_is_413() {
    // ~172,800 secondly occurrences over two days: past the expander's bound.
    let (mock, service) = setup();
    serve_series(&mock, "FREQ=SECONDLY");
    let started = Instant::now();
    let err = service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-03").await.unwrap_err();
    assert_eq!(err.code, 413);
    assert!(err.message.contains("expands to too many occurrences"), "{}", err.message);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn client_side_payload_too_large_passes_through() {
    let (mock, service) = setup();
    mock.state().error_to_return = Some(CalDavError::payload_too_large("Response too large (exceeds 2MB limit)"));
    let err = service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-02").await.unwrap_err();
    assert_eq!(err.code, 413);
    assert!(err.message.contains("2MB"), "{}", err.message);
}

fn error_code_of(result: &rmcp::model::CallToolResult) -> String {
    let text = result_text(result);
    let re = Regex::new(r#""code":\s*"([^"]+)""#).unwrap();
    re.captures(&text).map(|c| c[1].to_string()).unwrap_or_else(|| panic!("no error code in {text}"))
}

#[test]
fn over_count_and_payload_too_large_share_the_mcp_shape() {
    let over_count = error::service_error(413, "Too many events in range (exceeds 1000). Narrow the date range.");
    let too_large = error::service_error(413, "Response too large (exceeds 2MB limit)");
    assert_eq!(over_count.is_error, Some(true));
    assert_eq!(too_large.is_error, over_count.is_error);
    assert!(over_count.structured_content.is_none());
    assert!(too_large.structured_content.is_none());
    assert_eq!(error_code_of(&over_count), "PAYLOAD_TOO_LARGE");
    assert_eq!(error_code_of(&too_large), error_code_of(&over_count));
}
