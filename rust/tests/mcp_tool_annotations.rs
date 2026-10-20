//! Tool annotations match each tool's behavior. Port of `ToolAnnotationsTest.kt`.

use icloud_calendar_mcp::server::tools;
use rmcp::model::ToolAnnotations;

fn annotations(name: &str) -> ToolAnnotations {
    tools::definitions()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{name} tool should exist"))
        .annotations
        .unwrap_or_else(|| panic!("{name} annotations should not be null"))
}

#[test]
fn list_calendars_has_read_only_annotations() {
    let a = annotations("list_calendars");
    assert_eq!(a.read_only_hint, Some(true), "should be read-only");
    assert_eq!(a.destructive_hint, Some(false), "should not be destructive");
    assert_eq!(a.idempotent_hint, Some(true), "should be idempotent");
    assert_eq!(a.open_world_hint, Some(true), "should interact with external world");
}

#[test]
fn get_events_has_read_only_annotations() {
    let a = annotations("get_events");
    assert_eq!(a.read_only_hint, Some(true));
    assert_eq!(a.destructive_hint, Some(false));
    assert_eq!(a.idempotent_hint, Some(true));
    assert_eq!(a.open_world_hint, Some(true));
}

#[test]
fn create_event_has_write_annotations() {
    let a = annotations("create_event");
    assert_eq!(a.read_only_hint, Some(false), "should not be read-only");
    assert_eq!(a.destructive_hint, Some(false), "should not be destructive");
    assert_eq!(a.idempotent_hint, Some(false), "creates a new event each time");
    assert_eq!(a.open_world_hint, Some(true));
}

#[test]
fn update_event_has_idempotent_write_annotations() {
    let a = annotations("update_event");
    assert_eq!(a.read_only_hint, Some(false));
    assert_eq!(a.destructive_hint, Some(false));
    assert_eq!(a.idempotent_hint, Some(true), "same update = same result");
    assert_eq!(a.open_world_hint, Some(true));
}

#[test]
fn delete_event_has_destructive_annotations() {
    let a = annotations("delete_event");
    assert_eq!(a.read_only_hint, Some(false));
    assert_eq!(a.destructive_hint, Some(true), "should be destructive");
    assert_eq!(a.idempotent_hint, Some(true), "delete twice = same result");
    assert_eq!(a.open_world_hint, Some(true));
}
