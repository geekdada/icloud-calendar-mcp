//! Port of icaldav-core `ICalAlarmTest`.
//!
//! There is no `triggerMinutes()` helper; `TimeDelta::num_minutes` stands in.
//! `AlarmProximity` and the ACKNOWLEDGED / RELATED-TO / DEFAULT-ALARM /
//! PROXIMITY fields are raw alarm lines (see `ical_parser_rfc9074.rs`), so the
//! proximity enum cases are skipped and "all optional fields" uses raw lines.

mod ical_support;

use chrono::TimeDelta;
use ical_support::*;
use icloud_calendar_mcp::ical::content::ContentLine;
use icloud_calendar_mcp::ical::duration::{format_duration, parse_duration};
use icloud_calendar_mcp::ical::{AlarmAction, ICalAlarm, ICalDateTime, ICalEvent};

fn relative(action: AlarmAction, trigger: TimeDelta) -> ICalAlarm {
    let mut alarm = ICalAlarm::new(action);
    alarm.trigger = Some(trigger);
    alarm
}

fn trigger_minutes(alarm: &ICalAlarm) -> Option<i64> {
    alarm.trigger.map(|t| t.num_minutes())
}

fn with_alarm(alarm: ICalAlarm) -> ICalEvent {
    let mut e = ICalEvent::new("alarm-test", utc("2023-12-15T14:00:00Z"));
    e.alarms.push(alarm);
    e
}

#[test]
fn construction_and_trigger_minutes() {
    let mut display = relative(AlarmAction::Display, TimeDelta::minutes(-15));
    display.description = Some("Event starting soon".into());
    assert_eq!(display.trigger_absolute, None);
    assert_eq!(trigger_minutes(&display), Some(-15));

    assert_eq!(trigger_minutes(&relative(AlarmAction::Audio, TimeDelta::minutes(-30))), Some(-30));
    assert_eq!(trigger_minutes(&relative(AlarmAction::Email, TimeDelta::hours(-1))), Some(-60));
    assert_eq!(trigger_minutes(&relative(AlarmAction::Display, TimeDelta::minutes(30))), Some(30));
    assert_eq!(trigger_minutes(&relative(AlarmAction::Display, TimeDelta::hours(-2))), Some(-120));
    assert_eq!(trigger_minutes(&relative(AlarmAction::Display, -TimeDelta::minutes(90))), Some(-90));
    assert_eq!(trigger_minutes(&relative(AlarmAction::Display, TimeDelta::zero())), Some(0));
    assert_eq!(trigger_minutes(&relative(AlarmAction::Display, TimeDelta::days(-7))), Some(-7 * 24 * 60));

    let mut absolute = ICalAlarm::new(AlarmAction::Display);
    absolute.trigger_absolute = Some(ICalDateTime::parse("20231215T090000Z", None).unwrap());
    assert_eq!(trigger_minutes(&absolute), None);
    assert_eq!(trigger_minutes(&ICalAlarm::new(AlarmAction::Display)), None);
}

#[test]
fn email_alarm_keeps_summary_and_description() {
    let mut email = relative(AlarmAction::Email, TimeDelta::hours(-1));
    email.description = Some("Email body".into());
    email.summary = Some("Email subject".into());
    let parsed = &round_trip(&with_alarm(email)).alarms[0];
    assert_eq!(parsed.action, AlarmAction::Email);
    assert_eq!(parsed.summary.as_deref(), Some("Email subject"));
    assert_eq!(parsed.description.as_deref(), Some("Email body"));
}

#[test]
fn absolute_trigger_round_trips() {
    let mut absolute = ICalAlarm::new(AlarmAction::Display);
    absolute.trigger_absolute = Some(ICalDateTime::parse("20231215T090000Z", None).unwrap());
    let parsed = &round_trip(&with_alarm(absolute)).alarms[0];
    assert_eq!(parsed.trigger, None);
    assert_eq!(parsed.trigger_absolute.unwrap().to_ical_string(), "20231215T090000Z");
}

#[test]
fn duration_parsing() {
    for (value, expected) in [
        ("-PT15M", TimeDelta::minutes(-15)),
        ("PT15M", TimeDelta::minutes(15)),
        ("-PT1H", TimeDelta::hours(-1)),
        ("-PT1H30M", TimeDelta::minutes(-90)),
        ("-P1D", TimeDelta::days(-1)),
        ("-P1DT2H", TimeDelta::hours(-26)),
        ("-P1W", TimeDelta::days(-7)),
        ("PT30S", TimeDelta::seconds(30)),
        ("P1DT2H30M15S", TimeDelta::days(1) + TimeDelta::hours(2) + TimeDelta::minutes(30) + TimeDelta::seconds(15)),
    ] {
        assert_eq!(parse_duration(value), Some(expected), "{value}");
    }
    assert_eq!(parse_duration("invalid"), None);
    assert_eq!(parse_duration(""), None);
}

#[test]
fn duration_formatting() {
    assert_eq!(format_duration(TimeDelta::minutes(-15)), "-PT15M");
    assert_eq!(format_duration(TimeDelta::minutes(15)), "PT15M");
    assert_eq!(format_duration(TimeDelta::hours(-2)), "-PT2H");
    assert_eq!(format_duration(TimeDelta::zero()), "PT0S");
}

#[test]
fn action_values() {
    for (value, expected) in [
        (Some("DISPLAY"), AlarmAction::Display),
        (Some("display"), AlarmAction::Display),
        (Some("AUDIO"), AlarmAction::Audio),
        (Some("audio"), AlarmAction::Audio),
        (Some("EMAIL"), AlarmAction::Email),
        (Some("email"), AlarmAction::Email),
        (Some("NONE"), AlarmAction::None),
        (Some("none"), AlarmAction::None),
        (None, AlarmAction::Display),
        (Some(""), AlarmAction::Display),
        (Some("UNKNOWN"), AlarmAction::Display),
        (Some("PROCEDURE"), AlarmAction::Display),
    ] {
        assert_eq!(AlarmAction::from_value(value), expected, "{value:?}");
    }
}

#[test]
fn action_none_is_never_turned_into_display() {
    // Apple's default-alarm suppressor: ACTION:NONE with a 1976 absolute trigger.
    let e = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:n\nDTSTART:20231215T140000Z\nBEGIN:VALARM\nACTION:NONE\nTRIGGER;VALUE=DATE-TIME:19760401T005545Z\nEND:VALARM\nEND:VEVENT",
    ));
    assert_eq!(e.alarms[0].action, AlarmAction::None);
    let ics = generate(&e);
    assert!(ics.contains("ACTION:NONE\r\n"));
    assert!(!ics.contains("ACTION:DISPLAY"));
    assert_eq!(round_trip(&e).alarms[0].action, AlarmAction::None);
}

#[test]
fn repeat_fields() {
    let plain = relative(AlarmAction::Display, TimeDelta::minutes(-15));
    assert_eq!((plain.repeat_count, plain.repeat_duration), (0, None));

    let mut repeating = plain.clone();
    repeating.repeat_count = 3;
    repeating.repeat_duration = Some(TimeDelta::minutes(5));
    let parsed = &round_trip(&with_alarm(repeating)).alarms[0];
    assert_eq!(parsed.repeat_count, 3);
    assert_eq!(parsed.repeat_duration, Some(TimeDelta::minutes(5)));
}

#[test]
fn trigger_related_to_end() {
    assert!(!relative(AlarmAction::Display, TimeDelta::minutes(-15)).trigger_related_to_end);
    let mut at_end = relative(AlarmAction::Display, TimeDelta::zero());
    at_end.trigger_related_to_end = true;
    assert!(round_trip(&with_alarm(at_end)).alarms[0].trigger_related_to_end);
}

#[test]
fn all_optional_fields_round_trip() {
    let mut alarm = relative(AlarmAction::Email, TimeDelta::minutes(-30));
    alarm.description = Some("Email body".into());
    alarm.summary = Some("Email subject".into());
    alarm.repeat_count = 2;
    alarm.repeat_duration = Some(TimeDelta::minutes(10));
    alarm.uid = Some("alarm-123".into());
    alarm.raw_properties =
        ["ACKNOWLEDGED:20231215T100000Z", "RELATED-TO:parent-alarm"].map(|l| ContentLine::parse(l).unwrap()).into();
    let parsed = round_trip(&with_alarm(alarm.clone())).alarms.remove(0);
    assert_eq!(parsed.uid, alarm.uid);
    for name in ["ACKNOWLEDGED", "RELATED-TO"] {
        assert!(parsed.raw_properties.iter().any(|p| p.name == name), "{name}");
    }
}

#[test]
fn value_equality_and_clone() {
    let mut a = relative(AlarmAction::Display, TimeDelta::minutes(-15));
    a.description = Some("Test".into());
    let b = a.clone();
    assert_eq!(a, b);
    let mut modified = a.clone();
    modified.description = Some("Modified".into());
    assert_ne!(a, modified);
}
