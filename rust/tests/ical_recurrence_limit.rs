//! Port of icaldav-core `RRuleExpanderLimitTest`: the [`MAX_ITERATIONS`]
//! per-series bound aborts pathological expansions instead of truncating them,
//! and far-seed series are fast-forwarded without shifting their instances.
//!
//! Windows include their end (as in ical4j), so the 100-day and one-week
//! windows yield 101 and 169 instances rather than icaldav's approximate ranges.

mod recurrence_support;

use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::expander::{ExpansionLimit, MAX_ITERATIONS, expand};
use icloud_calendar_mcp::ical::{ICalEvent, RRule};
use recurrence_support::*;

const TIME_BOUND: Duration = Duration::from_secs(10);

fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    instant(zdt(Tz::UTC, y, m, d, 10, 0))
}

fn series(value: &str, y: i32, m: u32, d: u32) -> ICalEvent {
    event(utc_dt(y, m, d, 10, 0), Some(rule(value)))
}

fn timed_expand(e: &ICalEvent, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<ICalEvent>, ExpansionLimit> {
    let started = Instant::now();
    let result = expand(e, start, end, &[]);
    assert!(started.elapsed() < TIME_BOUND, "expansion took {:?}", started.elapsed());
    result
}

#[test]
fn daily_over_100_days_returns_every_occurrence() {
    let e = series("FREQ=DAILY", 2026, 1, 1);
    let start = at(2026, 1, 1);
    assert_eq!(expand_all(&e, start, start + TimeDelta::days(100)).len(), 101);
}

#[test]
fn near_seed_secondly_over_three_days_aborts() {
    let e = series("FREQ=SECONDLY", 2026, 1, 1);
    let start = at(2026, 1, 1);
    let err = expand(&e, start, start + TimeDelta::days(3), &[]).unwrap_err();
    assert_eq!(err.limit, MAX_ITERATIONS);
}

#[test]
fn exactly_max_iterations_does_not_abort() {
    let e = series(&format!("FREQ=SECONDLY;COUNT={MAX_ITERATIONS}"), 2026, 1, 1);
    let start = at(2026, 1, 1);
    assert_eq!(expand_all(&e, start, start + TimeDelta::days(2)).len(), MAX_ITERATIONS);
}

#[test]
fn one_past_max_iterations_aborts() {
    let e = series(&format!("FREQ=SECONDLY;COUNT={}", MAX_ITERATIONS + 1), 2026, 1, 1);
    let start = at(2026, 1, 1);
    assert!(expand(&e, start, start + TimeDelta::days(2), &[]).is_err());
}

#[test]
fn near_seed_secondly_multi_day_aborts_fast() {
    let e = series("FREQ=SECONDLY", 2026, 6, 1);
    assert!(timed_expand(&e, at(2026, 6, 1), at(2026, 6, 5)).is_err());
}

#[test]
fn far_seed_secondly_from_1970_aborts_fast() {
    let e = series("FREQ=SECONDLY", 1970, 1, 1);
    assert!(timed_expand(&e, at(2026, 3, 2), at(2026, 3, 5)).is_err());
}

#[test]
fn far_seed_hourly_over_one_week_does_not_abort() {
    let e = series("FREQ=HOURLY", 2020, 1, 1);
    let occ = timed_expand(&e, at(2026, 3, 2), at(2026, 3, 9)).unwrap();
    assert_eq!(occ.len(), 169);
    assert_eq!(occ[0].dt_start.timestamp, at(2026, 3, 2).timestamp_millis());
}

#[test]
fn far_seed_monthly_on_day_31_keeps_day_31() {
    let e = series("FREQ=MONTHLY", 2019, 1, 31);
    let occ = timed_expand(&e, at(2026, 3, 1), at(2026, 12, 31)).unwrap();
    assert_eq!(dates(&occ), ["2026-03-31", "2026-05-31", "2026-07-31", "2026-08-31", "2026-10-31", "2026-12-31"]);
}

#[test]
fn far_seed_monthly_interval_stays_aligned_to_dtstart() {
    let e = series("FREQ=MONTHLY;INTERVAL=5", 2019, 1, 31);
    let occ = timed_expand(&e, at(2026, 3, 1), at(2026, 12, 31)).unwrap();
    assert_eq!(dates(&occ), ["2026-07-31", "2026-12-31"]);
}

#[test]
fn far_seed_yearly_on_february_29_skips_common_years() {
    let e = series("FREQ=YEARLY", 2000, 2, 29);
    let occ = timed_expand(&e, at(2027, 1, 1), at(2033, 1, 1)).unwrap();
    assert_eq!(dates(&occ), ["2028-02-29", "2032-02-29"]);
}

#[test]
fn far_seed_yearly_interval_with_by_day_stays_aligned() {
    let e = series("FREQ=YEARLY;INTERVAL=3;BYMONTH=3;BYDAY=-1SU", 2001, 3, 25);
    let occ = timed_expand(&e, at(2026, 1, 1), at(2032, 1, 1)).unwrap();
    assert_eq!(dates(&occ), ["2028-03-26", "2031-03-30"]);
}

#[test]
fn far_seed_count_series_ends_before_window() {
    // COUNT is not fast-forwarded: a far-seed COUNT series is walked from DTSTART
    // and must still stop at its last instance.
    let e = event(utc_dt(1990, 1, 1, 10, 0), Some(RRule { count: Some(3), ..rule("FREQ=YEARLY") }));
    assert!(expand_all(&e, at(2026, 1, 1), at(2027, 1, 1)).is_empty());
}
