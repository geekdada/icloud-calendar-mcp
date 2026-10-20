//! Port of icaldav-core `ICalParserParsingGapsPreTest`.
//!
//! The Kotlin suite disables "good event survives when sibling VEVENT has bad
//! property" because ical4j's stream parser dies on an unclosed quoted
//! parameter; the Rust parser handles it, so the case runs here.

mod ical_support;

use chrono::TimeDelta;
use ical_support::*;

fn single(lines: &str) -> String {
    vcal(&format!("BEGIN:VEVENT\n{}\nEND:VEVENT", trim_indent(lines)))
}

#[test]
fn folded_x_property_with_odd_parameters_does_not_break_the_event() {
    let e = parse_one(&single(
        "
        UID:good-event-123
        DTSTART:20231215T100000Z
        DTEND:20231215T110000Z
        SUMMARY:Team Meeting
        X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-APPLE-RADIUS=70.5
         ;X-TITLE=\"Home\":geo:37.332,-122.031",
    ));
    assert_eq!(e.uid, "good-event-123");
    assert_eq!(e.summary.as_deref(), Some("Team Meeting"));
    let location = raw(&e, "X-APPLE-STRUCTURED-LOCATION").unwrap();
    assert_eq!(location.param("X-TITLE"), Some("Home"));
    assert_eq!(location.value, "geo:37.332,-122.031");
}

#[test]
fn good_event_survives_sibling_with_unclosed_quoted_parameter() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:clean-event\nDTSTART:20231215T100000Z\nSUMMARY:Clean Event\nEND:VEVENT\nBEGIN:VEVENT\nUID:messy-event\nDTSTART:20231216T100000Z\nSUMMARY:Messy Event\nX-CUSTOM-BAD;PARAM=\":broken-value\nEND:VEVENT",
    ));
    let uids: Vec<_> = events.iter().map(|e| e.uid.as_str()).collect();
    assert_eq!(uids, ["clean-event", "messy-event"]);
}

#[test]
fn invalid_absolute_alarm_trigger_drops_only_the_alarm() {
    let e = parse_one(&single(
        "
        UID:event-with-bad-alarm
        DTSTART:20231215T100000Z
        SUMMARY:Has Bad Alarm
        BEGIN:VALARM
        ACTION:DISPLAY
        TRIGGER;VALUE=DATE-TIME:INVALID
        DESCRIPTION:Reminder
        END:VALARM",
    ));
    assert_eq!(e.uid, "event-with-bad-alarm");
    assert!(e.alarms.is_empty());
}

#[test]
fn attach_with_unusual_parameters_is_kept_verbatim() {
    let e = parse_one(&single(
        "
        UID:nc-event-456
        DTSTART:20231215T100000Z
        SUMMARY:Team Meeting
        ATTACH;FMTTYPE=application/octet-stream;FILENAME=notes.txt;SIZE=1234:https://cloud.example.com/remote.php/dav/files/user/notes.txt",
    ));
    assert_eq!(e.summary.as_deref(), Some("Team Meeting"));
    let attach = raw(&e, "ATTACH").unwrap();
    assert_eq!(attach.param("FILENAME"), Some("notes.txt"));
    assert!(generate(&e).contains("FILENAME=notes.txt"));
}

#[test]
fn missing_uid_is_generated_and_unique() {
    let one = parse_one(&single("DTSTART:20231215T100000Z\nDTEND:20231215T110000Z\nSUMMARY:Event Without UID"));
    assert!(!one.uid.is_empty());

    let two = parse(&vcal(
        "BEGIN:VEVENT\nDTSTART:20231215T100000Z\nSUMMARY:First No-UID\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20231216T100000Z\nSUMMARY:Second No-UID\nEND:VEVENT",
    ));
    assert_eq!(two.len(), 2);
    assert_ne!(two[0].uid, two[1].uid);
}

#[test]
fn missing_dtstart_falls_back_to_dtend() {
    let e = parse_one(&single("UID:no-dtstart-event\nDTEND:20231215T110000Z\nSUMMARY:No Start Time"));
    assert_eq!(e.dt_start.to_ical_string(), "20231215T110000Z");
}

#[test]
fn inverted_start_and_end_are_swapped() {
    let e = parse_one(&single("UID:swapped-times\nDTSTART:20231215T150000Z\nDTEND:20231215T100000Z"));
    assert_eq!(e.dt_start.to_ical_string(), "20231215T100000Z");
    assert_eq!(e.dt_end.unwrap().to_ical_string(), "20231215T150000Z");
}

#[test]
fn exception_rrule_is_stripped() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:recurring-master\nDTSTART:20231215T100000Z\nRRULE:FREQ=DAILY;COUNT=5\nEND:VEVENT\nBEGIN:VEVENT\nUID:recurring-master\nRECURRENCE-ID:20231216T100000Z\nDTSTART:20231216T140000Z\nRRULE:FREQ=DAILY;COUNT=5\nEND:VEVENT",
    ));
    assert_eq!(events.len(), 2);
    let exception = events.iter().find(|e| e.recurrence_id.is_some()).unwrap();
    assert!(exception.rrule.is_none());
}

#[test]
fn three_digit_utc_offset_in_vtimezone_is_tolerated() {
    let events = parse(
        "
        BEGIN:VCALENDAR
        VERSION:2.0
        PRODID:-//Synology//CalDAV//EN
        BEGIN:VTIMEZONE
        TZID:Asia/Kolkata
        BEGIN:STANDARD
        DTSTART:19700101T000000
        TZOFFSETFROM:+530
        TZOFFSETTO:+530
        END:STANDARD
        END:VTIMEZONE
        BEGIN:VEVENT
        UID:synology-event
        DTSTART;TZID=Asia/Kolkata:20231215T100000
        SUMMARY:Synology Event
        END:VEVENT
        END:VCALENDAR",
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].dt_start.timestamp, millis("2023-12-15T10:00:00+05:30"));
}

#[test]
fn misplaced_t_designator_in_trigger_is_repaired() {
    let e = parse_one(&single(
        "
        UID:bad-duration-event
        DTSTART:20231215T100000Z
        BEGIN:VALARM
        ACTION:DISPLAY
        TRIGGER:-PT2D
        DESCRIPTION:2 days before
        END:VALARM",
    ));
    assert_eq!(e.alarms.len(), 1);
    assert_eq!(e.alarms[0].trigger, Some(TimeDelta::days(-2)));
}
