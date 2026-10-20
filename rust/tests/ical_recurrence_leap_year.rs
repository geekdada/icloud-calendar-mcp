//! Port of icaldav-core `RRuleExpanderLeapYearTest`: February 29, short months
//! and BYSETPOS. The Kotlin BYSETPOS cases only asserted `size >= 0`; the exact
//! dates here were cross-checked against python-dateutil.

mod recurrence_support;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent};
use recurrence_support::*;

const WEEKDAYS: &str = "BYDAY=MO,TU,WE,TH,FR";

fn ny_midnight(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    instant(zdt(NY, y, m, d, 0, 0))
}

fn timed(rrule: &str, y: i32, m: u32, d: u32) -> ICalEvent {
    event(zoned(NY, y, m, d, 10, 0), Some(rule(rrule)))
}

fn all_day(rrule: &str, y: i32, m: u32, d: u32) -> ICalEvent {
    event(date(y, m, d), Some(rule(rrule)))
}

fn codes_between(e: &ICalEvent, from: (i32, u32, u32), to: (i32, u32, u32)) -> Vec<String> {
    day_codes(&expand_all(e, ny_midnight(from.0, from.1, from.2), ny_midnight(to.0, to.1, to.2)))
}

#[test]
fn yearly_on_february_29_only_occurs_in_leap_years() {
    let e = timed("FREQ=YEARLY;COUNT=3", 2024, 2, 29);
    assert_eq!(codes_between(&e, (2024, 1, 1), (2033, 12, 31)), ["20240229", "20280229", "20320229"]);
}

#[test]
fn monthly_on_day_29_skips_february_in_common_years() {
    let e = timed("FREQ=MONTHLY;BYMONTHDAY=29;COUNT=14", 2023, 1, 29);
    let codes = codes_between(&e, (2023, 1, 1), (2024, 3, 1));
    assert_eq!(codes.len(), 13);
    assert!(!codes.iter().any(|c| c.starts_with("202302")));
    assert_eq!(&codes[11..], ["20240129", "20240229"]);
}

#[test]
fn monthly_on_day_31_skips_months_without_31_days() {
    let e = timed("FREQ=MONTHLY;BYMONTHDAY=31;COUNT=12", 2024, 1, 31);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2024, 12, 31)),
        ["20240131", "20240331", "20240531", "20240731", "20240831", "20241031"]
    );
}

#[test]
fn last_day_of_month_follows_month_length() {
    let e = timed("FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=6", 2024, 1, 31);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2024, 7, 1)),
        ["20240131", "20240229", "20240331", "20240430", "20240531", "20240630"]
    );
}

#[test]
fn all_day_leap_day_birthday_recurs_every_four_years() {
    let e = all_day("FREQ=YEARLY;COUNT=10", 2000, 2, 29);
    let occ = expand_all(&e, ny_midnight(2000, 1, 1), ny_midnight(2040, 12, 31));
    let expected: Vec<String> = (0..10).map(|i| format!("{}0229", 2000 + 4 * i)).collect();
    assert_eq!(day_codes(&occ), expected);
    assert!(occ.iter().all(|o| o.dt_start.is_date));
}

#[test]
fn by_set_pos_1_is_first_weekday_of_month() {
    let e = timed(&format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=1;COUNT=6"), 2024, 1, 1);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2024, 7, 1)),
        ["20240101", "20240201", "20240301", "20240401", "20240501", "20240603"]
    );
}

#[test]
fn by_set_pos_minus_1_is_last_weekday_of_month() {
    let e = timed(&format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=-1;COUNT=6"), 2024, 1, 31);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2024, 7, 1)),
        ["20240131", "20240229", "20240329", "20240430", "20240531", "20240628"]
    );
}

#[test]
fn by_set_pos_2_is_second_weekday_of_month() {
    let e = timed(&format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=2;COUNT=6"), 2024, 1, 2);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2024, 7, 1)),
        ["20240102", "20240202", "20240304", "20240402", "20240502", "20240604"]
    );
}

#[test]
fn by_set_pos_first_and_last_weekday() {
    let e = timed(&format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=1,-1;COUNT=12"), 2024, 1, 1);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2024, 7, 1)),
        [
            "20240101", "20240131", "20240201", "20240229", "20240301", "20240329", "20240401", "20240430", "20240501",
            "20240531", "20240603", "20240628"
        ]
    );
}

#[test]
fn yearly_by_set_pos_us_election_day() {
    let e = all_day("FREQ=YEARLY;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8;BYSETPOS=1;COUNT=5", 2024, 11, 5);
    assert_eq!(
        codes_between(&e, (2024, 1, 1), (2030, 12, 31)),
        ["20241105", "20251104", "20261103", "20271102", "20281107"]
    );
}

fn utc_daily_at(hour: u32) -> Vec<String> {
    let e = event(
        ICalDateTime::utc(utc(&format!("2024-01-01T{hour:02}:00:00Z")).timestamp_millis()),
        Some(rule("FREQ=DAILY;COUNT=5")),
    );
    times_in(&expand_all(&e, utc("2024-01-01T00:00:00Z"), utc("2024-01-31T00:00:00Z")), Tz::UTC)
}

#[test]
fn utc_series_keeps_utc_hour() {
    assert_eq!(utc_daily_at(10), ["10:00"; 5]);
}

#[test]
fn utc_series_late_in_the_day_keeps_utc_hour() {
    assert_eq!(utc_daily_at(23), ["23:00"; 5]);
}
