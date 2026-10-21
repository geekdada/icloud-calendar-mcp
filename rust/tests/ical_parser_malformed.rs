//! Port of icaldav-core `ICalParserMalformedInputTest`.
//!
//! The Kotlin cases mostly assert "does not crash"; here each also pins the
//! concrete Rust outcome. `importId` is not modelled; COLOR is a raw line.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ical::timezone::system_zone;

fn one_event(body: &str) -> Vec<icloud_calendar_mcp::ical::ICalEvent> {
    parse_all_events(&vcal(body)).expect("calendar parses")
}

#[test]
fn missing_uid_gets_a_generated_one() {
    let events =
        one_event("BEGIN:VEVENT\nDTSTART:20231215T100000Z\nDTEND:20231215T110000Z\nSUMMARY:No UID\nEND:VEVENT");
    assert_eq!(events.len(), 1);
    assert!(!events[0].uid.is_empty());
}

#[test]
fn missing_dtstart_falls_back_to_dtend() {
    let events = one_event("BEGIN:VEVENT\nUID:test-uid-123\nDTEND:20231215T110000Z\nSUMMARY:x\nEND:VEVENT");
    assert_eq!(events[0].dt_start.to_ical_string(), "20231215T110000Z");
}

#[test]
fn uid_and_dtstart_are_enough() {
    let events = one_event("BEGIN:VEVENT\nUID:minimal-event\nDTSTART:20231215T100000Z\nEND:VEVENT");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].uid, "minimal-event");
}

#[test]
fn garbage_datetime_drops_only_that_event() {
    let events = one_event(
        "BEGIN:VEVENT\nUID:bad\nDTSTART:not-a-date\nSUMMARY:Bad\nEND:VEVENT\nBEGIN:VEVENT\nUID:good\nDTSTART:20231215T100000Z\nEND:VEVENT",
    );
    assert_eq!(events.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), ["good"]);
}

#[test]
fn date_values_are_all_day_with_or_without_value_param() {
    assert!(one_event("BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20231215\nEND:VEVENT")[0].is_all_day);
    let bare = &one_event("BEGIN:VEVENT\nUID:b\nDTSTART:20231215\nEND:VEVENT")[0];
    assert!(bare.is_all_day);
    assert!(bare.dt_start.is_date);
}

#[test]
fn invalid_timezone_falls_back_to_system_zone() {
    let events = one_event("BEGIN:VEVENT\nUID:t\nDTSTART;TZID=Invalid/Timezone:20231215T100000\nEND:VEVENT");
    assert_eq!(events[0].dt_start.timezone, Some(system_zone()));
}

#[test]
fn folded_long_and_special_text() {
    let folded =
        one_event("BEGIN:VEVENT\nUID:f\nDTSTART:20231215T100000Z\nSUMMARY:Line 1\n  continuation line\nEND:VEVENT");
    assert_eq!(folded[0].summary.as_deref(), Some("Line 1 continuation line"));

    let long = "A".repeat(100_000);
    let events = one_event(&format!("BEGIN:VEVENT\nUID:l\nDTSTART:20231215T100000Z\nDESCRIPTION:{long}\nEND:VEVENT"));
    assert_eq!(events[0].description.as_deref(), Some(long.as_str()));

    let special = one_event(
        "BEGIN:VEVENT\nUID:s\nDTSTART:20231215T100000Z\nLOCATION:Café \"Main\"\\, Room 123\\; Building A\nEND:VEVENT",
    );
    assert_eq!(special[0].location.as_deref(), Some("Café \"Main\", Room 123; Building A"));
}

#[test]
fn null_bytes_do_not_crash() {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:test\r\nDTSTART:20231215T100000Z\r\nSUMMARY:Test\u{0}Event\r\nEND:VEVENT\r\nEND:VCALENDAR";
    let events = parse_all_events(ics).unwrap();
    assert_eq!(events[0].summary.as_deref(), Some("Test\u{0}Event"));
}

#[test]
fn malformed_rrules_are_ignored_or_inert() {
    for rule in ["INTERVAL=1", "FREQ=INVALID"] {
        let events = one_event(&format!("BEGIN:VEVENT\nUID:r\nDTSTART:20231215T100000Z\nRRULE:{rule}\nEND:VEVENT"));
        assert!(events[0].rrule.is_none(), "{rule}");
    }
    let negative = one_event("BEGIN:VEVENT\nUID:r\nDTSTART:20231215T100000Z\nRRULE:FREQ=DAILY;COUNT=-5\nEND:VEVENT");
    assert_eq!(negative[0].rrule.as_ref().unwrap().count, Some(-5));
}

#[test]
fn malformed_valarms() {
    let missing_trigger = one_event(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nBEGIN:VALARM\nACTION:DISPLAY\nDESCRIPTION:R\nEND:VALARM\nEND:VEVENT",
    );
    assert_eq!(
        missing_trigger[0].alarms[0].trigger,
        Some(chrono::TimeDelta::minutes(-15)),
        "defaults to 15 min before"
    );
    let invalid_trigger = one_event(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20231215T100000Z\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:INVALID\nDESCRIPTION:R\nEND:VALARM\nEND:VEVENT",
    );
    assert!(invalid_trigger[0].alarms.is_empty(), "an unreadable trigger drops the alarm, not the event");
}

#[test]
fn structure_issues() {
    let unclosed = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:x\nDTSTART:20231215T100000Z\nEND:VCALENDAR";
    assert!(parse_all_events(unclosed).is_err());

    let nested = one_event(
        "BEGIN:VEVENT\nUID:outer\nDTSTART:20231215T100000Z\nBEGIN:VEVENT\nUID:inner\nDTSTART:20231216T100000Z\nEND:VEVENT\nEND:VEVENT",
    );
    assert_eq!(nested.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), ["outer"]);

    assert!(parse_all_events("BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR").unwrap().is_empty());

    let two = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:event1\nDTSTART:20231215T100000Z\nEND:VEVENT\nEND:VCALENDAR\nBEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:event2\nDTSTART:20231216T100000Z\nEND:VEVENT\nEND:VCALENDAR";
    let events = parse_all_events(two).unwrap();
    assert_eq!(events[0].uid, "event1", "the first calendar is read");
}

#[test]
fn utf8_bom_is_ignored() {
    let ics = format!("\u{feff}{}", vcal("BEGIN:VEVENT\nUID:bom\nDTSTART:20231215T100000Z\nEND:VEVENT"));
    let events = parse_all_events(&ics).unwrap();
    assert_eq!(events[0].uid, "bom");
}

#[test]
fn mixed_line_endings() {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\nPRODID:-//Test//EN\rBEGIN:VEVENT\r\nUID:test\nDTSTART:20231215T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR";
    assert_eq!(parse_all_events(ics).unwrap()[0].uid, "test");
}

#[test]
fn production_edge_cases() {
    let events = one_event(
        "BEGIN:VEVENT\nUID:recurring-event\nDTSTART:20231215T100000Z\nDTEND:20231215T110000Z\nRRULE:FREQ=DAILY;COUNT=5\nSUMMARY:Daily\nEND:VEVENT\nBEGIN:VEVENT\nUID:recurring-event\nRECURRENCE-ID:20231216T100000Z\nDTSTART:20231216T140000Z\nDTEND:20231216T150000Z\nSUMMARY:Moved\nEND:VEVENT",
    );
    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|e| e.recurrence_id.is_none()));
    assert!(events.iter().any(|e| e.recurrence_id.is_some() && e.uid == "recurring-event"));

    let exdates = one_event(
        "BEGIN:VEVENT\nUID:e\nDTSTART:20231215T100000Z\nRRULE:FREQ=DAILY;COUNT=10\nEXDATE:20231217T100000Z\nEXDATE:20231220T100000Z\nEND:VEVENT",
    );
    assert_eq!(exdates[0].exdates.len(), 2);

    let color = one_event("BEGIN:VEVENT\nUID:c\nDTSTART:20231215T100000Z\nCOLOR:red\nEND:VEVENT");
    assert_eq!(raw(&color[0], "COLOR").unwrap().value, "red");

    let cats = one_event("BEGIN:VEVENT\nUID:b\nDTSTART:20231215T100000Z\nCATEGORIES:foo,,bar,\nEND:VEVENT");
    assert_eq!(cats[0].categories, ["foo", "bar"]);
}
