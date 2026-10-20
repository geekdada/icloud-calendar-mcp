//! Port of WritePathExpansionCapTest: a this-and-future edit or delete on a
//! pathologically dense series must surface as a 413, not escape as an error
//! of another kind or hang.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use icloud_calendar_mcp::caldav::CalDavEvent;
use icloud_calendar_mcp::ics::EventPatch;
use icloud_calendar_mcp::scope::EventScope;
use icloud_calendar_mcp::service::{CalendarService, handle};
use support::*;

/// A live occurrence far enough after DTSTART that the truncation expansion trips the bound.
const FAR_OCCURRENCE: &str = "20260101T000000Z";

/// An open-ended hourly master from 2020: tens of thousands of occurrences before 2026.
fn register_hourly_series(mock: &MockCalDavClient) -> CalDavEvent {
    let event = CalDavEvent {
        uid: "dense-series".into(),
        href: "/cal/dense-series.ics".into(),
        url: "https://caldav.icloud.com/cal/dense-series.ics".into(),
        etag: Some("\"etag-dense\"".into()),
        ical_data: "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\nBEGIN:VEVENT\nUID:dense-series\nDTSTART:20200101T000000Z\nDTEND:20200101T010000Z\nSUMMARY:Dense hourly series\nRRULE:FREQ=HOURLY\nEND:VEVENT\nEND:VCALENDAR".into(),
    };
    mock.state().registered_events.insert(event.uid.clone(), event.clone());
    event
}

fn setup() -> (Arc<MockCalDavClient>, CalendarService, String) {
    let mock = MockCalDavClient::new();
    let event = register_hourly_series(&mock);
    let token = handle::encode(&event.href, event.etag.as_deref(), Some(FAR_OCCURRENCE));
    let service = CalendarService::new(mock.clone());
    (mock, service, token)
}

#[tokio::test]
async fn this_and_future_delete_on_dense_series_is_413() {
    let (mock, service, token) = setup();
    let started = Instant::now();
    let err = service.delete_event(&token, Some(EventScope::ThisAndFuture)).await.unwrap_err();
    assert_eq!(err.code, 413);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(mock.state().update_event_called, 0, "nothing is written");
}

#[tokio::test]
async fn this_and_future_edit_on_dense_series_is_413() {
    let (mock, service, token) = setup();
    let patch = EventPatch { summary: Some("Edited from here on".into()), ..EventPatch::default() };
    let started = Instant::now();
    let err = service.update_event(&token, patch, Some(EventScope::ThisAndFuture)).await.unwrap_err();
    assert_eq!(err.code, 413);
    assert!(started.elapsed() < Duration::from_secs(10));
    let state = mock.state();
    assert_eq!((state.create_event_called, state.update_event_called), (0, 0), "nothing is written");
}
