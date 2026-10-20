//! Port of icaldav-core `ICalParserRdateTest`.
//!
//! Deliberate difference: a VALUE=PERIOD RDATE is not expanded but kept as a
//! raw line, so it survives a round trip instead of being dropped.

mod ical_support;

use chrono_tz::Tz;
use ical_support::*;

fn event(lines: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\nUID:rdate@test.com\nDTSTAMP:20260101T000000Z\n{}\nSUMMARY:x\nEND:VEVENT",
        trim_indent(lines)
    ))
}

#[test]
fn single_utc_rdate() {
    let e = parse_one(&event("DTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nRDATE:20260120T100000Z"));
    assert_eq!(e.rdates.len(), 1);
    assert_eq!(e.rdates[0].to_ical_string(), "20260120T100000Z");
}

#[test]
fn single_rdate_with_tzid() {
    let e =
        parse_one(&event("DTSTART;TZID=America/New_York:20260115T100000\nRDATE;TZID=America/New_York:20260120T100000"));
    assert_eq!(e.rdates[0].timezone, Some(Tz::America__New_York));
}

#[test]
fn date_rdate() {
    let e = parse_one(&event("DTSTART;VALUE=DATE:20260115\nDTEND;VALUE=DATE:20260116\nRDATE;VALUE=DATE:20260120"));
    assert!(e.rdates[0].is_date);
    assert_eq!(e.rdates[0].to_ical_string(), "20260120");
}

#[test]
fn comma_separated_and_repeated_rdates() {
    let comma = parse_one(&event("DTSTART:20260115T100000Z\nRDATE:20260120T100000Z,20260125T100000Z,20260130T100000Z"));
    let strings: Vec<String> = comma.rdates.iter().map(|d| d.to_ical_string()).collect();
    assert_eq!(strings, ["20260120T100000Z", "20260125T100000Z", "20260130T100000Z"]);
    let repeated = parse_one(&event(
        "DTSTART:20260115T100000Z\nRDATE:20260120T100000Z\nRDATE:20260125T100000Z\nRDATE:20260130T100000Z",
    ));
    assert_eq!(repeated.rdates.len(), 3);
    let zones = parse_one(&event(
        "DTSTART:20260115T100000Z\nRDATE;TZID=America/New_York:20260120T100000\nRDATE;TZID=Europe/London:20260125T150000",
    ));
    assert_eq!(zones.rdates[0].timezone, Some(Tz::America__New_York));
    assert_eq!(zones.rdates[1].timezone, Some(Tz::Europe__London));
}

#[test]
fn rdate_with_rrule_and_exdate() {
    let e = parse_one(&event(
        "DTSTART:20260115T100000Z\nRRULE:FREQ=WEEKLY;COUNT=4\nEXDATE:20260122T100000Z\nRDATE:20260210T100000Z",
    ));
    assert!(e.rrule.is_some());
    assert_eq!(e.exdates.len(), 1);
    assert_eq!(e.rdates[0].to_ical_string(), "20260210T100000Z");
}

#[test]
fn rdate_only_event_is_recurring() {
    let e = parse_one(&event("DTSTART:20260115T100000Z\nRDATE:20260120T100000Z,20260125T100000Z"));
    assert!(e.rrule.is_none());
    assert_eq!(e.rdates.len(), 2);
    assert!(e.is_recurring());
    let plain = parse_one(&event("DTSTART:20260115T100000Z"));
    assert!(plain.rdates.is_empty());
    assert!(!plain.is_recurring());
}

#[test]
fn period_rdate_is_kept_raw_not_expanded() {
    let e = parse_one(&event(
        "DTSTART:20260115T100000Z\nRDATE;VALUE=PERIOD:20260120T100000Z/20260120T120000Z\nRDATE:20260125T100000Z",
    ));
    assert_eq!(e.rdates.len(), 1);
    assert_eq!(e.rdates[0].to_ical_string(), "20260125T100000Z");
    assert!(generate(&e).contains("RDATE;VALUE=PERIOD:20260120T100000Z/20260120T120000Z\r\n"));
}

#[test]
fn malformed_rdate_value_is_skipped_and_event_survives() {
    let e = parse_one(&event("DTSTART:20260115T100000Z\nRDATE:INVALID,20260125T100000Z"));
    assert_eq!(e.uid, "rdate@test.com");
    assert_eq!(e.rdates.len(), 1);
}

#[test]
fn rdates_round_trip() {
    let e =
        parse_one(&event("DTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nRDATE:20260120T100000Z,20260125T100000Z"));
    assert!(generate(&e).contains("RDATE:"));
    let again = round_trip(&e);
    assert_eq!(again.rdates, e.rdates);
}
