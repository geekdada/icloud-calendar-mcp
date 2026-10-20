//! Port of icaldav-core `ICalDateTimeTest`.
//!
//! Skipped: `customTimezoneResolver` (no pluggable resolver) and the alias
//! table size / typo-filter checks (the table is crate-private; resolution
//! through it is covered below).

use chrono::{Datelike, TimeZone, Timelike};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::ICalDateTime;
use icloud_calendar_mcp::ical::timezone::{resolve_tzid, system_zone};

fn parse(value: &str, tzid: Option<&str>) -> ICalDateTime {
    ICalDateTime::parse(value, tzid).unwrap()
}

fn utc_millis(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
}

#[test]
fn utc_datetime() {
    let dt = parse("20231215T140000Z", None);
    assert!(dt.is_utc && !dt.is_date);
    let z = dt.to_zoned();
    assert_eq!((z.year(), z.month(), z.day(), z.hour(), z.minute()), (2023, 12, 15, 14, 0));
    assert_eq!(dt.to_ical_string(), "20231215T140000Z");
    assert_eq!(dt.to_day_code(), "20231215");
}

#[test]
fn zoned_local_datetime() {
    let dt = parse("20231215T090000", Some("America/New_York"));
    assert_eq!(dt.timezone, Some(Tz::America__New_York));
    assert!(!dt.is_date);
    assert_eq!(dt.to_zoned().hour(), 9);
}

#[test]
fn all_day_date() {
    let dt = parse("20231215", None);
    assert!(dt.is_date);
    let z = dt.to_zoned();
    assert_eq!((z.year(), z.month(), z.day(), z.hour()), (2023, 12, 15, 0));
    assert_eq!(dt.to_ical_string(), "20231215");
    assert_eq!(parse("20240101", None).to_day_code(), "20240101");
}

#[test]
fn floating_datetime_uses_the_system_zone() {
    let dt = parse("20231215T140000", None);
    assert_eq!(dt.timezone, Some(system_zone()));
    assert!(!dt.is_date && !dt.is_utc);
}

#[test]
fn from_zoned() {
    let utc = Tz::UTC.with_ymd_and_hms(2023, 12, 15, 14, 30, 0).unwrap();
    let dt = ICalDateTime::from_zoned(utc, false);
    assert_eq!(dt.timezone, Some(Tz::UTC));
    assert!(!dt.is_date);
    assert_eq!(dt.to_zoned().timestamp_millis(), utc.timestamp_millis());

    let midnight = system_zone().with_ymd_and_hms(2023, 12, 15, 0, 0, 0).unwrap();
    let all_day = ICalDateTime::from_zoned(midnight, true);
    assert!(all_day.is_date);
    assert_eq!(all_day.to_ical_string(), "20231215");
}

#[test]
fn zone_conversion_preserves_the_instant() {
    let dt = parse("20231215T140000Z", None);
    let ny = Tz::America__New_York.timestamp_millis_opt(dt.timestamp).unwrap();
    assert_eq!(ICalDateTime::from_zoned(ny, false).timestamp, dt.timestamp);
}

#[test]
fn iana_and_windows_tzids_resolve() {
    assert_eq!(parse("20260115T140000", Some("America/New_York")).timestamp, utc_millis(2026, 1, 15, 19, 0));
    assert_eq!(parse("20260115T140000", Some("Eastern Standard Time")).timestamp, utc_millis(2026, 1, 15, 19, 0));
    assert_eq!(parse("20260115T143000", Some("India Standard Time")).timestamp, utc_millis(2026, 1, 15, 9, 0));
    assert_eq!(parse("20260115T140000", Some("W. Europe Standard Time")).timestamp, utc_millis(2026, 1, 15, 13, 0));
}

#[test]
fn unknown_tzid_falls_back_to_the_system_zone() {
    assert_eq!(parse("20260115T140000", Some("Nonexistent/Zone")).timezone, Some(system_zone()));
}

#[test]
fn windows_names_are_case_insensitive() {
    let expected = resolve_tzid("Eastern Standard Time");
    let offset = |tz: Tz| tz.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap().timestamp_millis();
    assert_eq!(offset(expected), utc_millis(2026, 1, 15, 17, 0));
    for name in ["eastern standard time", "EASTERN STANDARD TIME"] {
        assert_eq!(resolve_tzid(name), expected, "{name}");
    }
}
