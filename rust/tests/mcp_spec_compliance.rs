//! MCP spec compliance of the server surface: tool titles and output schemas,
//! the list_calendars input schema, the calendar resource metadata, the
//! resource error codes, and the advertised capabilities.
//! Port of `McpSpecComplianceTest.kt`.

mod support;

use std::sync::Arc;

use icloud_calendar_mcp::caldav::CalDavError;
use icloud_calendar_mcp::logging::{McpLogger, NullSink};
use icloud_calendar_mcp::ratelimit::RateLimiter;
use icloud_calendar_mcp::server::{CALENDARS_URI, CalendarServer, tools};
use rmcp::ServerHandler;
use rmcp::model::{ErrorCode, ResourceContents, Tool};
use serde_json::Value;
use support::*;

const TOOL_NAMES: [&str; 5] = ["list_calendars", "get_events", "create_event", "update_event", "delete_event"];

fn tool(name: &str) -> Tool {
    tools::definitions().into_iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name} not registered"))
}

fn output_props(name: &str) -> serde_json::Map<String, Value> {
    tool(name).output_schema.unwrap()["properties"].as_object().unwrap().clone()
}

fn output_required(name: &str) -> Vec<String> {
    tool(name).output_schema.unwrap()["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn registers_exactly_the_five_tools_in_order() {
    let names: Vec<String> = tools::definitions().iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, TOOL_NAMES);
}

#[test]
fn all_tools_have_human_readable_title() {
    for name in TOOL_NAMES {
        let title = tool(name).title.unwrap_or_else(|| panic!("{name} should have a title"));
        assert!(!title.trim().is_empty(), "{name} title should not be blank");
    }
}

#[test]
fn tool_titles_are_distinct_from_tool_names() {
    for name in TOOL_NAMES {
        assert_ne!(tool(name).title.as_deref(), Some(name), "{name} title should differ from its name");
    }
}

#[test]
fn all_tools_have_descriptions() {
    for name in TOOL_NAMES {
        assert!(tool(name).description.is_some_and(|d| !d.is_empty()), "{name} description");
    }
}

#[test]
fn list_calendars_has_minimal_input_schema() {
    let schema = tool("list_calendars").input_schema;
    assert_eq!(schema["type"], "object");
    let props = schema.get("properties").and_then(Value::as_object);
    assert!(props.is_none_or(|p| p.is_empty()), "list_calendars inputSchema should have no properties: {props:?}");
}

#[test]
fn input_schemas_declare_required_fields() {
    let required = |name: &str| -> Vec<String> {
        tool(name)
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |r| r.iter().map(|v| v.as_str().unwrap().to_string()).collect())
    };
    assert_eq!(required("get_events"), ["calendar_id", "start_date", "end_date"]);
    assert_eq!(required("create_event"), ["calendar_id", "title"]);
    assert_eq!(required("update_event"), ["event_id"]);
    assert_eq!(required("delete_event"), ["event_id"]);
}

#[test]
fn every_required_input_is_a_declared_property() {
    for t in tools::definitions() {
        let props = t.input_schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
        for field in t.input_schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            assert!(props.contains_key(field.as_str().unwrap()), "{}: required {field} is not a property", t.name);
        }
    }
}

#[test]
fn all_tools_have_output_schema() {
    for name in TOOL_NAMES {
        let schema = tool(name).output_schema.unwrap_or_else(|| panic!("{name} should have outputSchema"));
        assert_eq!(schema["type"], "object");
    }
}

#[test]
fn output_schema_properties_are_non_empty() {
    for name in TOOL_NAMES {
        assert!(!output_props(name).is_empty(), "{name} outputSchema properties should not be empty");
    }
}

#[test]
fn output_schema_declares_required_fields() {
    for name in TOOL_NAMES {
        assert!(!output_required(name).is_empty(), "{name} outputSchema required list should not be empty");
    }
}

#[test]
fn list_calendars_output_schema_has_calendars_array() {
    assert_eq!(output_props("list_calendars")["calendars"]["type"], "array");
    assert_eq!(output_required("list_calendars"), ["calendars"]);
}

#[test]
fn get_events_output_schema_has_events_array() {
    assert_eq!(output_props("get_events")["events"]["type"], "array");
    assert_eq!(output_required("get_events"), ["events"]);
}

#[test]
fn create_event_output_schema_has_success_uid_summary_message() {
    let props = output_props("create_event");
    let required = output_required("create_event");
    for field in ["success", "uid", "summary", "message"] {
        assert!(props.contains_key(field), "create_event outputSchema should have {field}");
        assert!(required.iter().any(|r| r == field), "create_event {field} should be required");
    }
    assert!(props.contains_key("handle") && props.contains_key("alarms"));
}

#[test]
fn update_event_output_schema_has_success_uid_message() {
    let props = output_props("update_event");
    let required = output_required("update_event");
    for field in ["success", "uid", "message"] {
        assert!(props.contains_key(field));
        assert!(required.iter().any(|r| r == field));
    }
}

#[test]
fn delete_event_output_schema_has_success_and_message() {
    let props = output_props("delete_event");
    let required = output_required("delete_event");
    for field in ["success", "message"] {
        assert!(props.contains_key(field), "delete_event outputSchema should have {field}");
        assert!(required.iter().any(|r| r == field), "delete_event {field} should be required");
    }
}

#[test]
fn scope_property_enumerates_the_scope_tokens() {
    for name in ["update_event", "delete_event"] {
        let scope = &tool(name).input_schema["properties"]["scope"];
        assert_eq!(scope["enum"], serde_json::json!(["this_occurrence", "this_and_future", "all_events"]), "{name}");
    }
}

#[test]
fn alarm_items_require_a_trigger() {
    for name in ["create_event", "update_event"] {
        let items = &tool(name).input_schema["properties"]["alarms"]["items"];
        assert_eq!(items["required"], serde_json::json!(["trigger"]), "{name}");
        assert_eq!(items["properties"]["repeat_count"]["type"], "integer");
    }
}

#[test]
fn rrule_description_does_not_mention_ical4j() {
    for name in ["create_event", "update_event"] {
        let description = tool(name).input_schema["properties"]["rrule"]["description"].as_str().unwrap().to_string();
        assert!(description.contains("WKST"), "{name}: {description}");
        assert!(!description.contains("ical4j"), "{name}: {description}");
    }
}

// ───────────────────────────── resources ─────────────────────────────

#[test]
fn calendar_resource_is_registered_with_title() {
    let resources = CalendarServer::resources();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].title.as_deref(), Some("iCloud Calendars"));
}

#[test]
fn calendar_resource_has_correct_metadata() {
    let resource = &CalendarServer::resources()[0];
    assert_eq!(resource.uri, "calendar://calendars");
    assert_eq!(resource.uri, CALENDARS_URI);
    assert_eq!(resource.name, "iCloud Calendars");
    assert_eq!(resource.mime_type.as_deref(), Some("application/json"));
    assert!(resource.description.is_some());
}

fn server_with(service_mock: Option<&Arc<MockCalDavClient>>, read_limit: u32) -> CalendarServer {
    CalendarServer::with_parts(
        service_mock.map(service),
        Arc::new(RateLimiter::new(read_limit, 20, 60_000)),
        Arc::new(McpLogger::new(Arc::new(NullSink))),
    )
}

#[tokio::test]
async fn reading_the_resource_returns_calendar_json() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home"), read_only_calendar("shared", "Shared")]);
    let result = server_with(Some(&mock), 60).read_resource_uri(CALENDARS_URI).await.unwrap();
    let ResourceContents::TextResourceContents { uri, mime_type, text, .. } = &result.contents[0] else {
        panic!("expected text contents");
    };
    assert_eq!(uri, CALENDARS_URI);
    assert_eq!(mime_type.as_deref(), Some("application/json"));
    let body: Value = serde_json::from_str(text).unwrap();
    let calendars = body["calendars"].as_array().unwrap();
    assert_eq!(calendars.len(), 2);
    assert_eq!(calendars[0]["id"], "home");
    assert_eq!(calendars[0]["uri"], "calendar://calendars/home");
    assert_eq!(calendars[0]["color"], "#FF0000");
    assert_eq!(calendars[0]["readOnly"], false);
    assert_eq!(calendars[1]["readOnly"], true);
}

#[tokio::test]
async fn resource_rate_limit_error_uses_code_32000() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let server = server_with(Some(&mock), 1);
    server.read_resource_uri(CALENDARS_URI).await.unwrap();
    let error = server.read_resource_uri(CALENDARS_URI).await.unwrap_err();
    assert_eq!(error.code, ErrorCode(-32000));
    assert!(error.message.contains("Rate limit exceeded"), "{}", error.message);
}

#[tokio::test]
async fn resource_without_credentials_uses_code_32001() {
    let error = server_with(None, 60).read_resource_uri(CALENDARS_URI).await.unwrap_err();
    assert_eq!(error.code, ErrorCode(-32001));
    assert_eq!(error.message, "iCloud credentials not configured");
}

#[tokio::test]
async fn resource_service_failure_uses_code_32002() {
    let mock = MockCalDavClient::new();
    mock.state().list_calendars_result = Some(Err(CalDavError::server(503, "Service unavailable")));
    let error = server_with(Some(&mock), 60).read_resource_uri(CALENDARS_URI).await.unwrap_err();
    assert_eq!(error.code, ErrorCode(-32002));
    assert!(error.message.starts_with("Calendar service error: "), "{}", error.message);
}

#[tokio::test]
async fn resource_error_codes_are_distinct() {
    let codes = [-32000, -32001, -32002];
    let unique: std::collections::HashSet<_> = codes.iter().collect();
    assert_eq!(unique.len(), 3);
}

#[tokio::test]
async fn unknown_resource_is_resource_not_found() {
    let error = server_with(None, 60).read_resource_uri("calendar://other").await.unwrap_err();
    assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
}

#[tokio::test]
async fn unknown_tool_is_invalid_params() {
    let error = server_with(None, 60).call_tool_by_name("drop_tables", None).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(error.message.contains("drop_tables"));
}

// ───────────────────────────── capabilities ─────────────────────────────

#[test]
fn advertises_tools_resources_prompts_and_logging() {
    let info = server_with(None, 60).get_info();
    let caps = serde_json::to_value(&info.capabilities).unwrap();
    assert_eq!(caps["tools"]["listChanged"], true);
    assert_eq!(caps["resources"]["listChanged"], true);
    assert!(caps.get("prompts").is_some());
    assert!(caps.get("logging").is_some());
    assert_eq!(info.server_info.name, "icloud-calendar-mcp");
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
}
