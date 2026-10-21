//! Port of icaldav-core `ICalDateTimeTimezoneTest` (VALUE=DATE handling).
//!
//! Kotlin `fromLocalDate(date, zone)` maps to `from_local_date(date)` for UTC
//! and to `from_zoned(local midnight in zone, true)` for other zones.

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::ICalDateTime;

const DAY_MS: i64 = 86_400_000;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn parse(value: &str) -> ICalDateTime {
    ICalDateTime::parse(value, None).unwrap()
}

fn jan23_utc_midnight() -> i64 {
    date(2026, 1, 23).and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
}

#[test]
fn date_is_utc_midnight_without_zone() {
    let dt = parse("20260123");
    assert_eq!(dt.timestamp, jan23_utc_midnight());
    assert!(dt.is_utc);
    assert_eq!(dt.timezone, None);
}

#[test]
fn date_accessors_return_the_calendar_date() {
    let dt = parse("20260123");
    assert_eq!(dt.to_local_date(), date(2026, 1, 23));
    assert_eq!(dt.to_day_code(), "20260123");
    assert_eq!(dt.to_ical_string(), "20260123");
}

#[test]
fn exclusive_end_and_consecutive_days() {
    assert_eq!(parse("20260125").to_local_date(), date(2026, 1, 25));
    assert_eq!(parse("20260124").timestamp - parse("20260123").timestamp, DAY_MS);
    let (dec31, jan01) = (parse("20251231"), parse("20260101"));
    assert_eq!((dec31.to_local_date(), jan01.to_local_date()), (date(2025, 12, 31), date(2026, 1, 1)));
    assert_eq!(jan01.timestamp - dec31.timestamp, DAY_MS);
}

#[test]
fn leap_day_old_and_far_future_dates() {
    for (value, expected) in
        [("20240229", date(2024, 2, 29)), ("19700101", date(1970, 1, 1)), ("21001231", date(2100, 12, 31))]
    {
        let dt = parse(value);
        assert_eq!(dt.to_local_date(), expected);
        assert_eq!(dt.to_day_code(), value);
        assert_eq!(dt.to_ical_string(), value);
    }
}

#[test]
fn from_local_date_matches_parse() {
    let dt = ICalDateTime::from_local_date(date(2026, 1, 23));
    assert!(dt.is_date);
    assert_eq!(dt.timestamp, jan23_utc_midnight());
    assert_eq!(dt.timestamp, parse("20260123").timestamp);
}

#[test]
fn zoned_midnight_keeps_the_calendar_date_in_every_zone() {
    for zone in [
        Tz::UTC,
        Tz::America__New_York,
        Tz::America__Los_Angeles,
        Tz::Pacific__Honolulu,
        Tz::Europe__London,
        Tz::Europe__Paris,
        Tz::Asia__Tokyo,
        Tz::Pacific__Auckland,
    ] {
        let midnight = zone.with_ymd_and_hms(2026, 1, 23, 0, 0, 0).unwrap();
        let dt = ICalDateTime::from_zoned(midnight, true);
        assert!(dt.is_date);
        assert_eq!(dt.to_local_date(), date(2026, 1, 23), "{zone}");
        assert_eq!(dt.to_day_code(), "20260123", "{zone}");
        assert_eq!(dt.to_ical_string(), "20260123", "{zone}");
    }
}

#[test]
fn from_timestamp_dates_read_in_utc() {
    let dt = ICalDateTime::from_timestamp(jan23_utc_midnight(), None, true);
    assert_eq!(dt.to_local_date(), date(2026, 1, 23));
    assert_eq!(dt.to_ical_string(), "20260123");

    // Local midnight in New York is 05:00 UTC, still Jan 23 when read in UTC.
    let est_midnight = Tz::America__New_York.with_ymd_and_hms(2026, 1, 23, 0, 0, 0).unwrap().timestamp_millis();
    let est = ICalDateTime::from_timestamp(est_midnight, Some(Tz::America__New_York), true);
    assert_eq!(est.to_local_date(), date(2026, 1, 23));
    assert_eq!(est.to_ical_string(), "20260123");
}
