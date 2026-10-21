//! MCP-layer wiring for scoped writes and occurrence identity: get_events
//! emits `recurrenceId` per occurrence, update/delete validate `scope` before
//! any CalDAV call, and a valid scope reaches the service.
//! Port of `ScopeMcpWiringTest.kt` (the mocked service is replaced by the mock
//! CalDAV client, so threading is observed through the requests it receives).

mod support;

use std::sync::Arc;

use icloud_calendar_mcp::server::CalendarServer;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use support::*;

const SERIES: &str = "BEGIN:VEVENT\r\nUID:series-1\r\nDTSTART:20260115T090000Z\r\nDTEND:20260115T091500Z\r\nRRULE:FREQ=DAILY;COUNT=5\r\nSUMMARY:Standup\r\nEND:VEVENT\r\n";

fn setup() -> (Arc<MockCalDavClient>, CalendarServer) {
    let mock = MockCalDavClient::with_calendars(vec![calendar("cal-1", "Work")]);
    let events = vec![
        caldav_event("series-1", &vcalendar(SERIES)),
        caldav_event(
            "single-1",
            &vcalendar(&timed_vevent("single-1", "One-off", "20260115T100000Z", "20260115T110000Z")),
        ),
    ];
    {
        // The listing and the writable store both hold the events, as a server would.
        let mut s = mock.state();
        s.registered_events = events.iter().map(|e| (e.uid.clone(), e.clone())).collect();
        s.events_response = events;
    }
    let (server, _) = server(&mock);
    (mock, server)
}

async fn call(server: &CalendarServer, name: &str, args: Value) -> CallToolResult {
    server.call_tool_by_name(name, args.as_object().cloned()).await.unwrap()
}

async fn events_on(server: &CalendarServer, day: &str) -> Vec<Value> {
    let result = call(server, "get_events", json!({"calendar_id": "cal-1", "start_date": day, "end_date": day})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    result_json(&result)["events"].as_array().unwrap().clone()
}

fn by_uid<'a>(events: &'a [Value], uid: &str) -> &'a Value {
    events.iter().find(|e| e["uid"] == uid).unwrap_or_else(|| panic!("no event {uid}"))
}

#[tokio::test]
async fn get_events_emits_recurrence_id_for_occurrences_only() {
    let (_, server) = setup();
    let events = events_on(&server, "2026-01-16").await;
    let occurrence = by_uid(&events, "series-1");
    assert_eq!(occurrence["recurrenceId"], "20260116T090000Z");
    assert_eq!(occurrence["rrule"], "FREQ=DAILY;COUNT=5", "rrule still emitted");
    assert!(occurrence["handle"].as_str().unwrap().starts_with("evt2_"));
    assert_eq!(occurrence["startTime"], "2026-01-16T09:00:00Z", "the occurrence's own date, not the master's");
    let all = events_on(&server, "2026-01-15").await;
    let standalone = by_uid(&all, "single-1");
    assert!(standalone.get("recurrenceId").is_none(), "standalone event carries no recurrenceId");
    assert!(standalone["handle"].as_str().unwrap().starts_with("evt1_"));
}

#[tokio::test]
async fn each_occurrence_in_range_is_its_own_result() {
    let (_, server) = setup();
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": "cal-1", "start_date": "2026-01-01", "end_date": "2026-01-31"}),
    )
    .await;
    let events = result_json(&result)["events"].as_array().unwrap().clone();
    let occurrences: Vec<&Value> = events.iter().filter(|e| e["uid"] == "series-1").collect();
    assert_eq!(occurrences.len(), 5, "COUNT=5");
    let ids: std::collections::HashSet<&str> = occurrences.iter().map(|e| e["handle"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 5, "every occurrence has its own handle");
}

#[tokio::test]
async fn update_event_rejects_an_invalid_scope_before_calling_the_service() {
    let (mock, server) = setup();
    let result =
        call(&server, "update_event", json!({"event_id": "evt2_handle", "title": "X", "scope": "the_whole_universe"}))
            .await;
    assert_eq!(result.is_error, Some(true));
    assert!(result_text(&result).contains("this_occurrence"), "lists allowed scopes: {}", result_text(&result));
    let s = mock.state();
    assert_eq!(s.get_event_called + s.update_event_called, 0);
}

#[tokio::test]
async fn update_event_rejects_series_fields_under_an_occurrence_scope() {
    let (mock, server) = setup();
    for scope in ["this_occurrence", "this_and_future"] {
        let result =
            call(&server, "update_event", json!({"event_id": "evt2_handle", "rrule": "FREQ=WEEKLY", "scope": scope}))
                .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result_text(&result).contains("rrule"), "names the offending field: {}", result_text(&result));
        let result = call(
            &server,
            "update_event",
            json!({"event_id": "evt2_handle", "exdates": ["2026-01-16"], "scope": scope}),
        )
        .await;
        assert!(result_text(&result).contains("exdates"), "{}", result_text(&result));
    }
    assert_eq!(mock.state().get_event_called, 0);
}

#[tokio::test]
async fn delete_event_rejects_an_invalid_scope_before_calling_the_service() {
    let (mock, server) = setup();
    let result = call(&server, "delete_event", json!({"event_id": "evt2_handle", "scope": "nope"})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(result_text(&result).contains("this_occurrence"), "{}", result_text(&result));
    assert_eq!(mock.state().delete_event_called, 0);
}

#[tokio::test]
async fn update_event_threads_this_occurrence_to_the_service() {
    let (mock, server) = setup();
    let events = events_on(&server, "2026-01-16").await;
    let handle = by_uid(&events, "series-1")["handle"].as_str().unwrap().to_string();
    let result =
        call(&server, "update_event", json!({"event_id": handle, "title": "Renamed", "scope": "this_occurrence"}))
            .await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let ics = mock.state().last_updated_ics.clone().unwrap();
    assert!(ics.contains("RECURRENCE-ID:20260116T090000Z"), "an exception for that instance: {ics}");
    assert!(ics.contains("SUMMARY:Renamed"));
    assert!(ics.contains("RRULE:FREQ=DAILY;COUNT=5"), "the master's rule is untouched");
    assert!(result_json(&result)["handle"].as_str().unwrap().starts_with("evt2_"), "an occurrence handle comes back");
}

#[tokio::test]
async fn occurrence_handle_without_scope_is_rejected() {
    let (mock, server) = setup();
    let events = events_on(&server, "2026-01-16").await;
    let handle = by_uid(&events, "series-1")["handle"].as_str().unwrap().to_string();
    let result = call(&server, "update_event", json!({"event_id": handle, "title": "Renamed"})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(result_text(&result).contains("Set scope"), "{}", result_text(&result));
    let deleted = call(&server, "delete_event", json!({"event_id": handle})).await;
    assert_eq!(deleted.is_error, Some(true));
    let s = mock.state();
    assert_eq!(s.update_event_called + s.delete_event_called, 0);
}

#[tokio::test]
async fn delete_event_threads_all_events_to_the_service() {
    let (mock, server) = setup();
    let events = events_on(&server, "2026-01-16").await;
    let handle = by_uid(&events, "series-1")["handle"].as_str().unwrap().to_string();
    let result = call(&server, "delete_event", json!({"event_id": handle, "scope": "all_events"})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let s = mock.state();
    assert_eq!(s.delete_event_called, 1, "all_events deletes the whole resource");
    assert_eq!(s.last_deleted_href.as_deref(), Some("/cal/series-1.ics"));
}

#[tokio::test]
async fn delete_event_this_occurrence_writes_an_exdate() {
    let (mock, server) = setup();
    let events = events_on(&server, "2026-01-17").await;
    let handle = by_uid(&events, "series-1")["handle"].as_str().unwrap().to_string();
    let result = call(&server, "delete_event", json!({"event_id": handle, "scope": "this_occurrence"})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    let s = mock.state();
    assert_eq!(s.delete_event_called, 0, "cancelling one occurrence is a PUT, not a DELETE");
    assert!(s.last_updated_ics.as_ref().unwrap().contains("EXDATE:20260117T090000Z"));
}

#[tokio::test]
async fn scope_on_a_standalone_event_is_rejected_by_the_service() {
    let (_, server) = setup();
    let events = events_on(&server, "2026-01-15").await;
    let handle = by_uid(&events, "single-1")["handle"].as_str().unwrap().to_string();
    let result =
        call(&server, "update_event", json!({"event_id": handle, "title": "X", "scope": "this_occurrence"})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(result_text(&result).contains("needs an occurrence reference"), "{}", result_text(&result));
}
