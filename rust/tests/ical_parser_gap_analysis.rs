//! Port of the VEVENT parts of icaldav-core `ICalParserGapAnalysisTest`.
//!
//! CLASS and X-* properties are kept as verbatim raw lines (the Kotlin model
//! had a `classification` enum and a `rawProperties` map keyed by name).

mod ical_support;

use chrono::{DateTime, TimeDelta, Utc};
use ical_support::*;
use icloud_calendar_mcp::ical::expander::expand;

fn utc_instant(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn all_day_recurring_dates_do_not_shift() {
    let yearly = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20260101\nDTEND;VALUE=DATE:20260102\nRRULE:FREQ=YEARLY;COUNT=5\nSUMMARY:x\nEND:VEVENT",
    ));
    let occurrences =
        expand(&yearly, utc_instant("2025-12-01T00:00:00Z"), utc_instant("2031-12-31T00:00:00Z"), &[]).unwrap();
    let days: Vec<String> = occurrences.iter().map(|o| o.dt_start.to_day_code()).collect();
    assert_eq!(days, ["20260101", "20270101", "20280101", "20290101", "20300101"]);

    let monthly = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:m\nDTSTART;VALUE=DATE:20260315\nDTEND;VALUE=DATE:20260316\nRRULE:FREQ=MONTHLY;COUNT=3\nSUMMARY:x\nEND:VEVENT",
    ));
    let occurrences =
        expand(&monthly, utc_instant("2026-01-01T00:00:00Z"), utc_instant("2026-12-31T00:00:00Z"), &[]).unwrap();
    let days: Vec<String> = occurrences.iter().map(|o| o.dt_start.to_day_code()).collect();
    assert_eq!(days, ["20260315", "20260415", "20260515"]);
}

fn event_with(lines: &str) -> icloud_calendar_mcp::ical::ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:g\nDTSTART:20260115T140000Z\nDTEND:20260115T150000Z\nSUMMARY:x\n{lines}\nEND:VEVENT"
    )))
}

#[test]
fn class_values_round_trip() {
    for class in ["PUBLIC", "PRIVATE", "CONFIDENTIAL"] {
        let e = event_with(&format!("CLASS:{class}"));
        assert_eq!(raw(&round_trip(&e), "CLASS").unwrap().value, class);
    }
}

#[test]
fn x_properties_are_preserved() {
    let apple = event_with("X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-TITLE=Apple Park:geo:37.334722,-122.008889");
    let line = raw(&apple, "X-APPLE-STRUCTURED-LOCATION").unwrap();
    assert_eq!(line.param("X-TITLE"), Some("Apple Park"));
    assert_eq!(line.value, "geo:37.334722,-122.008889");

    let google = event_with("X-GOOGLE-CONFERENCE:https://meet.google.com/abc-defg-hij");
    assert_eq!(raw(&google, "X-GOOGLE-CONFERENCE").unwrap().value, "https://meet.google.com/abc-defg-hij");

    let many = event_with("X-CUSTOM-PROP1:value1\nX-CUSTOM-PROP2:value2\nX-CUSTOM-PROP3:value3");
    assert_eq!(many.raw_properties.iter().filter(|p| p.name.starts_with("X-")).count(), 3);

    let again = round_trip(&event_with("X-CUSTOM-TEST:test-value-123"));
    assert_eq!(raw(&again, "X-CUSTOM-TEST").unwrap().value, "test-value-123");
}

#[test]
fn repeated_x_properties_are_all_kept() {
    // A map keyed by name would collapse these; raw lines keep every one.
    let e = event_with("X-TAG:one\nX-TAG:two");
    let tags: Vec<_> = e.raw_properties.iter().filter(|p| p.name == "X-TAG").map(|p| p.value.as_str()).collect();
    assert_eq!(tags, ["one", "two"]);
    assert_eq!(round_trip(&e).raw_properties.iter().filter(|p| p.name == "X-TAG").count(), 2);
}

#[test]
fn valarm_triggers() {
    let pt = event_with("BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT30M\nDESCRIPTION:d\nEND:VALARM");
    assert_eq!(pt.alarms[0].trigger, Some(TimeDelta::seconds(-1800)));
    let days = event_with("BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nDESCRIPTION:d\nEND:VALARM");
    assert_eq!(days.alarms[0].trigger, Some(TimeDelta::seconds(-86_400)));
    let absolute =
        event_with("BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;VALUE=DATE-TIME:20260115T130000Z\nDESCRIPTION:d\nEND:VALARM");
    assert_eq!(absolute.alarms[0].trigger_absolute.unwrap().to_ical_string(), "20260115T130000Z");
    assert_eq!(absolute.alarms[0].trigger, None);
    let three = event_with(
        "BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT1H\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nEND:VALARM",
    );
    assert_eq!(three.alarms.len(), 3);
}

#[test]
fn recurrence_id_forms() {
    for (master, recid) in [
        ("DTSTART:20260101T100000Z", "RECURRENCE-ID:20260103T100000Z"),
        ("DTSTART;TZID=America/New_York:20260101T100000", "RECURRENCE-ID;TZID=America/New_York:20260103T100000"),
        ("DTSTART;VALUE=DATE:20260101", "RECURRENCE-ID;VALUE=DATE:20260103"),
    ] {
        let events = parse(&vcal(&format!(
            "BEGIN:VEVENT\nUID:r\n{master}\nRRULE:FREQ=DAILY;COUNT=5\nEND:VEVENT\nBEGIN:VEVENT\nUID:r\n{recid}\n{master}\nEND:VEVENT"
        )));
        let exception = events.iter().find(|e| e.recurrence_id.is_some()).unwrap();
        let rid = exception.recurrence_id.unwrap();
        assert_eq!(rid.to_day_code(), "20260103", "{recid}");
        assert_eq!(rid.is_date, recid.contains("VALUE=DATE"));
    }
}
