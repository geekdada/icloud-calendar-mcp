//! Port of icaldav-core `RRuleExpanderAdversarialTest`.
//!
//! Where the Kotlin suite only asserted "does not crash" (`size >= 0`), these
//! pin the exact result. Series whose DTSTART the rule does not generate keep
//! DTSTART as their first instance (RFC 5545 §3.3.10), so the "6th Monday" and
//! "Feb 30" cases yield one occurrence instead of icaldav's zero.

mod recurrence_support;

use std::time::{Duration, Instant};

use chrono::{DateTime, Months, TimeDelta, Utc, Weekday};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::expander::{ExpansionLimit, MAX_ITERATIONS, expand};
use icloud_calendar_mcp::ical::{Frequency, ICalDateTime, ICalEvent, RRule};
use recurrence_support::*;

fn default_start() -> ICalDateTime {
    utc_dt(2024, 1, 1, 10, 0)
}

fn range_years(years: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = utc("2024-01-01T10:00:00Z");
    (start - TimeDelta::days(1), start.checked_add_months(Months::new(12 * years)).unwrap())
}

fn expand_years(e: &ICalEvent, years: u32) -> Vec<ICalEvent> {
    let (start, end) = range_years(years);
    expand_all(e, start, end)
}

fn try_expand_years(e: &ICalEvent, years: u32) -> Result<Vec<ICalEvent>, ExpansionLimit> {
    let (start, end) = range_years(years);
    expand(e, start, end, &[])
}

fn daily() -> RRule {
    RRule::new(Frequency::Daily)
}

#[test]
fn null_rrule_returns_single_occurrence() {
    assert_eq!(expand_years(&event(default_start(), None), 1).len(), 1);
}

#[test]
fn empty_by_day_list_falls_back_to_dtstart_weekday() {
    let r = RRule { count: Some(5), by_day: Some(vec![]), ..RRule::new(Frequency::Weekly) };
    let occ = expand_years(&event(default_start(), Some(r)), 1);
    assert_eq!(day_codes(&occ), ["20240101", "20240108", "20240115", "20240122", "20240129"]);
}

#[test]
fn zero_interval_is_treated_as_one() {
    let r = RRule { interval: 0, count: Some(5), ..daily() };
    assert_eq!(expand_years(&event(default_start(), Some(r)), 1).len(), 5);
}

#[test]
fn negative_interval_is_treated_as_one() {
    let r = RRule { interval: -1, count: Some(5), ..daily() };
    assert_eq!(expand_years(&event(default_start(), Some(r)), 1).len(), 5);
}

#[test]
fn very_large_interval_leaves_only_dtstart_in_window() {
    let r = RRule { interval: 999_999, count: Some(5), ..daily() };
    assert_eq!(day_codes(&expand_years(&event(default_start(), Some(r)), 10)), ["20240101"]);
}

#[test]
fn zero_count_generates_nothing() {
    let r = RRule { count: Some(0), ..daily() };
    assert!(expand_years(&event(default_start(), Some(r)), 1).is_empty());
}

#[test]
fn negative_count_generates_nothing() {
    let r = RRule { count: Some(-5), ..daily() };
    assert!(expand_years(&event(default_start(), Some(r)), 1).is_empty());
}

#[test]
fn infinite_daily_recurrence_over_a_century_aborts() {
    let err = try_expand_years(&event(default_start(), Some(daily())), 100).unwrap_err();
    assert_eq!(err.limit, MAX_ITERATIONS);
    assert_eq!(err.uid, "test-event");
}

#[test]
fn secondly_with_count_completes_quickly_within_limit() {
    let r = RRule { count: Some(10_000), ..RRule::new(Frequency::Secondly) };
    let started = Instant::now();
    let occ = expand_years(&event(default_start(), Some(r)), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(occ.len(), 10_000);
}

#[test]
fn unbounded_minutely_over_a_year_aborts() {
    assert!(try_expand_years(&event(default_start(), Some(RRule::new(Frequency::Minutely))), 1).is_err());
}

#[test]
fn until_in_the_past_generates_nothing_in_future_range() {
    let r = RRule { until: Some(utc_dt(1970, 1, 1, 0, 0)), ..daily() };
    assert!(expand_years(&event(default_start(), Some(r)), 1).is_empty());
}

#[test]
fn until_before_dtstart_generates_nothing() {
    let r = RRule { until: Some(utc_dt(2023, 12, 31, 10, 0)), ..daily() };
    assert!(expand_years(&event(default_start(), Some(r)), 1).is_empty());
}

#[test]
fn sixth_monday_never_exists_so_only_dtstart_remains() {
    let r = RRule { count: Some(12), by_day: Some(vec![nth(6, Weekday::Mon)]), ..RRule::new(Frequency::Monthly) };
    assert_eq!(day_codes(&expand_years(&event(default_start(), Some(r)), 1)), ["20240101"]);
}

#[test]
fn last_friday_of_month() {
    let r = RRule { count: Some(12), by_day: Some(vec![nth(-1, Weekday::Fri)]), ..RRule::new(Frequency::Monthly) };
    let occ = expand_years(&event(utc_dt(2024, 1, 26, 10, 0), Some(r)), 1);
    assert_eq!(occ.len(), 12);
    assert_eq!(&day_codes(&occ)[..3], ["20240126", "20240223", "20240329"]);
    assert_eq!(day_codes(&occ)[11], "20241227");
}

#[test]
fn by_day_on_other_weekday_keeps_dtstart_then_follows_rule() {
    let r = RRule { count: Some(5), by_day: Some(vec![wd(Weekday::Mon)]), ..RRule::new(Frequency::Weekly) };
    let occ = expand_years(&event(utc_dt(2024, 1, 2, 10, 0), Some(r)), 1);
    assert_eq!(day_codes(&occ), ["20240102", "20240108", "20240115", "20240122", "20240129"]);
}

#[test]
fn by_month_day_31_skips_short_months() {
    let r = RRule { count: Some(12), by_month_day: Some(vec![31]), ..RRule::new(Frequency::Monthly) };
    let occ = expand_years(&event(default_start(), Some(r)), 1);
    assert_eq!(
        day_codes(&occ),
        ["20240101", "20240131", "20240331", "20240531", "20240731", "20240831", "20241031", "20241231"]
    );
}

#[test]
fn february_30_never_exists_so_only_dtstart_remains() {
    let r = RRule {
        count: Some(5),
        by_month: Some(vec![2]),
        by_month_day: Some(vec![30]),
        ..RRule::new(Frequency::Yearly)
    };
    assert_eq!(day_codes(&expand_years(&event(default_start(), Some(r)), 10)), ["20240101"]);
}

#[test]
fn february_29_only_in_leap_years() {
    let r = RRule {
        count: Some(10),
        by_month: Some(vec![2]),
        by_month_day: Some(vec![29]),
        ..RRule::new(Frequency::Yearly)
    };
    let occ = expand_years(&event(default_start(), Some(r)), 50);
    assert_eq!(occ.len(), 10);
    assert_eq!(&day_codes(&occ)[..3], ["20240101", "20240229", "20280229"]);
    assert_eq!(day_codes(&occ)[9], "20560229");
}

#[test]
fn exdate_excludes_exact_occurrences() {
    let mut e = event(default_start(), Some(RRule { count: Some(5), ..daily() }));
    e.exdates = vec![utc_dt(2024, 1, 2, 10, 0), utc_dt(2024, 1, 4, 10, 0)];
    assert_eq!(day_codes(&expand_years(&e, 1)), ["20240101", "20240103", "20240105"]);
}

#[test]
fn excluding_every_occurrence_leaves_nothing() {
    let mut e = event(default_start(), Some(RRule { count: Some(3), ..daily() }));
    e.exdates = vec![utc_dt(2024, 1, 1, 10, 0), utc_dt(2024, 1, 2, 10, 0), utc_dt(2024, 1, 3, 10, 0)];
    assert!(expand_years(&e, 1).is_empty());
}

#[test]
fn all_day_occurrences_stay_dates_at_utc_midnight() {
    let mut e = event(date(2024, 1, 1), Some(RRule { count: Some(5), ..daily() }));
    e.is_all_day = true;
    let occ = expand_years(&e, 1);
    assert_eq!(occ.len(), 5);
    assert!(occ.iter().all(|o| o.dt_start.is_date));
    assert_eq!(times_in(&occ, Tz::UTC), ["0:00"; 5]);
}

#[test]
fn zoned_series_keeps_local_wall_time() {
    let e = event(zoned(NY, 2024, 1, 1, 10, 0), Some(RRule { count: Some(5), ..daily() }));
    let start = instant(zdt(NY, 2023, 12, 31, 10, 0));
    let occ = expand_all(&e, start, instant(zdt(NY, 2025, 1, 1, 10, 0)));
    assert_eq!(times_in(&occ, NY), ["10:00"; 5]);
}

#[test]
fn by_set_pos_second_tuesday() {
    let r = RRule {
        count: Some(12),
        by_day: Some(vec![wd(Weekday::Tue)]),
        by_set_pos: Some(vec![2]),
        ..RRule::new(Frequency::Monthly)
    };
    let occ = expand_years(&event(default_start(), Some(r)), 1);
    // DTSTART (a Monday) plus eleven second Tuesdays.
    assert_eq!(occ.len(), 12);
    assert_eq!(&day_codes(&occ)[..4], ["20240101", "20240109", "20240213", "20240312"]);
    assert_eq!(day_codes(&occ)[11], "20241112");
}

#[test]
fn wkst_with_sunday_by_day() {
    let r = RRule {
        count: Some(4),
        by_day: Some(vec![wd(Weekday::Sun)]),
        wkst: Weekday::Mon,
        ..RRule::new(Frequency::Weekly)
    };
    let occ = expand_years(&event(default_start(), Some(r)), 1);
    assert_eq!(day_codes(&occ), ["20240101", "20240107", "20240114", "20240121"]);
}
