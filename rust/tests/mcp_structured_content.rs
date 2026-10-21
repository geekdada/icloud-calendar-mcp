//! A tool that declares an outputSchema must return matching structured
//! content on success (strict clients reject the response otherwise), and an
//! error result must set isError and carry no structured content.
//! Port of `StructuredContentTest.kt`, driven through the mock CalDAV client.

mod support;

use icloud_calendar_mcp::caldav::CalDavError;
use icloud_calendar_mcp::server::{CalendarServer, tools};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use support::*;

const CALENDAR_ID: &str = "cal-123";

fn setup() -> (std::sync::Arc<MockCalDavClient>, CalendarServer) {
    let mock = MockCalDavClient::with_calendars(vec![calendar(CALENDAR_ID, "Privat")]);
    mock.state().events_response = vec![caldav_event(
        "event-456",
        &vcalendar(&timed_vevent("event-456", "Team Sync", "20260115T090000Z", "20260115T100000Z")),
    )];
    let (server, _) = server(&mock);
    (mock, server)
}

async fn call(server: &CalendarServer, name: &str, args: Value) -> CallToolResult {
    server.call_tool_by_name(name, args.as_object().cloned()).await.unwrap()
}

/// Every required property is present with its declared JSON type.
fn assert_matches_output_schema(name: &str, result: &CallToolResult) {
    assert_ne!(result.is_error, Some(true), "{name} should have succeeded: {}", result_text(result));
    let structured =
        result.structured_content.as_ref().unwrap_or_else(|| panic!("{name} must return structuredContent"));
    let structured = structured.as_object().expect("structured content is an object");
    let tool = tools::definitions().into_iter().find(|t| t.name == name).unwrap();
    let schema = tool.output_schema.unwrap();
    let properties = schema["properties"].as_object().unwrap();
    for field in schema["required"].as_array().unwrap() {
        assert!(structured.contains_key(field.as_str().unwrap()), "{name} structuredContent should contain {field}");
    }
    for (key, value) in structured {
        match properties.get(key).and_then(|p| p["type"].as_str()) {
            Some("array") => assert!(value.is_array(), "{name}.{key} should be an array"),
            Some("object") => assert!(value.is_object(), "{name}.{key} should be an object"),
            Some("boolean") => assert!(value.is_boolean(), "{name}.{key} should be a boolean"),
            Some("string") => assert!(value.is_string(), "{name}.{key} should be a string"),
            _ => {}
        }
    }
}

fn assert_mirrors_text_content(name: &str, result: &CallToolResult) {
    let text: Value = serde_json::from_str(&result_text(result)).unwrap();
    assert_eq!(Some(&text), result.structured_content.as_ref(), "{name} structuredContent should mirror the text");
}

fn assert_error_has_no_structured_content(name: &str, result: &CallToolResult) {
    assert_eq!(result.is_error, Some(true), "{name} error path should set isError");
    assert_eq!(result.structured_content, None, "{name} error response must not carry structuredContent");
}

#[tokio::test]
async fn list_calendars_returns_matching_structured_content() {
    let (_, server) = setup();
    let result = call(&server, "list_calendars", json!({})).await;
    assert_matches_output_schema("list_calendars", &result);
    assert_mirrors_text_content("list_calendars", &result);
    let calendars = result.structured_content.unwrap()["calendars"].as_array().unwrap().clone();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0]["supportedComponents"], json!(["VEVENT"]));
}

#[tokio::test]
async fn get_events_returns_matching_structured_content() {
    let (_, server) = setup();
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": CALENDAR_ID, "start_date": "2026-01-01", "end_date": "2026-01-31"}),
    )
    .await;
    assert_matches_output_schema("get_events", &result);
    assert_mirrors_text_content("get_events", &result);
    let events = result.structured_content.unwrap()["events"].as_array().unwrap().clone();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["uid"], "event-456");
    assert_eq!(event["summary"], "Team Sync");
    assert_eq!(event["isAllDay"], false);
    assert_eq!(event["startTime"], "2026-01-15T09:00:00Z");
    assert_eq!(event["endTime"], "2026-01-15T10:00:00Z");
    assert!(event["handle"].as_str().unwrap().starts_with("evt1_"));
    assert!(event.get("startDate").is_none(), "timed events carry no startDate");
    assert!(event.get("recurrenceId").is_none());
    assert!(event.get("attendeeCount").is_none(), "zero attendees are omitted");
}

#[tokio::test]
async fn get_events_reports_all_day_dates_and_optional_fields() {
    let mock = MockCalDavClient::with_calendars(vec![calendar(CALENDAR_ID, "Privat")]);
    let body = "BEGIN:VEVENT\r\nUID:a\r\nDTSTART;VALUE=DATE:20260115\r\nDTEND;VALUE=DATE:20260117\r\nSUMMARY:Trip\r\nLOCATION:Paris\r\nSTATUS:TENTATIVE\r\nCATEGORIES:Travel,Fun\r\nPRIORITY:3\r\nURL:https://example.com/trip\r\nORGANIZER;CN=Jane:mailto:jane@example.com\r\nATTENDEE:mailto:a@example.com\r\nEND:VEVENT\r\n";
    mock.state().events_response = vec![caldav_event("a", &vcalendar(body))];
    let (server, _) = server(&mock);
    let result = call(
        &server,
        "get_events",
        json!({"calendar_id": CALENDAR_ID, "start_date": "2026-01-01", "end_date": "2026-01-31"}),
    )
    .await;
    let event = result.structured_content.unwrap()["events"][0].clone();
    assert_eq!(event["isAllDay"], true);
    assert_eq!(event["startDate"], "2026-01-15");
    assert_eq!(event["endDate"], "2026-01-16");
    assert!(event.get("startTime").is_none());
    assert_eq!(event["location"], "Paris");
    assert_eq!(event["status"], "TENTATIVE");
    assert_eq!(event["categories"], json!(["Travel", "Fun"]));
    assert_eq!(event["priority"], 3);
    assert_eq!(event["url"], "https://example.com/trip");
    assert_eq!(event["organizer"], "Jane <jane@example.com>");
    assert_eq!(event["attendeeCount"], 1);
}

#[tokio::test]
async fn create_event_returns_matching_structured_content() {
    let (_, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": CALENDAR_ID, "title": "Team Sync", "start_time": "2026-01-15T09:00:00", "end_time": "2026-01-15T10:00:00"}),
    )
    .await;
    assert_matches_output_schema("create_event", &result);
    assert_mirrors_text_content("create_event", &result);
    let body = result.structured_content.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["summary"], "Team Sync");
    assert_eq!(body["message"], "Event created successfully");
    assert!(body["uid"].as_str().unwrap().ends_with("@icloud-calendar-mcp"));
    assert!(body.get("alarms").is_none(), "alarms omitted when the event has none");
}

#[tokio::test]
async fn create_event_echoes_alarms() {
    let (_, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({
            "calendar_id": CALENDAR_ID, "title": "Team Sync",
            "start_time": "2026-01-15T09:00:00Z", "end_time": "2026-01-15T10:00:00Z",
            "alarms": [{"trigger": "-PT15M"}, {"trigger": "-PT5M", "action": "EMAIL", "summary": "Soon", "repeat_count": 2, "repeat_duration": "PT1M"}]
        }),
    )
    .await;
    assert_matches_output_schema("create_event", &result);
    let alarms = result.structured_content.unwrap()["alarms"].as_array().unwrap().clone();
    assert_eq!(alarms.len(), 2);
    assert_eq!(alarms[0]["trigger"], "-PT15M");
    assert_eq!(alarms[0]["action"], "DISPLAY");
    assert_eq!(alarms[1]["action"], "EMAIL");
    assert_eq!(alarms[1]["summary"], "Soon");
    assert_eq!(alarms[1]["repeat_count"], 2);
    assert_eq!(alarms[1]["repeat_duration"], "PT1M");
}

#[tokio::test]
async fn create_event_accepts_summary_as_alias_for_title() {
    let (mock, server) = setup();
    let result = call(
        &server,
        "create_event",
        json!({"calendar_id": CALENDAR_ID, "summary": "Aliased Title", "start_time": "2026-01-15T09:00:00", "end_time": "2026-01-15T10:00:00"}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    assert!(mock.state().last_created_ics.as_ref().unwrap().contains("SUMMARY:Aliased Title"));
}

#[tokio::test]
async fn title_wins_over_summary_alias() {
    let (mock, server) = setup();
    call(
        &server,
        "create_event",
        json!({"calendar_id": CALENDAR_ID, "title": "Real", "summary": "Alias", "start_time": "2026-01-15T09:00:00", "end_time": "2026-01-15T10:00:00"}),
    )
    .await;
    let ics = mock.state().last_created_ics.clone().unwrap();
    assert!(ics.contains("SUMMARY:Real") && !ics.contains("SUMMARY:Alias"));
}

async fn create_handle(server: &CalendarServer) -> String {
    let created = call(
        server,
        "create_event",
        json!({"calendar_id": CALENDAR_ID, "title": "Team Sync", "start_time": "2026-01-15T09:00:00Z", "end_time": "2026-01-15T10:00:00Z"}),
    )
    .await;
    result_json(&created)["handle"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn update_event_returns_matching_structured_content() {
    let (_, server) = setup();
    let handle = create_handle(&server).await;
    let result = call(&server, "update_event", json!({"event_id": handle, "title": "Team Sync (moved)"})).await;
    assert_matches_output_schema("update_event", &result);
    assert_mirrors_text_content("update_event", &result);
    let body = result.structured_content.unwrap();
    assert_eq!(body["message"], "Event updated successfully");
    assert_ne!(body["handle"], json!(handle), "the refreshed handle carries the new etag");
}

#[tokio::test]
async fn delete_event_returns_matching_structured_content() {
    let (_, server) = setup();
    let handle = create_handle(&server).await;
    let result = call(&server, "delete_event", json!({"event_id": handle})).await;
    assert_matches_output_schema("delete_event", &result);
    assert_mirrors_text_content("delete_event", &result);
    assert_eq!(result.structured_content.unwrap()["success"], true);
}

#[tokio::test]
async fn service_error_paths_omit_structured_content() {
    let (mock, server) = setup();
    mock.state().error_to_return = Some(CalDavError::server(500, "boom"));
    assert_error_has_no_structured_content("list_calendars", &call(&server, "list_calendars", json!({})).await);
    assert_error_has_no_structured_content(
        "get_events",
        &call(
            &server,
            "get_events",
            json!({"calendar_id": CALENDAR_ID, "start_date": "2026-01-01", "end_date": "2026-01-31"}),
        )
        .await,
    );
    assert_error_has_no_structured_content(
        "create_event",
        &call(
            &server,
            "create_event",
            json!({"calendar_id": CALENDAR_ID, "title": "Team Sync", "start_time": "2026-01-15T09:00:00", "end_time": "2026-01-15T10:00:00"}),
        )
        .await,
    );
    assert_error_has_no_structured_content(
        "update_event",
        &call(&server, "update_event", json!({"event_id": "event-456", "title": "Renamed"})).await,
    );
    assert_error_has_no_structured_content(
        "delete_event",
        &call(&server, "delete_event", json!({"event_id": "event-456"})).await,
    );
}

#[tokio::test]
async fn validation_error_paths_omit_structured_content() {
    let (_, server) = setup();
    assert_error_has_no_structured_content(
        "get_events",
        &call(
            &server,
            "get_events",
            json!({"calendar_id": CALENDAR_ID, "start_date": "not-a-date", "end_date": "also-not"}),
        )
        .await,
    );
    assert_error_has_no_structured_content(
        "delete_event",
        &call(&server, "delete_event", json!({"event_id": ""})).await,
    );
}
