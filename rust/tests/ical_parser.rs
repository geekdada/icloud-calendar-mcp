//! Port of icaldav-core `ICalParserTest`.
//!
//! Not ported: `importId` (not modelled) and `extractUid` (lives in the CalDAV
//! XML parser, covered by its own suite). GEO is kept as a raw line.

mod ical_support;

use chrono::TimeDelta;
use ical_support::*;
use icloud_calendar_mcp::ical::{AlarmAction, Frequency};

#[test]
fn parse_simple_event() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:test-event-123
        DTSTAMP:20231215T100000Z
        DTSTART:20231215T140000Z
        DTEND:20231215T150000Z
        SUMMARY:Test Event
        DESCRIPTION:This is a test description
        LOCATION:Test Location
        END:VEVENT",
    ));
    assert_eq!(event.uid, "test-event-123");
    assert_eq!(event.summary.as_deref(), Some("Test Event"));
    assert_eq!(event.description.as_deref(), Some("This is a test description"));
    assert_eq!(event.location.as_deref(), Some("Test Location"));
    assert!(!event.is_all_day);
}

#[test]
fn parse_all_day_event() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:allday-event-123
        DTSTAMP:20231215T100000Z
        DTSTART;VALUE=DATE:20231215
        DTEND;VALUE=DATE:20231216
        SUMMARY:All Day Event
        END:VEVENT",
    ));
    assert!(event.is_all_day);
    assert_eq!(event.dt_start.to_day_code(), "20231215");
}

#[test]
fn parse_event_with_rrule() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:recurring-event-123
        DTSTAMP:20231215T100000Z
        DTSTART:20231215T140000Z
        DTEND:20231215T150000Z
        SUMMARY:Weekly Meeting
        RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=10
        END:VEVENT",
    ));
    let rrule = event.rrule.unwrap();
    assert_eq!(rrule.freq, Frequency::Weekly);
    assert_eq!(rrule.count, Some(10));
    assert_eq!(rrule.by_day.unwrap().len(), 3);
}

#[test]
fn parse_event_with_recurrence_id() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:recurring-123
        DTSTAMP:20231215T100000Z
        RECURRENCE-ID:20231208T140000Z
        DTSTART:20231209T150000Z
        DTEND:20231209T160000Z
        SUMMARY:Modified Meeting
        END:VEVENT",
    ));
    assert_eq!(event.uid, "recurring-123");
    assert_eq!(event.recurrence_id.unwrap().to_ical_string(), "20231208T140000Z");
}

#[test]
fn parse_event_with_exdate() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:recurring-123
        DTSTAMP:20231215T100000Z
        DTSTART:20231201T140000Z
        RRULE:FREQ=DAILY;COUNT=30
        EXDATE:20231208T140000Z,20231215T140000Z
        SUMMARY:Daily Meeting
        END:VEVENT",
    ));
    assert_eq!(event.exdates.len(), 2);
}

#[test]
fn parse_event_with_valarm() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:event-with-alarm
        DTSTAMP:20231215T100000Z
        DTSTART:20231215T140000Z
        SUMMARY:Meeting with Reminder
        BEGIN:VALARM
        ACTION:DISPLAY
        TRIGGER:-PT15M
        DESCRIPTION:Event reminder
        END:VALARM
        END:VEVENT",
    ));
    assert_eq!(event.alarms.len(), 1);
    assert_eq!(event.alarms[0].action, AlarmAction::Display);
    assert_eq!(event.alarms[0].trigger, Some(TimeDelta::minutes(-15)));
}

#[test]
fn parse_multiple_events_in_single_file() {
    let events = parse(&vcal(
        "
        BEGIN:VEVENT
        UID:event-1
        DTSTAMP:20231215T100000Z
        DTSTART:20231215T140000Z
        SUMMARY:Event One
        END:VEVENT
        BEGIN:VEVENT
        UID:event-2
        DTSTAMP:20231215T100000Z
        DTSTART:20231216T140000Z
        SUMMARY:Event Two
        END:VEVENT",
    ));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].uid, "event-1");
    assert_eq!(events[1].uid, "event-2");
}

#[test]
fn parse_escaped_text_correctly() {
    let event = parse_one(&vcal(
        r"
        BEGIN:VEVENT
        UID:escaped-event
        DTSTAMP:20231215T100000Z
        DTSTART:20231215T140000Z
        SUMMARY:Meeting\, Important
        DESCRIPTION:Line 1\nLine 2\nLine 3
        END:VEVENT",
    ));
    assert_eq!(event.summary.as_deref(), Some("Meeting, Important"));
    assert_eq!(event.description.as_deref(), Some("Line 1\nLine 2\nLine 3"));
}

fn event_with(extra: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\nUID:p\nDTSTAMP:20231215T100000Z\nDTSTART:20231215T140000Z\n{extra}\nSUMMARY:x\nEND:VEVENT"
    ))
}

#[test]
fn priority_values() {
    assert_eq!(parse_one(&event_with("PRIORITY:1")).priority, 1);
    assert_eq!(parse_one(&event_with("PRIORITY:9")).priority, 9);
    assert_eq!(parse_one(&event_with("X-NONE:1")).priority, 0);
    assert_eq!(parse_one(&event_with("PRIORITY:15")).priority, 9, "coerced to max");
    assert_eq!(parse_one(&event_with("PRIORITY:-3")).priority, 0, "coerced to min");
    assert_eq!(parse_one(&event_with("PRIORITY:high")).priority, 0, "unreadable is undefined");
}

#[test]
fn geo_is_preserved_verbatim() {
    let event = parse_one(&event_with("GEO:37.386013;-122.082932"));
    assert_eq!(raw(&event, "GEO").unwrap().value, "37.386013;-122.082932");
    let negative = parse_one(&event_with("GEO:-33.8688;151.2093"));
    assert_eq!(raw(&negative, "GEO").unwrap().value, "-33.8688;151.2093");
    assert!(raw(&parse_one(&event_with("X-A:1")), "GEO").is_none());
    assert!(generate(&negative).contains("GEO:-33.8688;151.2093\r\n"));
}
