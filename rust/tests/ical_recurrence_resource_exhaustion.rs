//! Port of icaldav-core `RecurrenceResourceExhaustionTest` (CWE-400): adversarial
//! rules either finish quickly or abort at the expansion bound.
//!
//! The Kotlin suite logged counts and asserted upper bounds; with inclusive
//! window ends the exact counts are pinned here.

mod recurrence_support;

use std::time::{Duration, Instant};

use chrono::{DateTime, Months, TimeDelta, Utc};
use icloud_calendar_mcp::ical::expander::{ExpansionLimit, expand};
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent};
use recurrence_support::*;

const TIME_BOUND: Duration = Duration::from_secs(10);

fn default_start() -> DateTime<Utc> {
    utc("2024-01-01T10:00:00Z")
}

fn at_offset(delta: TimeDelta) -> ICalDateTime {
    ICalDateTime::utc((default_start() + delta).timestamp_millis())
}

fn series(rrule: &str) -> ICalEvent {
    event(at_offset(TimeDelta::zero()), Some(rule(rrule)))
}

fn years_later(years: u32) -> DateTime<Utc> {
    default_start().checked_add_months(Months::new(12 * years)).unwrap()
}

fn timed_expand(e: &ICalEvent, end: DateTime<Utc>, overrides: &[ICalEvent]) -> Result<Vec<ICalEvent>, ExpansionLimit> {
    let started = Instant::now();
    let result = expand(e, default_start(), end, overrides);
    assert!(started.elapsed() < TIME_BOUND, "expansion took {:?}", started.elapsed());
    result
}

#[test]
fn unbounded_secondly_over_one_hour_completes() {
    let occ = timed_expand(&series("FREQ=SECONDLY"), default_start() + TimeDelta::hours(1), &[]).unwrap();
    assert_eq!(occ.len(), 3601);
}

#[test]
fn unbounded_minutely_over_one_day_completes() {
    let occ = timed_expand(&series("FREQ=MINUTELY"), default_start() + TimeDelta::days(1), &[]).unwrap();
    assert_eq!(occ.len(), 1441);
}

#[test]
fn unbounded_hourly_over_a_century_aborts() {
    assert!(timed_expand(&series("FREQ=HOURLY"), years_later(100), &[]).is_err());
}

#[test]
fn unbounded_daily_over_a_millennium_aborts() {
    assert!(timed_expand(&series("FREQ=DAILY"), years_later(1000), &[]).is_err());
}

#[test]
fn count_of_50000_aborts_before_materializing() {
    assert!(timed_expand(&series("FREQ=DAILY;COUNT=50000"), years_later(150), &[]).is_err());
}

#[test]
fn five_thousand_exdates_are_applied_quickly() {
    let mut e = series("FREQ=DAILY;COUNT=10000");
    e.exdates = (1..=5000).map(|day| at_offset(TimeDelta::days(day * 2))).collect();
    let occ = timed_expand(&e, years_later(30), &[]).unwrap();
    // Offsets 2, 4, …, 9998 are excluded; offset 10000 is past COUNT.
    assert_eq!(occ.len(), 10_000 - 4_999);
}

#[test]
fn five_thousand_overrides_are_applied_quickly() {
    let master = series("FREQ=DAILY;COUNT=10000");
    let overrides: Vec<ICalEvent> = (1..=5000)
        .map(|day| {
            let mut o = event(at_offset(TimeDelta::days(day) + TimeDelta::hours(2)), None);
            o.uid = master.uid.clone();
            o.summary = Some(format!("Override {day}"));
            o.recurrence_id = Some(at_offset(TimeDelta::days(day)));
            o
        })
        .collect();
    let occ = timed_expand(&master, years_later(30), &overrides).unwrap();
    assert_eq!(occ.len(), 10_000);
    let replaced = occ.iter().filter(|o| o.summary.as_deref().is_some_and(|s| s.starts_with("Override"))).count();
    assert_eq!(replaced, 5000);
}

#[test]
fn monthly_weekdays_with_many_set_positions_completes() {
    let occ =
        timed_expand(&series("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=1,2,3,4,5,-1,-2"), years_later(100), &[])
            .unwrap();
    // Seven distinct positions in each of 1200 months (2124-01-01 is a Saturday).
    assert_eq!(occ.len(), 7 * 1200);
}

#[test]
fn yearly_every_week_number_with_by_day_completes() {
    let weeks: Vec<String> = (1..=53).map(|w| w.to_string()).collect();
    let rrule = format!("FREQ=YEARLY;BYWEEKNO={};BYDAY=MO,FR", weeks.join(","));
    let occ = timed_expand(&series(&rrule), years_later(50), &[]).unwrap();
    // Every Monday and Friday of the 50 years (count cross-checked with python-dateutil).
    assert_eq!(occ.len(), 5219);
}

#[test]
fn yearly_every_year_day_completes() {
    let days: Vec<String> = (1..=366).map(|d| d.to_string()).collect();
    let occ =
        timed_expand(&series(&format!("FREQ=YEARLY;BYYEARDAY={}", days.join(","))), years_later(10), &[]).unwrap();
    // Every day from 2024-01-01 to 2034-01-01 inclusive; day 366 only exists in leap years.
    assert_eq!(occ.len(), 3654);
}

#[test]
fn secondly_with_count_respects_count() {
    let occ = timed_expand(&series("FREQ=SECONDLY;COUNT=5000"), default_start() + TimeDelta::hours(2), &[]).unwrap();
    assert_eq!(occ.len(), 5000);
}

#[test]
fn zero_width_range_at_an_occurrence_returns_it() {
    let occ = expand_all(&series("FREQ=DAILY;COUNT=100"), default_start(), default_start());
    assert_eq!(day_codes(&occ), ["20240101"]);
}

#[test]
fn zero_width_range_between_occurrences_is_empty() {
    let instant = default_start() + TimeDelta::hours(5);
    assert!(expand_all(&series("FREQ=DAILY;COUNT=100"), instant, instant).is_empty());
}

#[test]
fn range_before_series_start_is_empty() {
    let start = default_start().checked_sub_months(Months::new(120)).unwrap();
    let end = default_start().checked_sub_months(Months::new(60)).unwrap();
    assert!(expand_all(&series("FREQ=DAILY;COUNT=100"), start, end).is_empty());
}

#[test]
fn range_after_until_is_empty() {
    let mut e = series("FREQ=DAILY");
    e.rrule.as_mut().unwrap().until = Some(at_offset(TimeDelta::days(30)));
    assert!(expand_all(&e, years_later(1), years_later(2)).is_empty());
}

#[test]
fn far_future_range_completes() {
    let started = Instant::now();
    let occ = expand_all(&series("FREQ=YEARLY"), utc("9990-01-01T00:00:00Z"), utc("9999-12-31T23:59:59Z"));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(occ.len(), 10);
    assert_eq!(day_codes(&occ)[0], "99900101");
}
