//! Port of icaldav-core `ICalParserRoundtripTest`: parse -> generate -> parse.
//!
//! COLOR and GEO are not modelled; they ride through as verbatim raw lines.

mod ical_support;

use chrono::TimeDelta;
use ical_support::*;
use icloud_calendar_mcp::ical::{AlarmAction, EventStatus, Frequency, ICalEvent};

fn event(extra: &str) -> ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:rt\nDTSTAMP:20231215T100000Z\nDTSTART:20231215T140000Z\nDTEND:20231215T150000Z\nSUMMARY:Event\n{}\nEND:VEVENT",
        trim_indent(extra)
    )))
}

#[test]
fn simple_event_round_trips() {
    let e1 = event("DESCRIPTION:A test description\nLOCATION:Conference Room A");
    let e2 = round_trip(&e1);
    assert_eq!(e1.uid, e2.uid);
    assert_eq!(e1.summary, e2.summary);
    assert_eq!(e1.description, e2.description);
    assert_eq!(e1.location, e2.location);
    assert_eq!(e1.is_all_day, e2.is_all_day);
    assert_eq!(e1.dt_start, e2.dt_start);
    assert_eq!(e1.dt_end, e2.dt_end);
}

#[test]
fn special_characters_and_unicode_round_trip() {
    let e1 = parse_one(&vcal(
        r"BEGIN:VEVENT
UID:sc
DTSTART:20231215T140000Z
SUMMARY:Meeting\, Important
DESCRIPTION:Line 1\nLine 2\nLine 3
LOCATION:Room\; Building A
END:VEVENT",
    ));
    let e2 = round_trip(&e1);
    assert_eq!(e2.summary.as_deref(), Some("Meeting, Important"));
    assert_eq!(e2.description.as_deref(), Some("Line 1\nLine 2\nLine 3"));
    assert_eq!(e2.location.as_deref(), Some("Room; Building A"));

    let unicode = event("LOCATION:東京 🗼 café");
    assert_eq!(round_trip(&unicode).location.as_deref(), Some("東京 🗼 café"));
}

#[test]
fn all_day_events_round_trip() {
    for (start, end) in [("20231215", "20231216"), ("20231215", "20231218")] {
        let e1 = parse_one(&vcal(&format!(
            "BEGIN:VEVENT\nUID:ad\nDTSTART;VALUE=DATE:{start}\nDTEND;VALUE=DATE:{end}\nSUMMARY:x\nEND:VEVENT"
        )));
        assert!(e1.is_all_day && e1.dt_start.is_date);
        let e2 = round_trip(&e1);
        assert!(e2.is_all_day && e2.dt_start.is_date);
        assert_eq!(e2.dt_start.to_day_code(), start);
        assert_eq!(e2.dt_end.unwrap().to_day_code(), end);
    }
}

#[test]
fn recurrence_fields_round_trip() {
    let simple = event("RRULE:FREQ=DAILY;COUNT=10");
    let rule = simple.rrule.clone().unwrap();
    assert_eq!((rule.freq, rule.count), (Frequency::Daily, Some(10)));
    assert_eq!(round_trip(&simple).rrule, simple.rrule);

    let complex = event("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=20");
    assert_eq!(round_trip(&complex).rrule, complex.rrule);

    let exdates = event("RRULE:FREQ=DAILY;COUNT=10\nEXDATE:20231217T140000Z\nEXDATE:20231220T140000Z");
    assert_eq!(exdates.exdates.len(), 2);
    assert_eq!(round_trip(&exdates).exdates, exdates.exdates);
}

#[test]
fn duration_round_trips() {
    let e1 = parse_one(&vcal("BEGIN:VEVENT\nUID:d\nDTSTART:20231215T140000Z\nDURATION:PT1H30M\nSUMMARY:x\nEND:VEVENT"));
    assert_eq!(e1.duration, Some(TimeDelta::minutes(90)));
    assert_eq!(round_trip(&e1).duration, e1.duration);
}

#[test]
fn alarms_round_trip() {
    let single = event("BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:15 minute reminder\nEND:VALARM");
    assert_eq!(single.alarms[0].action, AlarmAction::Display);
    let again = round_trip(&single);
    assert_eq!(again.alarms.len(), 1);
    assert_eq!(again.alarms[0].action, AlarmAction::Display);
    assert_eq!(again.alarms[0].trigger, Some(TimeDelta::minutes(-15)));
    assert_eq!(again.alarms[0].description.as_deref(), Some("15 minute reminder"));

    let multiple = event(
        "BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT1H\nDESCRIPTION:a\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:b\nEND:VALARM",
    );
    assert_eq!(round_trip(&multiple).alarms.len(), 2);
}

#[test]
fn metadata_round_trips() {
    let categories = event("CATEGORIES:WORK,IMPORTANT");
    assert_eq!(round_trip(&categories).categories, ["WORK", "IMPORTANT"]);

    let color = event("COLOR:red");
    assert_eq!(raw(&round_trip(&color), "COLOR").unwrap().value, "red");

    let url = event("URL:https://example.com/meeting");
    assert_eq!(url.url.as_deref(), Some("https://example.com/meeting"));
    assert_eq!(round_trip(&url).url, url.url);

    let status = event("STATUS:TENTATIVE");
    assert_eq!(status.status, Some(EventStatus::Tentative));
    assert_eq!(round_trip(&status).status, Some(EventStatus::Tentative));
}

#[test]
fn priority_and_geo_round_trip() {
    let high = event("PRIORITY:1");
    assert_eq!(round_trip(&high).priority, 1);

    let none = event("X-A:1");
    let generated = generate(&none);
    assert!(!generated.contains("PRIORITY:"), "0 means undefined and is not written");
    assert!(!generated.contains("GEO:"));
    assert_eq!(round_trip(&none).priority, 0);

    for geo in ["37.386013;-122.082932", "-33.8688;151.2093"] {
        let e = event(&format!("GEO:{geo}"));
        assert_eq!(raw(&round_trip(&e), "GEO").unwrap().value, geo);
    }
    let both = event("PRIORITY:2\nGEO:40.7128;-74.0060");
    let again = round_trip(&both);
    assert_eq!(again.priority, 2);
    assert_eq!(raw(&again, "GEO").unwrap().value, "40.7128;-74.0060");
}

#[test]
fn modified_instance_round_trips() {
    let e1 = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:recid\nRECURRENCE-ID:20231217T140000Z\nDTSTART:20231217T150000Z\nDTEND:20231217T160000Z\nSUMMARY:x\nEND:VEVENT",
    ));
    let e2 = round_trip(&e1);
    assert_eq!(e2.recurrence_id, e1.recurrence_id);
    assert_eq!(e2.uid, e1.uid);
}
