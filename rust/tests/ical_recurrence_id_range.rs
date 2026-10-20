//! Port of icaldav-core `parser/RecurrenceIdRangeTest`: `RECURRENCE-ID;RANGE=
//! THISANDFUTURE` (RFC 5545 §3.2.13) survives parse → generate → parse.

mod recurrence_support;

use icloud_calendar_mcp::ical::ICalEvent;
use icloud_calendar_mcp::ical::generator::generate_calendar;
use recurrence_support::*;

const THIS_AND_FUTURE: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\n\
BEGIN:VEVENT\nUID:standup\nDTSTART;TZID=America/New_York:20231201T100000\nDTEND;TZID=America/New_York:20231201T103000\n\
RRULE:FREQ=DAILY;COUNT=5\nSUMMARY:Standup\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:standup\nRECURRENCE-ID;RANGE=THISANDFUTURE;TZID=America/New_York:20231203T100000\n\
DTSTART;TZID=America/New_York:20231203T110000\nDTEND;TZID=America/New_York:20231203T113000\n\
SUMMARY:Standup (moved to 11am, from here on)\nEND:VEVENT\nEND:VCALENDAR\n";

fn override_of(ics: &str) -> ICalEvent {
    parse(ics).into_iter().find(|e| e.recurrence_id.is_some()).expect("override event")
}

fn recurrence_id_line(ics: &str) -> String {
    ics.lines().find(|l| l.starts_with("RECURRENCE-ID")).expect("RECURRENCE-ID line").to_string()
}

#[test]
fn this_and_future_is_lifted_onto_override() {
    assert_eq!(override_of(THIS_AND_FUTURE).recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
}

#[test]
fn absent_range_parses_to_none() {
    let ics = THIS_AND_FUTURE.replace(";RANGE=THISANDFUTURE", "");
    assert!(override_of(&ics).recurrence_id_range.is_none());
}

#[test]
fn range_value_is_case_insensitive() {
    let ics = THIS_AND_FUTURE.replace("RANGE=THISANDFUTURE", "RANGE=thisandfuture");
    assert_eq!(override_of(&ics).recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
}

#[test]
fn single_instance_override_has_no_range() {
    let e = override_of(&vcal(
        "BEGIN:VEVENT\nUID:standup\nRECURRENCE-ID;TZID=America/New_York:20231203T100000\n\
         DTSTART;TZID=America/New_York:20231203T110000\nSUMMARY:Standup (just this one)\nEND:VEVENT\n",
    ));
    assert!(e.recurrence_id_range.is_none());
}

#[test]
fn this_and_future_is_emitted_on_recurrence_id_line() {
    let line = recurrence_id_line(&generate_calendar(&[override_of(THIS_AND_FUTURE)]));
    assert!(line.contains("RANGE=THISANDFUTURE"), "line was: {line}");
}

#[test]
fn no_range_emits_no_range_parameter() {
    let mut plain = override_of(THIS_AND_FUTURE);
    plain.recurrence_id_range = None;
    let line = recurrence_id_line(&generate_calendar(&[plain]));
    assert!(!line.contains("RANGE"), "line was: {line}");
}

#[test]
fn this_and_future_survives_round_trip() {
    let once = override_of(THIS_AND_FUTURE);
    let twice = override_of(&generate_calendar(std::slice::from_ref(&once)));
    assert_eq!(twice.recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
    assert_eq!(twice.recurrence_id, once.recurrence_id);
}
