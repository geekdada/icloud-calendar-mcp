//! Port of CalendarServiceTest: the orchestration layer over a mocked CalDAV client.

mod support;

use std::collections::HashMap;
use std::sync::Arc;

use icloud_calendar_mcp::caldav::{CalDavCalendar, CalDavClient, CalDavError, CalDavEvent, CalDavResult};
use icloud_calendar_mcp::ics::{EventPatch, NewEvent};
use icloud_calendar_mcp::scope::EventScope;
use icloud_calendar_mcp::service::{CalendarService, handle};
use regex::Regex;
use support::*;

fn setup() -> (Arc<MockCalDavClient>, CalendarService) {
    let mock = MockCalDavClient::new();
    let service = CalendarService::new(mock.clone());
    (mock, service)
}

fn cal(id: &str) -> CalDavCalendar {
    calendar(id, "Cal")
}

fn resource(uid: &str, href: &str, etag: &str, ics: &str) -> CalDavEvent {
    CalDavEvent {
        uid: uid.into(),
        href: href.into(),
        url: format!("https://test.com{href}"),
        etag: Some(etag.into()),
        ical_data: ics.into(),
    }
}

/// A one-hour UTC event on 2025-01-15 10:00, LF line endings like the Kotlin fixtures.
fn simple_ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:{uid}\nSUMMARY:{summary}\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nEND:VEVENT\nEND:VCALENDAR"
    )
}

fn timed_event(summary: &str, start: &str, end: &str) -> NewEvent {
    NewEvent {
        summary: summary.into(),
        start_time: Some(start.into()),
        end_time: Some(end.into()),
        ..NewEvent::default()
    }
}

fn summary_patch(summary: &str) -> EventPatch {
    EventPatch { summary: Some(summary.into()), ..EventPatch::default() }
}

/// Serve `event` from both the listing and the store, then list to warm the UID cache.
async fn warm(mock: &MockCalDavClient, service: &CalendarService, event: &CalDavEvent, day: &str) {
    {
        let mut s = mock.state();
        s.calendars = vec![cal("cal-1")];
        s.events_response = vec![event.clone()];
        s.registered_events.insert(event.uid.clone(), event.clone());
    }
    service.get_events("cal-1", day, day).await.unwrap();
}

// ───────────────────────────── list calendars ─────────────────────────────

#[tokio::test]
async fn list_calendars_returns_parsed_calendars() {
    let (mock, service) = setup();
    mock.state().calendars = vec![
        CalDavCalendar { color: Some("#FF5733".into()), ..calendar("personal-123", "Personal") },
        CalDavCalendar { color: Some("#3366FF".into()), ..read_only_calendar("work-456", "Work") },
    ];
    let calendars = service.list_calendars().await.unwrap();
    assert_eq!(calendars.len(), 2);
    assert_eq!(calendars[0].id, "personal-123");
    assert_eq!(calendars[0].name, "Personal");
    assert_eq!(calendars[0].color.as_deref(), Some("#FF5733"));
    assert!(!calendars[0].read_only);
    assert!(calendars[1].read_only);
    assert_eq!(calendars[0].supported_components, ["VEVENT"]);
}

#[tokio::test]
async fn list_calendars_returns_error_on_client_failure() {
    let (mock, service) = setup();
    mock.state().list_calendars_result = Some(Err(CalDavError::new(401, "Authentication failed", false)));
    let err = service.list_calendars().await.unwrap_err();
    assert_eq!(err.code, 401);
    assert!(err.message.contains("Authentication"));
}

#[tokio::test]
async fn list_calendars_validates_connection_on_first_call() {
    let (mock, service) = setup();
    mock.state().check_connection_result = Some(Err(CalDavError::bad_request("Server does not support CalDAV")));
    let err = service.list_calendars().await.unwrap_err();
    assert_eq!(err.code, 400);
    assert!(err.message.contains("CalDAV"));
}

#[tokio::test]
async fn list_calendars_caches_connection_validation() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    assert!(service.list_calendars().await.is_ok());
    mock.state().check_connection_result = Some(Err(CalDavError::server(500, "Server down")));
    assert!(service.list_calendars().await.is_ok(), "validation is cached");
    assert_eq!(mock.state().check_connection_called, 1);
}

// ───────────────────────────── get events ─────────────────────────────

#[tokio::test]
async fn get_events_returns_parsed_events() {
    let (mock, service) = setup();
    {
        let mut s = mock.state();
        s.calendars = vec![cal("personal-123")];
        s.events_response = vec![resource(
            "event-001",
            "/caldav/personal/event-001.ics",
            "\"etag1\"",
            "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:event-001\nSUMMARY:Team Meeting\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nLOCATION:Room A\nDESCRIPTION:Discuss Q1 goals\nEND:VEVENT\nEND:VCALENDAR",
        )];
    }
    let events = service.get_events("personal-123", "2025-01-15", "2025-01-15").await.unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.uid, "event-001");
    assert_eq!(e.summary, "Team Meeting");
    assert_eq!(e.start_time.as_deref(), Some("2025-01-15T10:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-01-15T11:00:00Z"));
    assert_eq!(e.location.as_deref(), Some("Room A"));
    assert_eq!(e.description.as_deref(), Some("Discuss Q1 goals"));
    assert_eq!(e.href, "/caldav/personal/event-001.ics");
    assert_eq!(e.etag.as_deref(), Some("\"etag1\""));
}

#[tokio::test]
async fn get_events_handles_all_day_events() {
    let (mock, service) = setup();
    {
        let mut s = mock.state();
        s.calendars = vec![cal("cal-1")];
        s.events_response = vec![resource(
            "allday-001",
            "/cal/allday.ics",
            "\"etag-allday\"",
            "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:allday-001\nSUMMARY:Holiday\nDTSTART;VALUE=DATE:20250115\nDTEND;VALUE=DATE:20250116\nEND:VEVENT\nEND:VCALENDAR",
        )];
    }
    let events = service.get_events("cal-1", "2025-01-15", "2025-01-15").await.unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].is_all_day);
    assert_eq!(events[0].start_date.as_deref(), Some("2025-01-15"));
    assert_eq!(events[0].end_date.as_deref(), Some("2025-01-15"), "inclusive end");
}

#[tokio::test]
async fn get_events_filters_out_unparseable_events() {
    let (mock, service) = setup();
    {
        let mut s = mock.state();
        s.calendars = vec![cal("cal-1")];
        s.events_response = vec![
            resource("good-001", "/cal/good.ics", "\"etag1\"", &simple_ics("good-001", "Good Event")),
            resource("bad-001", "/cal/bad.ics", "\"etag2\"", "INVALID ICS CONTENT"),
        ];
    }
    let events = service.get_events("cal-1", "2025-01-15", "2025-01-15").await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].uid, "good-001");
}

#[tokio::test]
async fn get_events_returns_error_for_unknown_calendar() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    let err = service.get_events("unknown-calendar", "2025-01-15", "2025-01-15").await.unwrap_err();
    assert_eq!(err.code, 404);
    assert!(err.message.contains("not found"));
}

#[tokio::test]
async fn get_events_gives_each_occurrence_a_distinct_occurrence_handle() {
    let (mock, service) = setup();
    {
        let mut s = mock.state();
        s.calendars = vec![cal("cal-1")];
        s.events_response = vec![resource(
            "daily-001",
            "/cal/daily.ics",
            "\"etag-daily\"",
            "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:daily-001\nSUMMARY:Daily standup\nDTSTART:20250115T090000Z\nDTEND:20250115T091500Z\nRRULE:FREQ=DAILY\nEND:VEVENT\nEND:VCALENDAR",
        )];
    }
    let events = service.get_events("cal-1", "2025-01-15", "2025-01-17").await.unwrap();
    assert_eq!(events.len(), 3);
    let mut recids: Vec<String> = events.iter().map(|e| e.recurrence_id.clone().unwrap()).collect();
    recids.sort();
    assert_eq!(recids, ["20250115T090000Z", "20250116T090000Z", "20250117T090000Z"]);

    let handles: std::collections::HashSet<_> = events.iter().map(|e| e.handle.clone().unwrap()).collect();
    assert_eq!(handles.len(), 3, "occurrence handles are distinct");
    for e in &events {
        let decoded = handle::EventHandle::decode(e.handle.as_ref().unwrap()).unwrap();
        assert!(decoded.is_occurrence_ref());
        assert_eq!(decoded.recurrence_id, e.recurrence_id);
        assert_eq!(decoded.href, "/cal/daily.ics");
        assert_eq!(decoded.etag.as_deref(), Some("\"etag-daily\""));
    }
}

#[tokio::test]
async fn get_events_on_standalone_event_gives_master_handle() {
    let (mock, service) = setup();
    {
        let mut s = mock.state();
        s.calendars = vec![cal("cal-1")];
        s.events_response =
            vec![resource("single-001", "/cal/single.ics", "\"etag-single\"", &simple_ics("single-001", "One-off"))];
    }
    let events = service.get_events("cal-1", "2025-01-15", "2025-01-15").await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recurrence_id, None);
    let decoded = handle::EventHandle::decode(events[0].handle.as_ref().unwrap()).unwrap();
    assert!(!decoded.is_occurrence_ref());
    assert_eq!(decoded.href, "/cal/single.ics");
    assert_eq!(decoded.etag.as_deref(), Some("\"etag-single\""));
}

// ───────────────────────────── create ─────────────────────────────

#[tokio::test]
async fn create_event_returns_success_with_event_id() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    let event = NewEvent {
        location: Some("Room B".into()),
        description: Some("Planning session".into()),
        ..timed_event("New Meeting", "2025-01-15T14:00:00Z", "2025-01-15T15:00:00Z")
    };
    let created = service.create_event("cal-1", event).await.unwrap();
    assert!(!created.uid.is_empty());
    assert_eq!(created.summary, "New Meeting");
    assert!(created.handle.as_deref().is_some_and(|h| h.starts_with("evt1_")));
    let ics = mock.state().last_created_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:New Meeting"));
    assert!(ics.contains("LOCATION:Room B"));
    assert!(
        ics.contains("CREATED:") && ics.contains("LAST-MODIFIED:"),
        "first creation stamps CREATED and LAST-MODIFIED"
    );
}

#[tokio::test]
async fn create_all_day_event() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    let event = NewEvent {
        summary: "Holiday".into(),
        start_date: Some("2025-01-15".into()),
        end_date: Some("2025-01-15".into()),
        is_all_day: true,
        ..NewEvent::default()
    };
    let created = service.create_event("cal-1", event).await.unwrap();
    assert!(created.is_all_day);
    assert_eq!(created.start_date.as_deref(), Some("2025-01-15"));
    assert!(mock.state().last_created_ics.as_ref().unwrap().contains("DTSTART;VALUE=DATE:20250115"));
}

#[tokio::test]
async fn create_event_fails_on_read_only_calendar() {
    let (mock, service) = setup();
    mock.state().calendars = vec![read_only_calendar("cal-1", "Cal")];
    let err = service
        .create_event("cal-1", timed_event("Test", "2025-01-15T10:00:00Z", "2025-01-15T11:00:00Z"))
        .await
        .unwrap_err();
    assert_eq!(err.code, 403);
    assert!(err.message.contains("read-only"));
    assert_eq!(mock.state().create_event_called, 0);
}

#[tokio::test]
async fn create_event_in_unknown_calendar_is_404() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    let err = service
        .create_event("nope", timed_event("Test", "2025-01-15T10:00:00Z", "2025-01-15T11:00:00Z"))
        .await
        .unwrap_err();
    assert_eq!(err.code, 404);
    assert!(err.message.contains("Calendar not found"));
}

#[tokio::test]
async fn create_event_with_unreadable_date_is_400_without_writing() {
    // Deliberate deviation: Kotlin surfaced the builder's parse exception as an
    // internal error; the Rust service reports it as invalid event data.
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    let event = NewEvent {
        summary: "x".into(),
        start_date: Some("2025-13-45".into()),
        is_all_day: true,
        ..NewEvent::default()
    };
    let err = service.create_event("cal-1", event).await.unwrap_err();
    assert_eq!(err.code, 400);
    assert!(err.message.starts_with("Invalid event data"), "{}", err.message);
    assert_eq!(mock.state().create_event_called, 0);
}

// ───────────────────────────── update ─────────────────────────────

#[tokio::test]
async fn update_event_modifies_existing_event() {
    let (mock, service) = setup();
    let existing = resource("event-001", "/cal/event-001.ics", "\"old-etag\"", &simple_ics("event-001", "Old Title"));
    warm(&mock, &service, &existing, "2025-01-15").await;

    let patch = EventPatch { location: Some("New Location".into()), ..summary_patch("New Title") };
    service.update_event("event-001", patch, None).await.unwrap();

    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:New Title"));
    assert!(ics.contains("LOCATION:New Location"));
    assert_eq!(mock.state().update_etags_seen, [Some("\"old-etag\"".to_string())]);
}

#[tokio::test]
async fn update_event_returns_error_for_unknown_event() {
    let (_mock, service) = setup();
    let err = service.update_event("unknown-event", summary_patch("Test"), None).await.unwrap_err();
    assert_eq!(err.code, 404);
}

#[tokio::test]
async fn update_event_resolves_by_handle_from_a_cold_cache() {
    let (mock, service) = setup();
    let event = resource("cold-001", "/cal/cold-001.ics", "\"etag-cold\"", &simple_ics("cold-001", "Cold Title"));
    mock.state().registered_events.insert(event.uid.clone(), event.clone());

    let token = handle::encode(&event.href, event.etag.as_deref(), None);
    service.update_event(&token, summary_patch("New Cold Title"), None).await.unwrap();

    let s = mock.state();
    assert!(s.get_event_called >= 1, "cold resolution fetches via get_event");
    assert!(s.last_updated_ics.as_ref().unwrap().contains("SUMMARY:New Cold Title"));
}

#[tokio::test]
async fn delete_event_resolves_by_handle_from_a_cold_cache() {
    let (mock, service) = setup();
    let event = resource(
        "cold-del-001",
        "/cal/cold-del-001.ics",
        "\"etag-cd\"",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:cold-del-001\nEND:VEVENT\nEND:VCALENDAR",
    );
    mock.state().registered_events.insert(event.uid.clone(), event.clone());

    service.delete_event(&handle::encode(&event.href, event.etag.as_deref(), None), None).await.unwrap();

    let s = mock.state();
    assert_eq!(s.last_deleted_href.as_deref(), Some("/cal/cold-del-001.ics"));
    assert_eq!(s.delete_etags_seen.first().cloned().flatten().as_deref(), Some("\"etag-cd\""));
    assert_eq!(s.get_event_called, 0, "happy-path delete needs no fetch");
}

#[tokio::test]
async fn handle_update_sends_the_handles_etag_as_if_match() {
    let (mock, service) = setup();
    let server_event = resource("oc-001", "/cal/oc-001.ics", "\"server-now\"", &simple_ics("oc-001", "Original"));
    mock.state().registered_events.insert(server_event.uid.clone(), server_event.clone());

    let token = handle::encode(&server_event.href, Some("\"caller-saw\""), None);
    service.update_event(&token, summary_patch("Edited"), None).await.unwrap();
    assert_eq!(mock.state().update_etags_seen[0].as_deref(), Some("\"caller-saw\""));
}

#[tokio::test]
async fn handle_update_falls_back_to_fetched_etag_without_one() {
    let (mock, service) = setup();
    let server_event = resource("oc-002", "/cal/oc-002.ics", "\"server-etag\"", &simple_ics("oc-002", "S"));
    mock.state().registered_events.insert(server_event.uid.clone(), server_event.clone());

    let token = handle::encode(&server_event.href, None, None);
    service.update_event(&token, summary_patch("Edited"), None).await.unwrap();
    assert_eq!(mock.state().update_etags_seen[0].as_deref(), Some("\"server-etag\""));
}

#[tokio::test]
async fn update_returns_refreshed_handle_with_new_etag() {
    let (mock, service) = setup();
    let event = resource("fresh-001", "/cal/fresh-001.ics", "\"v1\"", &simple_ics("fresh-001", "Title"));
    mock.state().registered_events.insert(event.uid.clone(), event.clone());

    let updated = service
        .update_event(&handle::encode(&event.href, Some("\"v1\""), None), summary_patch("T2"), None)
        .await
        .unwrap();
    let decoded = handle::EventHandle::decode(updated.handle.as_ref().unwrap()).unwrap();
    assert_eq!(decoded.etag.as_deref(), Some("\"updated-etag\""));
    assert_eq!(updated.summary, "T2");
}

#[tokio::test]
async fn update_of_unparseable_existing_body_is_422() {
    let (mock, service) = setup();
    let event = resource("junk-001", "/cal/junk-001.ics", "\"e\"", "this is not ics");
    mock.state().registered_events.insert(event.uid.clone(), event.clone());

    let err =
        service.update_event(&handle::encode(&event.href, None, None), summary_patch("X"), None).await.unwrap_err();
    assert_eq!(err.code, 422);
    assert!(err.message.contains("unparseable"), "{}", err.message);
    assert_eq!(mock.state().update_event_called, 0);
}

// ───────────────────────────── 412 refetch-and-retry-once ─────────────────────────────

#[tokio::test]
async fn update_recovers_from_stale_etag_412_by_refetching_once() {
    let (mock, service) = setup();
    let stale = resource("retry-001", "/cal/retry-001.ics", "\"stale-etag\"", &simple_ics("retry-001", "Retry Title"));
    warm(&mock, &service, &stale, "2025-01-15").await;
    {
        let mut s = mock.state();
        s.get_event_result = Some(Ok(CalDavEvent { etag: Some("\"fresh-etag\"".into()), ..stale.clone() }));
        s.fail_412_updates_remaining = 1;
    }

    service.update_event("retry-001", summary_patch("Reconciled"), None).await.unwrap();

    let s = mock.state();
    assert_eq!(s.update_event_called, 2, "exactly one retry after the 412");
    assert_eq!(s.update_etags_seen[0].as_deref(), Some("\"stale-etag\""));
    assert_eq!(s.update_etags_seen[1].as_deref(), Some("\"fresh-etag\""));
    assert!(s.last_updated_ics.as_ref().unwrap().contains("SUMMARY:Reconciled"));
}

#[tokio::test]
async fn update_does_not_retry_more_than_once_on_repeated_412() {
    let (mock, service) = setup();
    let event = resource(
        "retry-002",
        "/cal/retry-002.ics",
        "\"e\"",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:retry-002\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nEND:VEVENT\nEND:VCALENDAR",
    );
    {
        let mut s = mock.state();
        s.registered_events.insert(event.uid.clone(), event.clone());
        s.get_event_result = Some(Ok(event.clone()));
        s.fail_412_updates_remaining = 5;
    }

    let err = service
        .update_event(&handle::encode(&event.href, event.etag.as_deref(), None), summary_patch("X"), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, 412);
    assert_eq!(mock.state().update_event_called, 2, "one initial + exactly one retry");
}

#[tokio::test]
async fn update_412_with_failed_refetch_surfaces_the_conflict() {
    let (mock, service) = setup();
    let stale = resource("retry-003", "/cal/retry-003.ics", "\"stale\"", &simple_ics("retry-003", "T"));
    warm(&mock, &service, &stale, "2025-01-15").await;
    {
        let mut s = mock.state();
        s.fail_412_updates_remaining = 1;
        s.get_event_result = Some(Err(CalDavError::server(503, "Server error")));
    }
    let err = service.update_event("retry-003", summary_patch("X"), None).await.unwrap_err();
    assert_eq!(err.code, 412, "the original conflict is reported, not the refetch failure");
    assert_eq!(mock.state().update_event_called, 1);
}

#[tokio::test]
async fn delete_recovers_from_stale_etag_412_by_refetching_once() {
    let (mock, service) = setup();
    let event = resource(
        "retry-del-001",
        "/cal/retry-del-001.ics",
        "\"stale\"",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:retry-del-001\nEND:VEVENT\nEND:VCALENDAR",
    );
    {
        let mut s = mock.state();
        s.registered_events.insert(event.uid.clone(), event.clone());
        s.get_event_result = Some(Ok(CalDavEvent { etag: Some("\"fresh\"".into()), ..event.clone() }));
        s.fail_412_deletes_remaining = 1;
    }

    service.delete_event(&handle::encode(&event.href, Some("\"stale\""), None), None).await.unwrap();

    let s = mock.state();
    assert_eq!(s.delete_event_called, 2, "exactly one retry after the 412");
    assert_eq!(s.delete_etags_seen[0].as_deref(), Some("\"stale\""));
    assert_eq!(s.delete_etags_seen[1].as_deref(), Some("\"fresh\""));
}

// ───────────────────────────── cache eviction ─────────────────────────────

#[tokio::test]
async fn handle_based_delete_evicts_the_uid_keyed_cache_entry() {
    let (mock, service) = setup();
    let event = resource(
        "evict-001",
        "/cal/evict-001.ics",
        "\"e-evict\"",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:evict-001\nEND:VEVENT\nEND:VCALENDAR",
    );
    warm(&mock, &service, &event, "2025-01-15").await;
    assert_eq!(service.cache_size(), 1);

    service.delete_event(&handle::encode(&event.href, event.etag.as_deref(), None), None).await.unwrap();
    assert_eq!(service.cache_size(), 0, "no stale UID entry left to PUT-recreate the event");
}

#[tokio::test]
async fn delete_by_partition_href_evicts_canonical_cache_entry() {
    let (mock, service) = setup();
    let canonical = "https://caldav.icloud.com/123/calendars/home/p.ics";
    let event = resource("part-001", canonical, "\"e\"", &simple_ics("part-001", "Partitioned"));
    warm(&mock, &service, &event, "2025-01-15").await;
    assert_eq!(service.cache_size(), 1);

    let token = handle::encode("https://p180-caldav.icloud.com:443/123/calendars/home/p.ics", Some("\"e\""), None);
    service.delete_event(&token, None).await.unwrap();
    assert_eq!(service.cache_size(), 0);
}

// ───────────────────────────── delete ─────────────────────────────

#[tokio::test]
async fn delete_event_removes_existing_event() {
    let (mock, service) = setup();
    let event = resource("event-001", "/cal/event-001.ics", "\"etag1\"", &simple_ics("event-001", "Test Event"));
    warm(&mock, &service, &event, "2025-01-15").await;

    service.delete_event("event-001", None).await.unwrap();
    assert_eq!(mock.state().last_deleted_href.as_deref(), Some("/cal/event-001.ics"));
    assert!(service.get_event_by_id("event-001").is_err(), "evicted from the cache");
}

#[tokio::test]
async fn delete_event_returns_error_for_unknown_event() {
    let (_mock, service) = setup();
    assert_eq!(service.delete_event("unknown-event", None).await.unwrap_err().code, 404);
}

#[tokio::test]
async fn delete_event_handles_client_error() {
    let (mock, service) = setup();
    let event = resource("event-001", "/cal/event-001.ics", "\"etag1\"", &simple_ics("event-001", "Test Event"));
    warm(&mock, &service, &event, "2025-01-15").await;
    mock.state().delete_event_result = Some(Err(CalDavError::new(412, "Precondition failed", false)));

    let err = service.delete_event("event-001", None).await.unwrap_err();
    assert_eq!(err.code, 412);
}

// ───────────────────────────── lookup ─────────────────────────────

#[tokio::test]
async fn find_event_by_id_after_fetching_events() {
    let (mock, service) = setup();
    let event = resource("event-in-cal2", "/cal-2/event.ics", "\"etag\"", &simple_ics("event-in-cal2", "Found Event"));
    {
        let mut s = mock.state();
        s.calendars = vec![cal("cal-1"), cal("cal-2")];
        s.events_response = vec![event.clone()];
    }
    service.get_events("cal-2", "2025-01-15", "2025-01-15").await.unwrap();
    let found = service.get_event_by_id("event-in-cal2").unwrap();
    assert_eq!(found.uid, "event-in-cal2");
    assert_eq!(found.summary, "Found Event");
}

#[tokio::test]
async fn find_event_by_id_returns_error_if_not_in_cache() {
    let (_mock, service) = setup();
    assert_eq!(service.get_event_by_id("unknown-event").unwrap_err().code, 404);
}

// ───────────────────────────── property preservation ─────────────────────────────

#[tokio::test]
async fn update_event_preserves_valarm_attendee_and_x_props() {
    let (mock, service) = setup();
    let rich = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\nBEGIN:VEVENT\nUID:rich-event-001\nDTSTAMP:20251220T100000Z\nDTSTART:20251225T100000Z\nDTEND:20251225T110000Z\nSUMMARY:Team Meeting\nDESCRIPTION:Weekly sync\nLOCATION:Room A\nORGANIZER;CN=Boss:mailto:boss@example.com\nATTENDEE;CN=Alice:mailto:alice@example.com\nATTENDEE;CN=Bob:mailto:bob@example.com\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:15 min reminder\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT1H\nDESCRIPTION:1 hour reminder\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR";
    let event = resource("rich-event-001", "/cal/rich-event-001.ics", "\"etag1\"", rich);
    warm(&mock, &service, &event, "2025-12-25").await;

    service.update_event("rich-event-001", summary_patch("Updated Meeting"), None).await.unwrap();

    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:Updated Meeting"));
    for preserved in ["boss@example.com", "alice@example.com", "bob@example.com", "X-APPLE-TRAVEL-ADVISORY"] {
        assert!(ics.contains(preserved), "{preserved} preserved:\n{ics}");
    }
    assert_eq!(ics.matches("BEGIN:VALARM").count(), 2, "both VALARM blocks preserved");
    assert!(ics.contains("TRIGGER:-PT15M"));
    assert!(ics.contains("TRIGGER:-PT1H"));
    assert!(ics.contains("SEQUENCE:1"), "SEQUENCE incremented");
    assert!(ics.contains("DESCRIPTION:Weekly sync"));
    assert!(ics.contains("LOCATION:Room A"));
}

#[tokio::test]
async fn update_event_preserves_properties_through_multiple_updates() {
    let (mock, service) = setup();
    let original = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\nBEGIN:VEVENT\nUID:multi-update-001\nDTSTAMP:20251220T100000Z\nDTSTART:20251225T100000Z\nDTEND:20251225T110000Z\nSUMMARY:Original\nORGANIZER;CN=John:mailto:john@example.com\nATTENDEE;CN=Jane:mailto:jane@example.com\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT30M\nDESCRIPTION:Reminder\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR";
    let event = resource("multi-update-001", "/cal/multi-update-001.ics", "\"etag1\"", original);
    warm(&mock, &service, &event, "2025-12-25").await;

    service.update_event("multi-update-001", summary_patch("V2"), None).await.unwrap();
    let v2 = mock.state().last_updated_ics.clone().unwrap();
    assert!(v2.contains("SUMMARY:V2"));
    assert!(v2.contains("jane@example.com"));
    assert!(v2.contains("BEGIN:VALARM"));

    service.clear_cache();
    let v2_event = resource("multi-update-001", "/cal/multi-update-001.ics", "\"etag2\"", &v2);
    warm(&mock, &service, &v2_event, "2025-12-25").await;

    let patch = EventPatch { location: Some("Room 42".into()), ..EventPatch::default() };
    service.update_event("multi-update-001", patch, None).await.unwrap();
    let v3 = mock.state().last_updated_ics.clone().unwrap();
    assert!(v3.contains("SUMMARY:V2"), "title from v2 preserved");
    assert!(v3.contains("LOCATION:Room 42"));
    assert!(v3.contains("jane@example.com"));
    assert!(v3.contains("BEGIN:VALARM"));
    assert!(v3.contains("SEQUENCE:2"), "SEQUENCE incremented twice");
    assert_eq!(v3.matches("BEGIN:VALARM").count(), 1, "alarm not duplicated across updates");
}

#[tokio::test]
async fn full_crud_flow_with_extended_fields() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];

    let event = NewEvent {
        description: Some("Team sync".into()),
        location: Some("Zoom".into()),
        rrule: Some("FREQ=WEEKLY;BYDAY=MO".into()),
        ..timed_event("Weekly Standup", "2025-12-25T10:00:00Z", "2025-12-25T11:00:00Z")
    };
    let created = service.create_event("cal-1", event).await.unwrap();
    assert_eq!(created.summary, "Weekly Standup");
    assert!(mock.state().last_created_ics.as_ref().unwrap().contains("RRULE:FREQ=WEEKLY"));

    let uid = created.uid.clone();
    assert!(service.get_events("cal-1", "2025-12-25", "2025-12-25").await.is_ok());
    assert_eq!(service.get_event_by_id(&uid).unwrap().summary, "Weekly Standup");

    service.update_event(&uid, summary_patch("Daily Standup"), None).await.unwrap();
    let updated = mock.state().last_updated_ics.clone().unwrap();
    assert!(updated.contains("SUMMARY:Daily Standup"));
    assert!(updated.contains("RRULE:FREQ=WEEKLY"));
    assert!(updated.contains("LOCATION:Zoom"));
    assert!(updated.contains("DESCRIPTION:Team sync"));

    service.clear_cache();
    let stored = resource(&uid, &format!("/cal/{uid}.ics"), "\"updated-etag\"", &updated);
    warm(&mock, &service, &stored, "2025-12-25").await;
    service.delete_event(&uid, None).await.unwrap();
    assert_eq!(mock.state().last_deleted_href, Some(format!("/cal/{uid}.ics")));
}

// ───────────────────────────── read-after-write ─────────────────────────────

#[tokio::test]
async fn create_success_is_authoritative_despite_a_lagging_listing() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    let created = service
        .create_event("cal-1", timed_event("Fresh event", "2025-01-15T09:00:00Z", "2025-01-15T10:00:00Z"))
        .await
        .unwrap();
    let token = created.handle.clone().expect("create returns a handle");

    mock.state().events_response = Vec::new();
    assert!(service.get_events("cal-1", "2025-01-15", "2025-01-15").await.unwrap().is_empty());

    let href = handle::EventHandle::decode(&token).unwrap().href;
    assert!(mock.get_event(&href).await.is_ok(), "reachable by handle despite listing lag");
}

#[tokio::test]
async fn lagging_empty_listing_does_not_evict_a_just_created_event() {
    let (mock, service) = setup();
    mock.state().calendars = vec![cal("cal-1")];
    service
        .create_event("cal-1", timed_event("Cached event", "2025-01-15T09:00:00Z", "2025-01-15T10:00:00Z"))
        .await
        .unwrap();
    let size_after_create = service.cache_size();
    assert!(size_after_create >= 1);

    mock.state().events_response = Vec::new();
    service.get_events("cal-1", "2025-01-15", "2025-01-15").await.unwrap();
    assert_eq!(service.cache_size(), size_after_create);
}

// ───────────────────────────── scoped writes ─────────────────────────────

const SERIES_HREF: &str = "/cal/daily.ics";
const SERIES_ETAG: &str = "\"e-series\"";

/// A daily 09:00–09:15Z series from 2025-01-15, stored so get_event(href) finds it.
fn register_daily_series(mock: &MockCalDavClient, rrule: &str) -> CalDavEvent {
    let event = resource(
        "daily-series",
        SERIES_HREF,
        SERIES_ETAG,
        &format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\nBEGIN:VEVENT\nUID:daily-series\nSUMMARY:Daily standup\nDTSTART:20250115T090000Z\nDTEND:20250115T091500Z\n{rrule}\nEND:VEVENT\nEND:VCALENDAR"
        ),
    );
    mock.state().registered_events.insert(event.uid.clone(), event.clone());
    event
}

fn occurrence(recurrence_id: &str) -> String {
    handle::encode(SERIES_HREF, Some(SERIES_ETAG), Some(recurrence_id))
}

fn writes(mock: &MockCalDavClient) -> (u32, u32, u32) {
    let s = mock.state();
    (s.update_event_called, s.create_event_called, s.delete_event_called)
}

fn moved_patch() -> EventPatch {
    EventPatch {
        start_time: Some("2025-01-16T15:00:00Z".into()),
        end_time: Some("2025-01-16T16:00:00Z".into()),
        ..EventPatch::default()
    }
}

#[tokio::test]
async fn update_on_occurrence_reference_without_scope_is_rejected() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    let err = service.update_event(&occurrence("20250116T090000Z"), summary_patch("Renamed"), None).await.unwrap_err();
    assert_eq!(err.code, 400);
    assert_eq!(writes(&mock), (0, 0, 0));
}

#[tokio::test]
async fn delete_on_occurrence_reference_without_scope_is_rejected() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    let err = service.delete_event(&occurrence("20250116T090000Z"), None).await.unwrap_err();
    assert_eq!(err.code, 400);
    assert_eq!(writes(&mock), (0, 0, 0));
}

#[tokio::test]
async fn this_occurrence_scope_on_master_or_uid_reference_is_rejected() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    let master = handle::encode(SERIES_HREF, Some(SERIES_ETAG), None);
    for reference in [master.as_str(), "daily-series"] {
        let err =
            service.update_event(reference, summary_patch("X"), Some(EventScope::ThisOccurrence)).await.unwrap_err();
        assert_eq!(err.code, 400, "{reference}");
    }
    assert_eq!(writes(&mock), (0, 0, 0));
}

#[tokio::test]
async fn series_level_fields_with_occurrence_scopes_are_rejected() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    for scope in [EventScope::ThisOccurrence, EventScope::ThisAndFuture] {
        let patch = EventPatch { rrule: Some("FREQ=WEEKLY".into()), ..EventPatch::default() };
        let err = service.update_event(&occurrence("20250117T090000Z"), patch, Some(scope)).await.unwrap_err();
        assert_eq!(err.code, 400);
    }
    assert_eq!(writes(&mock), (0, 0, 0));
}

#[tokio::test]
async fn update_this_occurrence_writes_an_exception_and_keeps_the_master() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    let info = service
        .update_event(&occurrence("20250116T090000Z"), moved_patch(), Some(EventScope::ThisOccurrence))
        .await
        .unwrap();

    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("RRULE:FREQ=DAILY"), "master RRULE preserved:\n{ics}");
    assert!(ics.contains("DTSTART:20250115T090000Z"), "master DTSTART unchanged:\n{ics}");
    assert!(ics.contains("RECURRENCE-ID:20250116T090000Z"), "exception identifies the occurrence:\n{ics}");
    assert!(ics.contains("DTSTART:20250116T150000Z"), "exception moved:\n{ics}");

    assert_eq!(info.recurrence_id.as_deref(), Some("20250116T090000Z"));
    assert!(info.start_time.as_deref().unwrap().starts_with("2025-01-16T15:00"), "{:?}", info.start_time);
    let decoded = handle::EventHandle::decode(info.handle.as_ref().unwrap()).unwrap();
    assert!(decoded.is_occurrence_ref());
    assert_eq!(decoded.recurrence_id.as_deref(), Some("20250116T090000Z"));
    assert_eq!(decoded.etag.as_deref(), Some("\"updated-etag\""));
}

#[tokio::test]
async fn delete_this_occurrence_adds_an_exdate() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    service.delete_event(&occurrence("20250116T090000Z"), Some(EventScope::ThisOccurrence)).await.unwrap();

    assert_eq!(writes(&mock), (1, 0, 0), "a PUT of the master, not a DELETE");
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("RRULE:FREQ=DAILY"));
    assert!(ics.contains("DTSTART:20250115T090000Z"));
    assert!(Regex::new(r"EXDATE[^\n]*20250116").unwrap().is_match(&ics), "{ics}");
}

#[tokio::test]
async fn occurrence_writes_for_an_instant_not_in_the_series_are_409() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY;COUNT=3");
    let outside = occurrence("20250201T090000Z");

    for scope in [EventScope::ThisOccurrence, EventScope::ThisAndFuture] {
        assert_eq!(service.delete_event(&outside, Some(scope)).await.unwrap_err().code, 409);
        assert_eq!(service.update_event(&outside, summary_patch("x"), Some(scope)).await.unwrap_err().code, 409);
    }
    assert_eq!(writes(&mock), (0, 0, 0));
}

#[tokio::test]
async fn occurrence_write_on_a_non_recurring_event_is_400() {
    let (mock, service) = setup();
    let event = resource("single", SERIES_HREF, SERIES_ETAG, &simple_ics("single", "One-off"));
    mock.state().registered_events.insert(event.uid.clone(), event);
    // A standalone event has no occurrence at that instant, so the liveness check fails first.
    let err = service
        .update_event(&occurrence("20250115T100000Z"), summary_patch("x"), Some(EventScope::ThisOccurrence))
        .await
        .unwrap_err();
    assert_eq!(err.code, 409);
    assert_eq!(writes(&mock), (0, 0, 0));
}

#[tokio::test]
async fn a_412_on_this_occurrence_delete_reapplies_the_exdate() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    mock.state().fail_412_updates_remaining = 1;

    service.delete_event(&occurrence("20250116T090000Z"), Some(EventScope::ThisOccurrence)).await.unwrap();

    assert_eq!(writes(&mock), (2, 0, 0));
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(Regex::new(r"EXDATE[^\n]*20250116").unwrap().is_match(&ics), "{ics}");
    assert!(ics.contains("RRULE:FREQ=DAILY"));
}

#[tokio::test]
async fn a_412_on_this_and_future_delete_recomputes_the_truncation() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    mock.state().fail_412_updates_remaining = 1;

    service.delete_event(&occurrence("20250117T090000Z"), Some(EventScope::ThisAndFuture)).await.unwrap();

    assert_eq!(writes(&mock), (2, 0, 0));
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("UNTIL=20250116T090000Z"), "{ics}");
    assert!(ics.contains("DTSTART:20250115T090000Z"));
}

#[tokio::test]
async fn all_events_scope_on_occurrence_reference_edits_the_whole_series() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    service
        .update_event(&occurrence("20250116T090000Z"), summary_patch("Renamed series"), Some(EventScope::AllEvents))
        .await
        .unwrap();

    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:Renamed series"));
    assert!(ics.contains("RRULE:FREQ=DAILY"));
    assert!(!ics.contains("RECURRENCE-ID"));
}

#[tokio::test]
async fn a_412_on_occurrence_write_reapplies_the_exception() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    mock.state().fail_412_updates_remaining = 1;

    service
        .update_event(&occurrence("20250116T090000Z"), moved_patch(), Some(EventScope::ThisOccurrence))
        .await
        .unwrap();

    assert_eq!(writes(&mock).0, 2);
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("RECURRENCE-ID:20250116T090000Z"), "{ics}");
    assert!(ics.contains("DTSTART:20250116T150000Z"), "{ics}");
}

#[tokio::test]
async fn update_this_and_future_truncates_and_creates_a_new_series() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    let info = service
        .update_event(&occurrence("20250117T090000Z"), summary_patch("Renamed onward"), Some(EventScope::ThisAndFuture))
        .await
        .unwrap();

    assert_eq!(writes(&mock), (1, 1, 0));
    let (master, new_series) = {
        let s = mock.state();
        (s.last_updated_ics.clone().unwrap(), s.last_created_ics.clone().unwrap())
    };
    assert!(master.contains("UID:daily-series"));
    assert!(master.contains("DTSTART:20250115T090000Z"));
    assert!(master.contains("UNTIL=20250116T090000Z"), "{master}");
    assert!(!master.contains("SUMMARY:Renamed onward"));

    assert!(new_series.contains("SUMMARY:Renamed onward"), "{new_series}");
    assert!(new_series.contains("DTSTART:20250117T090000Z"), "{new_series}");
    assert!(new_series.contains("RRULE:FREQ=DAILY"));
    assert!(!new_series.contains("RECURRENCE-ID"));
    assert!(!new_series.contains("UID:daily-series"));

    assert_eq!(info.summary, "Renamed onward");
}

#[tokio::test]
async fn delete_this_and_future_truncates_without_deleting_the_resource() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    service.delete_event(&occurrence("20250117T090000Z"), Some(EventScope::ThisAndFuture)).await.unwrap();

    assert_eq!(writes(&mock), (1, 0, 0));
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("DTSTART:20250115T090000Z"));
    assert!(ics.contains("UNTIL=20250116T090000Z"));
    assert!(!ics.contains("RECURRENCE-ID"));
}

#[tokio::test]
async fn update_this_and_future_at_the_first_occurrence_edits_the_whole_series() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    service
        .update_event(&occurrence("20250115T090000Z"), summary_patch("Renamed all"), Some(EventScope::ThisAndFuture))
        .await
        .unwrap();

    assert_eq!(writes(&mock), (1, 0, 0), "one PUT, no split");
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:Renamed all"));
    assert!(ics.contains("RRULE:FREQ=DAILY"));
    assert!(ics.contains("DTSTART:20250115T090000Z"));
    assert!(!ics.contains("RECURRENCE-ID"));
    assert!(!ics.contains("UNTIL="));
}

#[tokio::test]
async fn delete_this_and_future_at_the_first_occurrence_deletes_the_resource() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");

    service.delete_event(&occurrence("20250115T090000Z"), Some(EventScope::ThisAndFuture)).await.unwrap();

    assert_eq!(writes(&mock), (0, 0, 1));
}

#[tokio::test]
async fn failed_truncation_after_creating_the_new_series_reports_the_duplicate() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    mock.state().update_event_result = Some(Err(CalDavError::new(507, "Insufficient storage", false)));

    let err = service
        .update_event(&occurrence("20250117T090000Z"), summary_patch("Renamed onward"), Some(EventScope::ThisAndFuture))
        .await
        .unwrap_err();
    assert_eq!(err.code, 507);
    assert!(err.message.contains("duplicate"), "{}", err.message);
    assert_eq!(mock.state().create_event_called, 1);
}

#[tokio::test]
async fn failed_create_of_the_new_series_leaves_the_master_untouched() {
    let mock = MockCalDavClient::new();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    let client = Arc::new(Intercept { inner: mock.clone(), fail_create: true, created_in: Default::default() });
    let service = CalendarService::new(client);

    let err = service
        .update_event(&occurrence("20250117T090000Z"), summary_patch("x"), Some(EventScope::ThisAndFuture))
        .await
        .unwrap_err();
    assert_eq!(err.code, 500);
    assert_eq!(mock.state().update_event_called, 0, "nothing truncated when the create fails");
}

#[tokio::test]
async fn a_412_on_this_and_future_master_put_recomputes_and_retries_once() {
    let (mock, service) = setup();
    register_daily_series(&mock, "RRULE:FREQ=DAILY");
    mock.state().fail_412_updates_remaining = 1;

    service
        .update_event(&occurrence("20250117T090000Z"), summary_patch("Renamed onward"), Some(EventScope::ThisAndFuture))
        .await
        .unwrap();

    assert_eq!(writes(&mock), (2, 1, 0), "continuing series created once, master re-PUT once");
    assert!(mock.state().last_updated_ics.as_ref().unwrap().contains("UNTIL=20250116T090000Z"));
}

#[tokio::test]
async fn this_and_future_new_series_is_created_in_the_masters_calendar() {
    let mock = MockCalDavClient::new();
    let event = resource(
        "daily-series",
        "/123/calendars/work/daily.ics",
        SERIES_ETAG,
        "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:daily-series\nSUMMARY:Daily standup\nDTSTART:20250115T090000Z\nDTEND:20250115T091500Z\nRRULE:FREQ=DAILY\nEND:VEVENT\nEND:VCALENDAR",
    );
    mock.state().registered_events.insert(event.uid.clone(), event.clone());
    let client = Arc::new(Intercept { inner: mock.clone(), fail_create: false, created_in: Default::default() });
    let service = CalendarService::new(client.clone());

    let token = handle::encode(&event.href, event.etag.as_deref(), Some("20250117T090000Z"));
    service.update_event(&token, summary_patch("x"), Some(EventScope::ThisAndFuture)).await.unwrap();
    assert_eq!(client.created_in.lock().unwrap().as_slice(), ["work"]);
}

/// Delegates to the mock, recording the calendar of each create and optionally failing it.
struct Intercept {
    inner: Arc<MockCalDavClient>,
    fail_create: bool,
    created_in: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl CalDavClient for Intercept {
    async fn list_calendars(&self) -> CalDavResult<Vec<CalDavCalendar>> {
        self.inner.list_calendars().await
    }
    async fn get_events(&self, calendar: &str, start: &str, end: &str) -> CalDavResult<Vec<CalDavEvent>> {
        self.inner.get_events(calendar, start, end).await
    }
    async fn get_event(&self, href: &str) -> CalDavResult<CalDavEvent> {
        self.inner.get_event(href).await
    }
    async fn create_event(&self, calendar: &str, ics: &str) -> CalDavResult<CalDavEvent> {
        self.created_in.lock().unwrap().push(calendar.to_string());
        if self.fail_create {
            return Err(CalDavError::server(500, "Server error"));
        }
        self.inner.create_event(calendar, ics).await
    }
    async fn update_event(&self, href: &str, ics: &str, etag: Option<&str>) -> CalDavResult<CalDavEvent> {
        self.inner.update_event(href, ics, etag).await
    }
    async fn delete_event(&self, href: &str, etag: Option<&str>) -> CalDavResult<()> {
        self.inner.delete_event(href, etag).await
    }
    async fn check_connection(&self) -> CalDavResult<bool> {
        self.inner.check_connection().await
    }
    async fn fetch_etags(
        &self,
        calendar: &str,
        start: &str,
        end: &str,
    ) -> CalDavResult<HashMap<String, Option<String>>> {
        self.inner.fetch_etags(calendar, start, end).await
    }
}
