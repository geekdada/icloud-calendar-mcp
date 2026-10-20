//! Port of icaldav-core `ICalParserNegativeTest`: invalid input must be
//! rejected or tolerated, never crash. Each case pins the Rust outcome.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ical::{EventStatus, ICalEvent, Transparency};

fn calendar(lines: &str) -> String {
    format!("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\n{}\nEND:VCALENDAR", trim_indent(lines))
}

fn events(lines: &str) -> Vec<ICalEvent> {
    parse_all_events(&calendar(lines)).expect("calendar parses")
}

fn event_with(line: &str) -> Vec<ICalEvent> {
    events(&format!("BEGIN:VEVENT\nUID:test-123\nDTSTART:20231215T100000Z\n{line}\nEND:VEVENT"))
}

#[test]
fn missing_required_properties() {
    let no_uid = events("BEGIN:VEVENT\nDTSTART:20231215T100000Z\nSUMMARY:x\nEND:VEVENT");
    assert!(!no_uid[0].uid.is_empty());
    assert!(events("BEGIN:VEVENT\nUID:x\nSUMMARY:no start\nEND:VEVENT").is_empty());

    let no_version =
        "BEGIN:VCALENDAR\nPRODID:-//Test//EN\nBEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nEND:VEVENT\nEND:VCALENDAR";
    assert_eq!(parse_all_events(no_version).unwrap().len(), 1);
    let no_prodid =
        "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nEND:VEVENT\nEND:VCALENDAR";
    assert_eq!(parse_all_events(no_prodid).unwrap().len(), 1);
}

#[test]
fn broken_structure_is_an_error() {
    let unclosed_calendar = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nEND:VEVENT";
    assert!(parse_all_events(unclosed_calendar).is_err());
    let unclosed_event = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nEND:VCALENDAR";
    assert!(parse_all_events(unclosed_event).is_err());
    let mismatched = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nEND:VTODO\nEND:VCALENDAR";
    assert!(parse_all_events(mismatched).is_err());
}

#[test]
fn property_shape_problems_are_tolerated() {
    assert_eq!(event_with("SUMMARY:")[0].summary.as_deref(), Some(""));
    let no_colon = event_with("SUMMARY Test Event");
    assert_eq!(no_colon[0].summary, None, "a line without ':' is skipped");
}

#[test]
fn non_calendar_input_is_an_error() {
    for input in [
        "",
        "   \n\t\n   ",
        "This is not iCalendar data.\nIt's just plain text.",
        "<!DOCTYPE html>\n<html><body><h1>Not a calendar</h1></body></html>",
        r#"{"events": [{"title": "Test"}]}"#,
        "BEGIN:VCALENDAR\u{0}VERSION:2.0\u{0}END:VCALENDAR",
    ] {
        assert!(parse_all_events(input).is_err(), "{input:?}");
    }
}

#[test]
fn invalid_start_values_drop_the_event() {
    for bad in ["not-a-date", "20231215TABCDEFZ", "20231315T100000Z", "20230230T100000Z"] {
        let parsed = events(&format!("BEGIN:VEVENT\nUID:t\nDTSTART:{bad}\nEND:VEVENT"));
        assert!(parsed.is_empty(), "{bad}");
    }
}

#[test]
fn invalid_property_values_fall_back() {
    assert!(event_with("RRULE:FREQ=INVALID")[0].rrule.is_none());
    assert!(event_with("RRULE:INTERVAL=2;COUNT=10")[0].rrule.is_none());
    assert_eq!(event_with("SEQUENCE:not-a-number")[0].sequence, 0);
    assert_eq!(event_with("SEQUENCE:-5")[0].sequence, -5);
    assert_eq!(event_with("STATUS:INVALID-STATUS")[0].status, Some(EventStatus::Confirmed));
    assert_eq!(event_with("TRANSP:MAYBE")[0].transparency, Transparency::Opaque);
    let bad_zone = events("BEGIN:VEVENT\nUID:t\nDTSTART;TZID=Invalid/Timezone:20231215T100000\nEND:VEVENT");
    assert_eq!(bad_zone.len(), 1);
}

#[test]
fn hostile_inputs_do_not_crash() {
    let long = "A".repeat(100_000);
    assert_eq!(event_with(&format!("SUMMARY:{long}"))[0].summary.as_deref(), Some(long.as_str()));

    let mut nested = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
    nested.push_str(&"BEGIN:VEVENT\n".repeat(100));
    nested.push_str("UID:test-123\nDTSTART:20231215T100000Z\n");
    nested.push_str(&"END:VEVENT\n".repeat(100));
    nested.push_str("END:VCALENDAR\n");
    assert!(parse_all_events(&nested).is_ok());

    assert_eq!(event_with("SUMMARY:Test \u{7} Event")[0].summary.as_deref(), Some("Test \u{7} Event"));
    assert_eq!(event_with("SUMMARY:Test 👨‍👩‍👧‍👦 Family")[0].summary.as_deref(), Some("Test 👨‍👩‍👧‍👦 Family"));

    let escapes = event_with("SUMMARY:Test\\nwith\\nnewlines\nDESCRIPTION:Backslash\\\\ and comma\\, and semicolon\\;");
    assert_eq!(escapes[0].summary.as_deref(), Some("Test\nwith\nnewlines"));
    assert_eq!(escapes[0].description.as_deref(), Some("Backslash\\ and comma, and semicolon;"));
}

#[test]
fn conflicting_properties() {
    let both = event_with("DTEND:20231215T110000Z\nDURATION:PT2H");
    assert!(both[0].dt_end.is_some() && both[0].duration.is_some());
    assert!(!generate(&both[0]).contains("DURATION:"), "DTEND wins on output");

    let rule = event_with("RRULE:FREQ=DAILY;COUNT=10;UNTIL=20240101T000000Z")[0].rrule.clone().unwrap();
    assert_eq!(rule.count, Some(10));
    assert!(rule.until.is_some());

    let duplicates = events(
        "BEGIN:VEVENT\nUID:dup\nDTSTART:20231215T100000Z\nSUMMARY:First\nEND:VEVENT\nBEGIN:VEVENT\nUID:dup\nDTSTART:20231216T100000Z\nSUMMARY:Second\nEND:VEVENT",
    );
    assert_eq!(duplicates.len(), 2);
}
