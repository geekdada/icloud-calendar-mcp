//! Port of icaldav-core `ICalGeneratorTest`.
//!
//! Skipped: METHOD output and the `includeVTimezone` / `includeAppleExtensions`
//! switches (the server always writes storage bodies with VTIMEZONEs and Apple
//! alarm extensions). A default alarm is marked with a raw DEFAULT-ALARM line.

mod ical_support;

use chrono::{TimeDelta, TimeZone, Weekday};
use chrono_tz::Tz;
use ical_support::*;
use icloud_calendar_mcp::ical::content::ContentLine;
use icloud_calendar_mcp::ical::generator::generate_calendar;
use icloud_calendar_mcp::ical::{
    AlarmAction, EventStatus, Frequency, ICalAlarm, ICalDateTime, ICalEvent, RRule, WeekdayNum,
};

fn test_event(uid: &str, zone: Tz, is_all_day: bool) -> ICalEvent {
    let start = zone.with_ymd_and_hms(2023, 12, 15, 14, 0, 0).unwrap();
    let end = zone.with_ymd_and_hms(2023, 12, 15, 15, 0, 0).unwrap();
    let mut e = ICalEvent::new(uid, ICalDateTime::from_zoned(start, is_all_day));
    e.dt_end = Some(ICalDateTime::from_zoned(end, is_all_day));
    e.is_all_day = is_all_day;
    e.summary = Some("Test Event".into());
    e.description = Some("Test description".into());
    e.location = Some("Test location".into());
    e.status = Some(EventStatus::Confirmed);
    e
}

fn ny_event() -> ICalEvent {
    test_event("test-uid-123", Tz::America__New_York, false)
}

fn alarm(minutes: i64) -> ICalAlarm {
    let mut a = ICalAlarm::new(AlarmAction::Display);
    a.trigger = Some(TimeDelta::minutes(minutes));
    a.description = Some("Reminder".into());
    a
}

fn valarm_block(ics: &str) -> &str {
    let start = ics.find("BEGIN:VALARM").unwrap();
    let end = ics.find("END:VALARM").unwrap();
    &ics[start..end]
}

#[test]
fn simple_event_has_structure_and_required_properties() {
    let ics = generate(&ny_event());
    for needle in [
        "BEGIN:VCALENDAR",
        "END:VCALENDAR",
        "BEGIN:VEVENT",
        "END:VEVENT",
        "UID:test-uid-123",
        "SUMMARY:Test Event",
        "VERSION:2.0",
        "PRODID:",
        "CALSCALE:GREGORIAN",
        "SEQUENCE:",
        "STATUS:",
        "\r\n",
    ] {
        assert!(ics.contains(needle), "missing {needle}");
    }
    assert!(!ics.contains("METHOD:"));
}

#[test]
fn all_day_and_timed_start_formats() {
    let all_day = generate(&test_event("a", Tz::America__New_York, true));
    assert!(all_day.contains("DTSTART;VALUE=DATE:20231215\r\n"));
    let timed = generate(&ny_event());
    assert!(timed.contains("DTSTART;TZID=America/New_York:20231215T140000\r\n"));
}

#[test]
fn alarm_and_recurrence_id() {
    let mut e = ny_event();
    e.alarms.push(alarm(-15));
    let ics = generate(&e);
    for needle in ["BEGIN:VALARM", "END:VALARM", "ACTION:DISPLAY", "TRIGGER:-PT15M"] {
        assert!(ics.contains(needle));
    }

    let mut exception = ny_event();
    exception.recurrence_id = Some(ICalDateTime::parse("20231208T140000Z", None).unwrap());
    assert!(generate(&exception).contains("RECURRENCE-ID:20231208T140000Z\r\n"));
}

#[test]
fn text_is_escaped() {
    let mut e = ny_event();
    e.summary = Some("Meeting, Important".into());
    e.description = Some("Line 1\nLine 2\nWith; semicolon".into());
    let ics = generate(&e);
    assert!(ics.contains("SUMMARY:Meeting\\, Important\r\n"));
    assert!(ics.contains("DESCRIPTION:Line 1\\nLine 2\\nWith\\; semicolon\r\n"));
}

#[test]
fn rrule_is_written() {
    let mut rule = RRule::new(Frequency::Weekly);
    rule.count = Some(10);
    rule.by_day = Some(vec![
        WeekdayNum { weekday: Weekday::Mon, ordinal: None },
        WeekdayNum { weekday: Weekday::Wed, ordinal: None },
    ]);
    let mut e = ny_event();
    e.rrule = Some(rule);
    assert!(generate(&e).contains("RRULE:FREQ=WEEKLY;COUNT=10;BYDAY=MO,WE\r\n"));
}

#[test]
fn round_trip_is_equivalent() {
    let original = ny_event();
    let parsed = round_trip(&original);
    assert_eq!(parsed.uid, original.uid);
    assert_eq!(parsed.summary, original.summary);
    assert_eq!(parsed.description, original.description);
    assert_eq!(parsed.location, original.location);
    assert_eq!(parsed.dt_start, original.dt_start);
}

#[test]
fn vtimezone_included_before_vevent() {
    let ics = generate(&ny_event());
    assert!(ics.contains("BEGIN:VTIMEZONE") && ics.contains("TZID:America/New_York") && ics.contains("END:VTIMEZONE"));
    let tz = ics.find("BEGIN:VTIMEZONE").unwrap();
    assert!(tz > 0 && tz < ics.find("BEGIN:VEVENT").unwrap());
}

#[test]
fn batch_deduplicates_and_collects_zones() {
    let ics = generate_calendar(&[ny_event(), test_event("event-2", Tz::America__New_York, false)]);
    assert_eq!(ics.matches("TZID:America/New_York\r\n").count(), 1);
    let mixed = generate_calendar(&[ny_event(), test_event("tokyo", Tz::Asia__Tokyo, false)]);
    assert!(mixed.contains("TZID:America/New_York\r\n") && mixed.contains("TZID:Asia/Tokyo\r\n"));
}

#[test]
fn apple_alarm_extensions() {
    let mut e = ny_event();
    e.alarms.push(alarm(-15));
    let ics = generate(&e);
    let block = valarm_block(&ics);
    assert!(block.contains("UID:"));
    assert!(block.contains("X-WR-ALARMUID:"));
    assert!(block.contains("X-APPLE-DEFAULT-ALARM:FALSE"));
}

#[test]
fn alarm_uid_is_mirrored_in_x_wr_alarmuid() {
    let mut a = alarm(-10);
    a.uid = Some("test-alarm-uid-123".into());
    let mut e = ny_event();
    e.alarms.push(a);
    let ics = generate(&e);
    assert!(ics.contains("UID:test-alarm-uid-123\r\n"));
    assert!(ics.contains("X-WR-ALARMUID:test-alarm-uid-123\r\n"));
}

#[test]
fn default_alarm_omits_x_apple_default_alarm() {
    let mut a = alarm(-15);
    a.raw_properties.push(ContentLine::parse("DEFAULT-ALARM:TRUE").unwrap());
    let mut e = ny_event();
    e.alarms.push(a);
    let ics = generate(&e);
    assert!(ics.contains("X-WR-ALARMUID:"));
    assert!(!ics.contains("X-APPLE-DEFAULT-ALARM:FALSE"));
    assert!(ics.contains("DEFAULT-ALARM:TRUE\r\n"));
}

#[test]
fn utc_event_has_no_vtimezone() {
    let mut e = ICalEvent::new("utc-event", ICalDateTime::parse("20231215T140000Z", None).unwrap());
    e.dt_end = Some(ICalDateTime::parse("20231215T150000Z", None).unwrap());
    assert!(!generate(&e).contains("BEGIN:VTIMEZONE"));
}

#[test]
fn priority_output() {
    for (priority, expected) in [(1, Some("PRIORITY:1")), (0, None), (9, Some("PRIORITY:9"))] {
        let mut e = ny_event();
        e.priority = priority;
        let ics = generate(&e);
        match expected {
            Some(line) => assert!(ics.contains(&format!("{line}\r\n"))),
            None => assert!(!ics.contains("PRIORITY:")),
        }
    }
}

#[test]
fn geo_output_from_raw_line() {
    let mut e = ny_event();
    assert!(!generate(&e).contains("GEO:"));
    for geo in ["37.386013;-122.082932", "-33.8688;151.2093"] {
        e.raw_properties = vec![ContentLine::parse(&format!("GEO:{geo}")).unwrap()];
        assert!(generate(&e).contains(&format!("GEO:{geo}\r\n")));
    }
}
