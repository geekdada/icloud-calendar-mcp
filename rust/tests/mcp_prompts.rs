//! The user-initiated prompts expand to messages that steer the client
//! through the right tool workflow. Port of `PromptsTest.kt`.

use icloud_calendar_mcp::server::prompts::{definitions, render};
use rmcp::model::{GetPromptResult, JsonObject, Role};
use serde_json::{Value, json};

fn args(value: Value) -> JsonObject {
    value.as_object().unwrap().clone()
}

fn text(result: &GetPromptResult) -> String {
    result
        .messages
        .iter()
        .map(|m| m.content.as_text().map(|t| t.text.clone()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

fn prompt(name: &str, arguments: Value) -> GetPromptResult {
    render(name, Some(&args(arguments))).unwrap_or_else(|| panic!("prompt {name} not registered"))
}

#[test]
fn all_three_prompts_are_registered() {
    let names: Vec<String> = definitions().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["schedule_meeting", "reschedule", "find_conflicts"]);
}

#[test]
fn schedule_meeting_declares_its_expected_arguments() {
    let prompt = definitions().into_iter().find(|p| p.name == "schedule_meeting").unwrap();
    let arguments = prompt.arguments.unwrap();
    let names: Vec<&str> = arguments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["title", "attendees", "duration"]);
    assert_eq!(arguments[0].required, Some(true), "title is required");
    assert_eq!(arguments[1].required, Some(false));
    assert!(prompt.description.is_some());
}

#[test]
fn every_prompt_declares_described_arguments() {
    for prompt in definitions() {
        assert!(prompt.description.is_some(), "{}", prompt.name);
        for argument in prompt.arguments.unwrap() {
            assert!(argument.description.is_some(), "{}.{}", prompt.name, argument.name);
        }
    }
}

#[test]
fn schedule_meeting_expands_to_a_create_event_workflow() {
    let result = prompt(
        "schedule_meeting",
        json!({"title": "Design Review", "attendees": "sam@example.com", "duration": "45m"}),
    );
    assert!(!result.messages.is_empty());
    assert_eq!(result.messages[0].role, Role::User);
    let text = text(&result);
    assert!(text.contains("create_event"), "{text}");
    assert!(text.contains("Design Review"));
    assert!(text.contains("sam@example.com"));
    assert!(text.contains("45m"));
    assert_eq!(result.description.as_deref(), Some("Schedule a meeting via create_event"));
}

#[test]
fn reschedule_walks_the_get_events_to_update_event_round_trip() {
    let text = text(&prompt("reschedule", json!({"event": "Design Review", "new_time": "2026-02-01T15:00:00Z"})));
    assert!(text.contains("get_events"), "{text}");
    assert!(text.contains("update_event"));
    assert!(text.contains("handle"));
    assert!(text.contains("2026-02-01T15:00:00Z"));
}

#[test]
fn find_conflicts_expands_to_a_conflict_reasoning_template() {
    let text = text(&prompt("find_conflicts", json!({"date": "2026-02-01"})));
    assert!(text.contains("get_events"), "{text}");
    assert!(text.contains("2026-02-01"));
    assert!(text.to_lowercase().contains("overlap") || text.to_lowercase().contains("conflict"));
}

#[test]
fn missing_optional_arguments_do_not_break_expansion() {
    let result = prompt("schedule_meeting", json!({}));
    let text = text(&result);
    assert!(text.contains("create_event"));
    assert!(text.contains("Untitled meeting"));
    assert!(text.contains("(none specified)"));
    assert!(text.contains("30m"));
    let none = render("schedule_meeting", None).unwrap();
    assert!(!none.messages.is_empty());
}

#[test]
fn blank_arguments_fall_back_to_defaults() {
    let text = text(&prompt("find_conflicts", json!({"date": "   "})));
    assert!(text.contains("(ask the user which day)"), "{text}");
}

#[test]
fn unknown_prompt_renders_nothing() {
    assert!(render("delete_everything", None).is_none());
}
