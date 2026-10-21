//! The get_events span cap is enforced in the tool handler, before any CalDAV
//! call. Port of `GetEventsSpanCapWiringTest.kt`: the mock counts calls, so a
//! missing validation wire would show up as a get_events call.

mod support;

use serde_json::json;
use support::*;

async fn call(args: serde_json::Value) -> (rmcp::model::CallToolResult, u32) {
    let mock = MockCalDavClient::new();
    let (server, _) = server(&mock);
    let result = server.call_tool_by_name("get_events", args.as_object().cloned()).await.unwrap();
    let calls = mock.state().get_events_called;
    (result, calls)
}

#[tokio::test]
async fn rejects_an_over_wide_range_before_calling_the_service() {
    let (result, calls) =
        call(json!({"calendar_id": "cal-1", "start_date": "2026-01-01", "end_date": "2027-06-01"})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(result_text(&result).contains("exceeds the maximum"), "{}", result_text(&result));
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn rejects_an_inverted_range_before_calling_the_service() {
    let (result, calls) =
        call(json!({"calendar_id": "cal-1", "start_date": "2026-06-01", "end_date": "2026-05-01"})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(result_text(&result).contains("must not precede"), "{}", result_text(&result));
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn accepts_a_full_leap_year() {
    let (result, calls) =
        call(json!({"calendar_id": "cal-1", "start_date": "2024-01-01", "end_date": "2025-01-01"})).await;
    assert_ne!(result.is_error, Some(true), "{}", result_text(&result));
    assert_eq!(calls, 1);
}
