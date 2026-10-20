//! Port of icaldav-core `ICalGeneratorEdgeCaseTest`. METHOD output is skipped
//! (storage bodies never carry one).

mod ical_support;

use chrono::TimeDelta;
use chrono_tz::Tz;
use ical_support::*;
use icloud_calendar_mcp::ical::content::unfold;
use icloud_calendar_mcp::ical::{AlarmAction, Frequency, ICalAlarm, ICalDateTime, ICalEvent, RRule};

const START: i64 = 1_702_000_000_000;

fn event() -> ICalEvent {
    let mut e = ICalEvent::new("test-uid", ICalDateTime::utc(START));
    e.dt_end = Some(ICalDateTime::utc(START + 3_600_000));
    e.summary = Some("Test Event".into());
    e
}

fn with_summary(summary: &str) -> String {
    let mut e = event();
    e.summary = Some(summary.into());
    generate(&e)
}

fn physical_lines(ics: &str) -> Vec<&str> {
    ics.split("\r\n").filter(|l| !l.is_empty()).collect()
}

#[test]
fn folding_counts_octets_and_keeps_characters_whole() {
    for summary in [
        "A".repeat(80),
        "\u{4e2d}".repeat(26),
        format!("Meeting {}", "\u{1F4C5}".repeat(20)),
        format!("{}{}", "A".repeat(73), "\u{4e2d}\u{4e2d}"),
    ] {
        let ics = with_summary(&summary);
        assert!(ics.contains("\r\n "), "long line folds: {summary}");
        assert!(physical_lines(&ics).iter().all(|l| l.len() <= 75));
        assert_eq!(parse_one(&ics).summary.as_deref(), Some(summary.as_str()));
    }
}

#[test]
fn long_description_folds_many_times() {
    let mut e = event();
    e.description = Some("Lorem ipsum ".repeat(100));
    let ics = generate(&e);
    assert!(ics.matches("\r\n ").count() > 5);
    assert_eq!(round_trip(&e).description, e.description);
}

#[test]
fn unicode_location_survives() {
    let mut e = event();
    e.location = Some("Caf\u{e9} M\u{fc}ller, Stra\u{df}e 123, \u{5317}\u{4eac}".into());
    let unfolded = unfold(&generate(&e));
    assert!(unfolded.contains("Caf\u{e9}") && unfolded.contains("\u{5317}\u{4eac}"));
}

#[test]
fn text_escaping() {
    assert!(with_summary("Meeting, Important, Urgent").contains("SUMMARY:Meeting\\, Important\\, Urgent\r\n"));
    assert!(with_summary("Task; Priority: High").contains("SUMMARY:Task\\; Priority: High\r\n"));
    let mut e = event();
    e.description = Some("Line 1\nLine 2\nLine 3".into());
    let ics = generate(&e);
    assert!(ics.contains("DESCRIPTION:Line 1\\nLine 2\\nLine 3\r\n"));
    assert!(!ics.contains("DESCRIPTION:Line 1\nLine 2"));
    assert!(with_summary("Path: C:\\Users\\test").contains("SUMMARY:Path: C:\\\\Users\\\\test\r\n"));
    // Backslash is escaped first, so a literal "\n" in text stays two characters.
    let tricky = with_summary("test\\nvalue");
    assert!(tricky.contains("SUMMARY:test\\\\nvalue\r\n"));
    assert_eq!(parse_one(&tricky).summary.as_deref(), Some("test\\nvalue"));
}

#[test]
fn required_fields_for_icloud() {
    let ics = generate(&event());
    for needle in ["CALSCALE:GREGORIAN", "STATUS:CONFIRMED", "SEQUENCE:0", "UID:test-uid", "PRODID:", "VERSION:2.0"] {
        assert!(ics.contains(needle), "{needle}");
    }
    assert!(!ics.contains("METHOD:"));
    let dtstamp = physical_lines(&ics).into_iter().find(|l| l.starts_with("DTSTAMP:")).unwrap();
    assert!(dtstamp.ends_with('Z'));
}

#[test]
fn recurrence_id_handling() {
    assert!(!generate(&event()).contains("RECURRENCE-ID:"));
    let mut exception = event();
    exception.recurrence_id = Some(ICalDateTime::utc(START));
    exception.rrule = Some(RRule::new(Frequency::Weekly));
    let ics = generate(&exception);
    assert!(ics.contains("RECURRENCE-ID:"));
    assert!(!ics.contains("RRULE:"), "a modified instance never carries RRULE");
}

#[test]
fn datetime_formatting() {
    let utc = generate(&event());
    assert!(physical_lines(&utc).into_iter().find(|l| l.starts_with("DTSTART:")).unwrap().ends_with('Z'));

    let mut all_day = event();
    all_day.is_all_day = true;
    all_day.dt_start = ICalDateTime { timestamp: START, timezone: None, is_utc: false, is_date: true };
    all_day.dt_end = None;
    let ics = generate(&all_day);
    let line = physical_lines(&ics).into_iter().find(|l| l.starts_with("DTSTART")).unwrap();
    assert!(line.starts_with("DTSTART;VALUE=DATE:"));
    assert!(!line.split_once(':').unwrap().1.contains('T'));

    let mut zoned = event();
    zoned.dt_start =
        ICalDateTime { timestamp: START, timezone: Some(Tz::America__New_York), is_utc: false, is_date: false };
    assert!(generate(&zoned).contains("DTSTART;TZID=America/New_York:"));
}

#[test]
fn valarm_generation() {
    let mut display = ICalAlarm::new(AlarmAction::Display);
    display.trigger = Some(TimeDelta::minutes(-15));
    display.description = Some("Reminder".into());
    let mut end_related = ICalAlarm::new(AlarmAction::Display);
    end_related.trigger = Some(TimeDelta::minutes(-5));
    end_related.trigger_related_to_end = true;
    let mut e = event();
    e.alarms = vec![display, end_related];
    let ics = generate(&e);
    for needle in ["BEGIN:VALARM", "ACTION:DISPLAY", "TRIGGER:-PT15M", "END:VALARM", "TRIGGER;RELATED=END:-PT5M"] {
        assert!(ics.contains(needle), "{needle}");
    }
}

#[test]
fn properties_precede_alarms() {
    let mut e = event();
    let mut a = ICalAlarm::new(AlarmAction::Display);
    a.trigger = Some(TimeDelta::minutes(-1));
    e.alarms.push(a);
    e.created = Some(ICalDateTime::utc(START));
    e.raw_properties.push(icloud_calendar_mcp::ical::content::ContentLine::parse("X-LAST:1").unwrap());
    let ics = generate(&e);
    let alarm_at = ics.find("BEGIN:VALARM").unwrap();
    assert!(ics.find("CREATED:").unwrap() < alarm_at);
    assert!(ics.find("X-LAST:1").unwrap() < alarm_at, "RFC 5545 puts eventprop before alarmc");
}
