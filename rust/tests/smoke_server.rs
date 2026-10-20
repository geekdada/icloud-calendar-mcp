//! End-to-end smoke test through the server and service over the mock client.

mod support;

use serde_json::json;
use support::*;

#[tokio::test]
async fn create_list_update_delete_round_trip() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let (server, sink) = server(&mock);

    let created = server
        .call_tool_by_name(
            "create_event",
            Some(json!({"calendar_id": "home", "title": "Standup", "start_time": "2025-01-15T09:00:00Z", "end_time": "2025-01-15T09:15:00Z"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(created.is_error, Some(true), "{}", result_text(&created));
    let body = result_json(&created);
    assert_eq!(body["success"], true);
    assert_eq!(body["summary"], "Standup");
    let handle = body["handle"].as_str().unwrap().to_string();
    assert!(handle.starts_with("evt1_"));
    assert_eq!(created.structured_content.as_ref().unwrap(), &body);

    let updated = server
        .call_tool_by_name(
            "update_event",
            Some(json!({"event_id": handle, "title": "Daily standup"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(updated.is_error, Some(true), "{}", result_text(&updated));
    assert!(mock.state().last_updated_ics.as_ref().unwrap().contains("SUMMARY:Daily standup"));
    let new_handle = result_json(&updated)["handle"].as_str().unwrap().to_string();

    let deleted = server
        .call_tool_by_name("delete_event", Some(json!({"event_id": new_handle}).as_object().unwrap().clone()))
        .await
        .unwrap();
    assert_eq!(result_json(&deleted)["success"], true);
    assert_eq!(sink.count(), 3, "create, update and delete are audited");
}
