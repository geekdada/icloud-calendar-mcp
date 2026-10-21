//! Port of icaldav-core `RecurrenceIdRangeTest`. The Kotlin `RecurrenceRange`
//! enum is a normalised `Option<String>` here.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::ICalEvent;

const THIS_AND_FUTURE: &str = "
    BEGIN:VCALENDAR
    VERSION:2.0
    PRODID:-//Test//Test//EN
    BEGIN:VEVENT
    UID:standup
    DTSTART;TZID=America/New_York:20231201T100000
    DTEND;TZID=America/New_York:20231201T103000
    RRULE:FREQ=DAILY;COUNT=5
    SUMMARY:Standup
    END:VEVENT
    BEGIN:VEVENT
    UID:standup
    RECURRENCE-ID;RANGE=THISANDFUTURE;TZID=America/New_York:20231203T100000
    DTSTART;TZID=America/New_York:20231203T110000
    DTEND;TZID=America/New_York:20231203T113000
    SUMMARY:Standup (moved to 11am, from here on)
    END:VEVENT
    END:VCALENDAR";

fn override_of(ics: &str) -> ICalEvent {
    parse(ics).into_iter().find(|e| e.recurrence_id.is_some()).expect("an override")
}

fn recurrence_id_line(event: &ICalEvent) -> String {
    lines(&generate(event)).into_iter().find(|l| l.starts_with("RECURRENCE-ID")).unwrap()
}

#[test]
fn this_and_future_is_lifted_onto_the_override() {
    assert_eq!(override_of(THIS_AND_FUTURE).recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
}

#[test]
fn absent_range_parses_to_none() {
    let ics = THIS_AND_FUTURE.replace(";RANGE=THISANDFUTURE", "");
    assert_eq!(override_of(&ics).recurrence_id_range, None);
}

#[test]
fn range_is_case_insensitive() {
    let ics = THIS_AND_FUTURE.replace("RANGE=THISANDFUTURE", "RANGE=thisandfuture");
    assert_eq!(override_of(&ics).recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
}

#[test]
fn plain_override_has_no_range() {
    let ics = vcal(
        "BEGIN:VEVENT\nUID:standup\nRECURRENCE-ID;TZID=America/New_York:20231203T100000\nDTSTART;TZID=America/New_York:20231203T110000\nSUMMARY:Standup (just this one)\nEND:VEVENT",
    );
    assert_eq!(override_of(&ics).recurrence_id_range, None);
}

#[test]
fn range_is_emitted_on_the_recurrence_id_line() {
    let line = recurrence_id_line(&override_of(THIS_AND_FUTURE));
    assert!(line.contains("RANGE=THISANDFUTURE"), "line was: {line}");
}

#[test]
fn no_range_emits_no_parameter() {
    let mut plain = override_of(THIS_AND_FUTURE);
    plain.recurrence_id_range = None;
    let line = recurrence_id_line(&plain);
    assert!(!line.contains("RANGE"), "line was: {line}");
}

#[test]
fn range_and_anchor_survive_a_round_trip() {
    let once = override_of(THIS_AND_FUTURE);
    let twice = override_of(&generate(&once));
    assert_eq!(twice.recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
    assert_eq!(twice.recurrence_id, once.recurrence_id);
}
