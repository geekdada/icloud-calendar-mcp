//! Port of icaldav-core `ICalParserDurationTest`: DURATION and floating times.

mod ical_support;

use chrono::TimeDelta;
use chrono_tz::Tz;
use ical_support::*;
use icloud_calendar_mcp::ical::Frequency;
use icloud_calendar_mcp::ical::generator::generate_calendar;

fn with_duration(start: &str, duration: &str) -> String {
    vcal(&format!("BEGIN:VEVENT\nUID:d\nDTSTAMP:20231215T100000Z\n{start}\nDURATION:{duration}\nSUMMARY:x\nEND:VEVENT"))
}

#[test]
fn duration_instead_of_dtend() {
    let event = parse_one(&with_duration("DTSTART:20231215T140000Z", "PT1H30M"));
    assert_eq!(event.dt_end, None);
    assert_eq!(event.duration, Some(TimeDelta::minutes(90)));
    assert!(generate_calendar(&[event]).contains("DURATION:PT1H30M\r\n"));
}

#[test]
fn duration_shapes() {
    assert_eq!(parse_one(&with_duration("DTSTART:20231215T100000Z", "PT2H")).duration, Some(TimeDelta::hours(2)));
    assert_eq!(parse_one(&with_duration("DTSTART:20231215T100000Z", "PT45M")).duration, Some(TimeDelta::minutes(45)));
    let days = parse_one(&with_duration("DTSTART;VALUE=DATE:20231215", "P3D"));
    assert_eq!(days.duration, Some(TimeDelta::days(3)));
    assert!(days.is_all_day);
    assert_eq!(parse_one(&with_duration("DTSTART;VALUE=DATE:20231215", "P2W")).duration, Some(TimeDelta::days(14)));
    assert_eq!(
        parse_one(&with_duration("DTSTART:20231215T100000Z", "P1DT2H30M")).duration,
        Some(TimeDelta::days(1) + TimeDelta::hours(2) + TimeDelta::minutes(30))
    );
    assert_eq!(parse_one(&with_duration("DTSTART:20231215T100000Z", "PT0S")).duration, Some(TimeDelta::zero()));
}

#[test]
fn recurring_event_with_duration() {
    let event = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:r\nDTSTART:20231215T100000Z\nDURATION:PT1H\nRRULE:FREQ=DAILY;COUNT=5\nSUMMARY:x\nEND:VEVENT",
    ));
    assert_eq!(event.duration, Some(TimeDelta::hours(1)));
    assert_eq!(event.rrule.unwrap().freq, Frequency::Daily);
}

#[test]
fn unreadable_duration_is_absent() {
    let event = parse_one(&with_duration("DTSTART:20231215T100000Z", "soon"));
    assert_eq!(event.duration, None);
}

#[test]
fn floating_utc_and_zoned_times_are_distinct() {
    let floating = parse_one(&vcal("BEGIN:VEVENT\nUID:f\nDTSTART:20231215T140000\nSUMMARY:x\nEND:VEVENT"));
    let utc_event = parse_one(&vcal("BEGIN:VEVENT\nUID:u\nDTSTART:20231215T140000Z\nSUMMARY:x\nEND:VEVENT"));
    let zoned =
        parse_one(&vcal("BEGIN:VEVENT\nUID:z\nDTSTART;TZID=America/New_York:20231215T140000\nSUMMARY:x\nEND:VEVENT"));
    assert!(!floating.dt_start.is_utc);
    assert!(!floating.is_all_day);
    assert!(utc_event.dt_start.is_utc);
    assert_eq!(zoned.dt_start.timezone, Some(Tz::America__New_York));
    assert!(!zoned.dt_start.is_utc);
}

#[test]
fn recurring_floating_event() {
    let event = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:fr\nDTSTART:20231215T090000\nDTEND:20231215T100000\nRRULE:FREQ=DAILY;COUNT=5\nSUMMARY:x\nEND:VEVENT",
    ));
    assert!(!event.dt_start.is_utc);
    assert!(event.rrule.is_some());
}

#[test]
fn dtend_wins_over_duration_on_output() {
    let event = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:b\nDTSTART:20231215T100000Z\nDTEND:20231215T120000Z\nDURATION:PT1H\nSUMMARY:x\nEND:VEVENT",
    ));
    assert!(event.dt_end.is_some());
    let ics = generate_calendar(&[event]);
    assert!(ics.contains("DTEND:20231215T120000Z\r\n"));
    assert!(!ics.contains("DURATION:"), "DTEND and DURATION are mutually exclusive on output");
}

#[test]
fn negative_alarm_trigger() {
    let event = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20231215T140000Z\nSUMMARY:x\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT30M\nDESCRIPTION:d\nEND:VALARM\nEND:VEVENT",
    ));
    assert_eq!(event.alarms[0].trigger, Some(TimeDelta::minutes(-30)));
}
