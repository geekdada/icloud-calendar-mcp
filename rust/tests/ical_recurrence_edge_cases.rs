//! Port of icaldav-core `RRuleEdgeCaseTest`.
//!
//! The Kotlin range end was exclusive; the Rust expander's is inclusive, so
//! where an occurrence would land exactly on the end the window is shortened
//! by a second to keep the asserted set identical.

mod recurrence_support;

use chrono::{Datelike, Offset, TimeDelta, Timelike, Weekday};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::expander::expand;
use icloud_calendar_mcp::ical::{Frequency, ICalDateTime, RRule};
use recurrence_support::*;

fn hour() -> TimeDelta {
    TimeDelta::hours(1)
}

fn counted(freq: Frequency, count: i32) -> RRule {
    RRule { count: Some(count), ..RRule::new(freq) }
}

// ───────────── DST transitions ─────────────

#[test]
fn daily_rule_across_spring_forward() {
    let start = zdt(NY, 2024, 3, 8, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 5)));
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(6)));
    assert_eq!(occ.len(), 5);
    assert!(occ.iter().all(|o| o.dt_start.to_zoned().hour() == 10), "hour preserved across DST");
}

#[test]
fn daily_rule_across_fall_back() {
    let start = zdt(NY, 2024, 11, 1, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 5)));
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(6)));
    assert_eq!(occ.len(), 5);
    assert!(occ.iter().all(|o| o.dt_start.to_zoned().hour() == 10));
}

#[test]
fn weekly_rule_preserves_local_time_across_dst() {
    let london = Tz::Europe__London;
    let start = zdt(london, 2024, 10, 21, 14, 30);
    let e =
        event_for(ICalDateTime::from_zoned(start, false), TimeDelta::minutes(90), Some(counted(Frequency::Weekly, 3)));
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::weeks(4)));
    assert_eq!(times_in(&occ, london), ["14:30", "14:30", "14:30"]);
}

// ───────────── leap years ─────────────

#[test]
fn yearly_rule_on_feb_29_skips_non_leap_years() {
    let start = zdt(LOCAL, 2024, 2, 29, 12, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Yearly, 4)));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2029, 2, 28, 12, 0)));
    // RFC 5545: an invalid date (Feb 29 in a common year) is skipped, not moved.
    assert_eq!(dates(&occ), ["2024-02-29", "2028-02-29"]);
}

#[test]
fn monthly_bymonthday_29_includes_leap_february() {
    let start = zdt(LOCAL, 2024, 1, 29, 10, 0);
    let r = RRule { by_month_day: Some(vec![29]), ..counted(Frequency::Monthly, 12) };
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(r));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2025, 1, 29, 10, 0)));
    assert_eq!(occ.len(), 12);
    assert!(dates(&occ).contains(&"2024-02-29".to_string()));
    assert!(occ.iter().all(|o| local_day(o) == 29));
}

#[test]
fn monthly_bymonthday_30_skips_february() {
    let start = zdt(LOCAL, 2024, 1, 30, 10, 0);
    let r = RRule { by_month_day: Some(vec![30]), ..counted(Frequency::Monthly, 12) };
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(r));
    let end = instant(zdt(LOCAL, 2025, 1, 30, 10, 0)) - TimeDelta::seconds(1);
    let occ = expand_all(&e, instant(start), end);
    assert_eq!(occ.len(), 11);
    assert!(occ.iter().all(|o| local_day(o) == 30));
}

#[test]
fn monthly_bymonthday_31_only_in_long_months() {
    let start = zdt(LOCAL, 2024, 1, 31, 10, 0);
    let r = RRule { by_month_day: Some(vec![31]), ..counted(Frequency::Monthly, 12) };
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(r));
    let end = instant(zdt(LOCAL, 2025, 1, 31, 10, 0)) - TimeDelta::seconds(1);
    let occ = expand_all(&e, instant(start), end);
    assert_eq!(
        dates(&occ),
        ["2024-01-31", "2024-03-31", "2024-05-31", "2024-07-31", "2024-08-31", "2024-10-31", "2024-12-31"]
    );
}

// ───────────── last day of month ─────────────

#[test]
fn monthly_bymonthday_minus_one_is_last_day() {
    let start = zdt(LOCAL, 2024, 1, 31, 10, 0);
    let r = RRule { by_month_day: Some(vec![-1]), ..counted(Frequency::Monthly, 6) };
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(r));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2024, 8, 31, 10, 0)));
    assert_eq!(dates(&occ), ["2024-01-31", "2024-02-29", "2024-03-31", "2024-04-30", "2024-05-31", "2024-06-30"]);
}

#[test]
fn monthly_byday_minus_one_friday() {
    let start = zdt(LOCAL, 2024, 1, 26, 10, 0);
    let r = RRule { by_day: Some(vec![nth(-1, Weekday::Fri)]), ..counted(Frequency::Monthly, 6) };
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(r));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2024, 8, 26, 10, 0)));
    assert_eq!(dates(&occ), ["2024-01-26", "2024-02-23", "2024-03-29", "2024-04-26", "2024-05-31", "2024-06-28"]);
    assert!(occ.iter().all(|o| o.dt_start.to_local_date().weekday() == Weekday::Fri));
}

#[test]
fn monthly_fifth_monday_only_in_months_that_have_one() {
    let start = zdt(LOCAL, 2024, 1, 29, 10, 0);
    let r = RRule { by_day: Some(vec![nth(5, Weekday::Mon)]), ..counted(Frequency::Monthly, 12) };
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(r));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2025, 1, 29, 10, 0)));
    assert_eq!(dates(&occ), ["2024-01-29", "2024-04-29", "2024-07-29", "2024-09-30", "2024-12-30"]);
}

// ───────────── EXDATE ─────────────

#[test]
fn exdate_excludes_specific_occurrence() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 5)));
    e.exdates = vec![zoned(LOCAL, 2024, 1, 3, 10, 0)];
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(6)));
    assert_eq!(day_codes(&occ), ["20240101", "20240102", "20240104", "20240105"]);
}

#[test]
fn multiple_exdates_exclude_multiple_occurrences() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 7)));
    e.exdates = [2, 4, 6].iter().map(|d| zoned(LOCAL, 2024, 1, *d, 10, 0)).collect();
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(8)));
    assert_eq!(day_codes(&occ), ["20240101", "20240103", "20240105", "20240107"]);
}

#[test]
fn exdate_on_all_day_event_matches_by_date() {
    let start = date(2024, 1, 1);
    let mut e = event(start, Some(counted(Frequency::Daily, 5)));
    e.exdates = vec![date(2024, 1, 3)];
    let occ = expand_all(&e, start.to_utc(), start.to_utc() + TimeDelta::days(6));
    assert_eq!(day_codes(&occ), ["20240101", "20240102", "20240104", "20240105"]);
}

// ───────────── RDATE ─────────────

#[test]
fn rdate_adds_additional_occurrence() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Weekly, 2)));
    e.rdates = vec![zoned(LOCAL, 2024, 1, 10, 10, 0)];
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(15)));
    assert_eq!(day_codes(&occ), ["20240101", "20240108", "20240110"]);
}

#[test]
fn rdate_without_rrule_keeps_dtstart_as_first_instance() {
    // Adapted: Kotlin dropped DTSTART for an RDATE-only series; RFC 5545 §3.8.5.2
    // makes the recurrence set DTSTART ∪ RDATE, so the start is an instance too.
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), None);
    e.rdates = vec![zoned(LOCAL, 2024, 1, 5, 10, 0), zoned(LOCAL, 2024, 1, 15, 14, 0), zoned(LOCAL, 2024, 2, 1, 9, 0)];
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2024, 3, 1, 10, 0)));
    assert_eq!(day_codes(&occ), ["20240101", "20240105", "20240115", "20240201"]);
}

#[test]
fn rdate_duplicating_rrule_occurrence_is_deduplicated() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Weekly, 3)));
    e.rdates = vec![zoned(LOCAL, 2024, 1, 8, 10, 0)];
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(20)));
    assert_eq!(day_codes(&occ), ["20240101", "20240108", "20240115"]);
}

#[test]
fn rdate_on_same_day_at_other_time_is_kept() {
    // Deduplication is by instant: an extra session later on an RRULE day is a
    // distinct instance (Kotlin deduplicated by day and dropped it).
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Weekly, 2)));
    e.rdates = vec![zoned(LOCAL, 2024, 1, 8, 15, 0)];
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(15)));
    assert_eq!(times_in(&occ, LOCAL), ["10:00", "10:00", "15:00"]);
}

#[test]
fn exdate_removes_rdate_occurrence() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Weekly, 2)));
    e.rdates = vec![zoned(LOCAL, 2024, 1, 10, 10, 0)];
    e.exdates = vec![zoned(LOCAL, 2024, 1, 10, 10, 0)];
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(15)));
    assert_eq!(day_codes(&occ), ["20240101", "20240108"]);
}

// ───────────── RECURRENCE-ID overrides ─────────────

#[test]
fn override_replaces_generated_occurrence() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let mut master = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 5)));
    master.uid = "master-event".into();
    let mut over = event_for(zoned(LOCAL, 2024, 1, 3, 14, 0), TimeDelta::hours(2), None);
    over.uid = "master-event".into();
    over.recurrence_id = Some(zoned(LOCAL, 2024, 1, 3, 10, 0));
    over.summary = Some("Modified Event".into());
    let occ = expand(&master, instant(start), instant(start + TimeDelta::days(6)), &[over]).unwrap();
    assert_eq!(occ.len(), 5);
    let jan3 = occ.iter().find(|o| o.dt_start.to_day_code() == "20240103").unwrap();
    assert_eq!(jan3.summary.as_deref(), Some("Modified Event"));
    assert_eq!(jan3.dt_start.to_zoned().hour(), 14);
}

#[test]
fn non_override_events_never_replace_occurrences() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let master = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 3)));
    let mut regular = event(zoned(LOCAL, 2024, 1, 2, 10, 0), None);
    regular.summary = Some("Regular".into());
    let occ = expand(&master, instant(start), instant(start + TimeDelta::days(5)), &[regular]).unwrap();
    assert!(occ.iter().all(|o| o.summary.as_deref() == Some("Test Event")));
}

// ───────────── large occurrence sets ─────────────

#[test]
fn daily_count_365_generates_full_year() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 365)));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2025, 1, 2, 10, 0)));
    assert_eq!(occ.len(), 365);
}

#[test]
fn open_weekly_rule_over_one_year() {
    let start = zdt(LOCAL, 2024, 1, 1, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(RRule::new(Frequency::Weekly)));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2025, 1, 1, 10, 0)));
    assert!((52..=53).contains(&occ.len()), "{}", occ.len());
}

#[test]
fn monthly_rule_for_ten_years() {
    let start = zdt(LOCAL, 2024, 1, 15, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Monthly, 120)));
    let occ = expand_all(&e, instant(start), instant(zdt(LOCAL, 2035, 1, 15, 10, 0)));
    assert_eq!(occ.len(), 120);
}

// ───────────── time zones ─────────────

#[test]
fn utc_event_expands_regardless_of_local_zone() {
    let start = zdt(Tz::UTC, 2024, 1, 1, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 3)));
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(5)));
    assert_eq!(times_in(&occ, Tz::UTC), ["10:00", "10:00", "10:00"]);
}

#[test]
fn tokyo_event_keeps_tokyo_wall_time() {
    let tokyo = Tz::Asia__Tokyo;
    let start = zdt(tokyo, 2024, 1, 1, 9, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Daily, 3)));
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::days(5)));
    assert_eq!(times_in(&occ, tokyo), ["9:00", "9:00", "9:00"]);
}

#[test]
fn shanghai_weekly_keeps_fixed_offset() {
    let shanghai = Tz::Asia__Shanghai;
    let start = zdt(shanghai, 2024, 6, 15, 10, 30);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Weekly, 4)));
    let occ = expand_all(&e, instant(start), instant(start + TimeDelta::weeks(5)));
    assert_eq!(times_in(&occ, shanghai), ["10:30", "10:30", "10:30", "10:30"]);
}

#[test]
fn shanghai_monthly_offset_is_always_plus_eight() {
    let shanghai = Tz::Asia__Shanghai;
    let start = zdt(shanghai, 2024, 1, 15, 8, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), Some(counted(Frequency::Monthly, 12)));
    let occ = expand_all(&e, instant(start), instant(zdt(shanghai, 2025, 1, 15, 8, 0)));
    assert_eq!(occ.len(), 12);
    for o in &occ {
        let local = o.dt_start.to_utc().with_timezone(&shanghai);
        assert_eq!(local.hour(), 8);
        assert_eq!(local.offset().fix().local_minus_utc(), 8 * 3600);
    }
}

// ───────────── all-day ─────────────

#[test]
fn all_day_event_expands_to_dates() {
    let start = date(2024, 1, 1);
    let e = event(start, Some(counted(Frequency::Daily, 3)));
    let occ = expand_all(&e, start.to_utc(), start.to_utc() + TimeDelta::days(5));
    assert_eq!(occ.len(), 3);
    assert!(occ.iter().all(|o| o.is_all_day && o.dt_start.is_date));
}

#[test]
fn weekly_all_day_on_specific_days() {
    let start = date(2024, 1, 1);
    let r = RRule {
        by_day: Some(vec![wd(Weekday::Mon), wd(Weekday::Wed), wd(Weekday::Fri)]),
        ..counted(Frequency::Weekly, 9)
    };
    let e = event(start, Some(r));
    let occ = expand_all(&e, start.to_utc(), start.to_utc() + TimeDelta::weeks(4));
    assert_eq!(occ.len(), 9);
    assert!(
        occ.iter().all(|o| matches!(o.dt_start.to_local_date().weekday(), Weekday::Mon | Weekday::Wed | Weekday::Fri))
    );
}

// ───────────── non-recurring ─────────────

#[test]
fn event_without_rrule_returns_single_occurrence() {
    let start = zdt(LOCAL, 2024, 1, 15, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), None);
    let occ = expand_all(&e, instant(start - TimeDelta::days(1)), instant(start + TimeDelta::days(5)));
    assert_eq!(occ.len(), 1);
    assert_eq!(occ[0].dt_start.timestamp, start.timestamp_millis());
}

#[test]
fn non_recurring_event_outside_range_is_returned_unfiltered() {
    // Range filtering of single events is the caller's job.
    let start = zdt(LOCAL, 2024, 1, 15, 10, 0);
    let e = event_for(ICalDateTime::from_zoned(start, false), hour(), None);
    let occ = expand_all(&e, instant(start + TimeDelta::days(5)), instant(start + TimeDelta::days(10)));
    assert_eq!(occ.len(), 1);
}
