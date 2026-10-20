//! Port of icaldav-core `RRuleExpanderComprehensiveTest`. Cases that only
//! asserted `size >= 1` now pin the exact dates.

mod recurrence_support;

use chrono::{DateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::ICalEvent;
use icloud_calendar_mcp::ical::expander::expand;
use recurrence_support::*;

fn ny_midnight(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    instant(zdt(NY, y, m, d, 0, 0))
}

/// `[first of month, +31 days]` in New York, as the Kotlin `rangeForMonth`.
fn month_window(y: i32, m: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = ny_midnight(y, m, 1);
    (start, start + TimeDelta::days(31))
}

fn ny_series(rrule: &str, (y, m, d): (i32, u32, u32), (h, min): (u32, u32)) -> ICalEvent {
    event(zoned(NY, y, m, d, h, min), Some(rule(rrule)))
}

fn codes(e: &ICalEvent, from: (i32, u32, u32), to: (i32, u32, u32)) -> Vec<String> {
    day_codes(&expand_all(e, ny_midnight(from.0, from.1, from.2), ny_midnight(to.0, to.1, to.2)))
}

fn december(e: &ICalEvent) -> Vec<ICalEvent> {
    let (start, end) = month_window(2023, 12);
    expand_all(e, start, end)
}

#[test]
fn daily_count() {
    let e = ny_series("FREQ=DAILY;COUNT=5", (2023, 12, 1), (10, 0));
    assert_eq!(day_codes(&december(&e)), ["20231201", "20231202", "20231203", "20231204", "20231205"]);
}

#[test]
fn daily_with_interval_skips_days() {
    let e = ny_series("FREQ=DAILY;INTERVAL=2;COUNT=5", (2023, 12, 1), (10, 0));
    assert_eq!(day_codes(&december(&e)), ["20231201", "20231203", "20231205", "20231207", "20231209"]);
}

#[test]
fn daily_until_stops_on_last_day() {
    let e = ny_series("FREQ=DAILY;UNTIL=20231210T235959Z", (2023, 12, 1), (10, 0));
    let occ = december(&e);
    assert_eq!(occ.len(), 10);
    assert_eq!(day_codes(&occ)[9], "20231210");
}

#[test]
fn weekly_count() {
    let e = ny_series("FREQ=WEEKLY;COUNT=4", (2023, 12, 4), (10, 0));
    assert_eq!(day_codes(&december(&e)), ["20231204", "20231211", "20231218", "20231225"]);
}

#[test]
fn weekly_on_monday_wednesday_friday() {
    let e = ny_series("FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=9", (2023, 12, 4), (10, 0));
    assert_eq!(
        day_codes(&december(&e)),
        ["20231204", "20231206", "20231208", "20231211", "20231213", "20231215", "20231218", "20231220", "20231222"]
    );
}

#[test]
fn weekly_every_two_weeks() {
    let e = ny_series("FREQ=WEEKLY;INTERVAL=2;COUNT=3", (2023, 12, 4), (10, 0));
    assert_eq!(codes(&e, (2023, 12, 1), (2024, 1, 31)), ["20231204", "20231218", "20240101"]);
}

#[test]
fn monthly_on_day_of_month() {
    let e = ny_series("FREQ=MONTHLY;COUNT=3", (2023, 12, 15), (10, 0));
    assert_eq!(codes(&e, (2023, 12, 1), (2024, 3, 31)), ["20231215", "20240115", "20240215"]);
}

#[test]
fn monthly_on_last_day() {
    let e = ny_series("FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=3", (2023, 12, 31), (10, 0));
    assert_eq!(codes(&e, (2023, 12, 1), (2024, 3, 31)), ["20231231", "20240131", "20240229"]);
}

#[test]
fn monthly_on_second_tuesday() {
    let e = ny_series("FREQ=MONTHLY;BYDAY=2TU;COUNT=3", (2023, 12, 12), (10, 0));
    assert_eq!(codes(&e, (2023, 12, 1), (2024, 3, 31)), ["20231212", "20240109", "20240213"]);
}

#[test]
fn monthly_on_last_friday() {
    let e = ny_series("FREQ=MONTHLY;BYDAY=-1FR;COUNT=3", (2023, 12, 29), (10, 0));
    assert_eq!(codes(&e, (2023, 12, 1), (2024, 3, 31)), ["20231229", "20240126", "20240223"]);
}

#[test]
fn yearly_on_same_date() {
    let e = ny_series("FREQ=YEARLY;COUNT=3", (2023, 12, 25), (10, 0));
    assert_eq!(codes(&e, (2023, 1, 1), (2026, 12, 31)), ["20231225", "20241225", "20251225"]);
}

#[test]
fn yearly_on_month_and_day() {
    let e = ny_series("FREQ=YEARLY;BYMONTH=7;BYMONTHDAY=4;COUNT=3", (2023, 7, 4), (10, 0));
    assert_eq!(codes(&e, (2023, 1, 1), (2026, 12, 31)), ["20230704", "20240704", "20250704"]);
}

#[test]
fn yearly_last_thursday_of_november() {
    // 2023-11-23 is the fourth, not the last, Thursday: DTSTART stays the first
    // instance and 2023-11-30 follows.
    let e = ny_series("FREQ=YEARLY;BYMONTH=11;BYDAY=-1TH;COUNT=3", (2023, 11, 23), (10, 0));
    assert_eq!(codes(&e, (2023, 1, 1), (2026, 12, 31)), ["20231123", "20231130", "20241128"]);
}

#[test]
fn single_exdate() {
    let mut e = ny_series("FREQ=DAILY;COUNT=5", (2023, 12, 1), (10, 0));
    e.exdates = vec![zoned(NY, 2023, 12, 3, 10, 0)];
    assert_eq!(day_codes(&december(&e)), ["20231201", "20231202", "20231204", "20231205"]);
}

#[test]
fn multiple_exdates() {
    let mut e = ny_series("FREQ=DAILY;COUNT=10", (2023, 12, 1), (10, 0));
    e.exdates = vec![zoned(NY, 2023, 12, 3, 10, 0), zoned(NY, 2023, 12, 5, 10, 0), zoned(NY, 2023, 12, 7, 10, 0)];
    assert_eq!(
        day_codes(&december(&e)),
        ["20231201", "20231202", "20231204", "20231206", "20231208", "20231209", "20231210"]
    );
}

#[test]
fn override_replaces_original_occurrence() {
    let mut master = ny_series("FREQ=DAILY;COUNT=5", (2023, 12, 1), (10, 0));
    master.uid = "recurring-meeting".into();
    master.summary = Some("Daily Standup".into());
    let mut moved = event(zoned(NY, 2023, 12, 3, 14, 0), None);
    moved.uid = "recurring-meeting".into();
    moved.summary = Some("Daily Standup (Moved)".into());
    moved.recurrence_id = Some(zoned(NY, 2023, 12, 3, 10, 0));

    let (start, end) = month_window(2023, 12);
    let occ = expand(&master, start, end, &[moved]).unwrap();
    assert_eq!(occ.len(), 5);
    assert_eq!(occ[2].summary.as_deref(), Some("Daily Standup (Moved)"));
    assert_eq!(times_in(&occ, NY), ["10:00", "10:00", "14:00", "10:00", "10:00"]);
}

#[test]
fn all_day_daily() {
    let e = event(date(2023, 12, 1), Some(rule("FREQ=DAILY;COUNT=5")));
    let occ = december(&e);
    assert_eq!(occ.len(), 5);
    assert!(occ.iter().all(|o| o.is_all_day && o.dt_start.is_date));
}

#[test]
fn daily_across_spring_forward_shifts_nonexistent_time_forward() {
    let e = ny_series("FREQ=DAILY;COUNT=3", (2024, 3, 9), (2, 30));
    let occ = expand_all(&e, ny_midnight(2024, 3, 8), ny_midnight(2024, 3, 15));
    assert_eq!(day_codes(&occ), ["20240309", "20240310", "20240311"]);
    // 02:30 does not exist on 2024-03-10; RFC 5545 §3.3.5 resolves it with the
    // pre-gap offset, i.e. 03:30 EDT.
    assert_eq!(times_in(&occ, NY), ["2:30", "3:30", "2:30"]);
}

#[test]
fn daily_across_fall_back() {
    let e = ny_series("FREQ=DAILY;COUNT=3", (2024, 11, 2), (1, 30));
    let occ = expand_all(&e, ny_midnight(2024, 11, 1), ny_midnight(2024, 11, 10));
    assert_eq!(day_codes(&occ), ["20241102", "20241103", "20241104"]);
    assert_eq!(times_in(&occ, NY), ["1:30"; 3]);
}

#[test]
fn weekly_keeps_local_time_across_dst() {
    let e = ny_series("FREQ=WEEKLY;COUNT=3", (2024, 3, 4), (10, 0));
    let occ = expand_all(&e, ny_midnight(2024, 3, 1), ny_midnight(2024, 3, 31));
    assert_eq!(day_codes(&occ), ["20240304", "20240311", "20240318"]);
    assert_eq!(times_in(&occ, NY), ["10:00"; 3]);
}

#[test]
fn all_day_yearly_keeps_date_regardless_of_zone() {
    let e = event(date(2023, 12, 25), Some(rule("FREQ=YEARLY;COUNT=3")));
    assert_eq!(codes(&e, (2023, 1, 1), (2026, 12, 31)), ["20231225", "20241225", "20251225"]);
}

#[test]
fn weekly_at_ambiguous_fall_back_time_uses_first_offset() {
    let e = ny_series("FREQ=WEEKLY;COUNT=3", (2024, 10, 27), (1, 30));
    let occ = expand_all(&e, ny_midnight(2024, 10, 25), ny_midnight(2024, 11, 15));
    assert_eq!(times_in(&occ, NY), ["1:30"; 3]);
    // 01:30 happens twice on 2024-11-03; the earlier (EDT) instant is used.
    assert_eq!(occ[1].dt_start.timestamp, utc("2024-11-03T05:30:00Z").timestamp_millis());
}

#[test]
fn overnight_event_keeps_exact_duration_across_dst() {
    let e = event_until(zoned(NY, 2024, 11, 2, 22, 0), zoned(NY, 2024, 11, 3, 6, 0), Some(rule("FREQ=DAILY;COUNT=3")));
    let occ = expand_all(&e, ny_midnight(2024, 11, 1), ny_midnight(2024, 11, 10));
    assert_eq!(occ.len(), 3);
    // RFC 5545 §3.8.5.3: a DTEND-specified duration is exact, here 9 hours
    // because the master spans the fall-back transition.
    for o in &occ {
        assert_eq!(o.dt_end.expect("DTEND").timestamp - o.dt_start.timestamp, 9 * 3_600_000);
    }
}

#[test]
fn first_monday_of_month() {
    let e = ny_series("FREQ=MONTHLY;BYDAY=1MO;COUNT=6", (2024, 1, 1), (10, 0));
    assert_eq!(
        codes(&e, (2024, 1, 1), (2024, 12, 31)),
        ["20240101", "20240205", "20240304", "20240401", "20240506", "20240603"]
    );
}

#[test]
fn third_wednesday_of_month() {
    let e = ny_series("FREQ=MONTHLY;BYDAY=3WE;COUNT=4", (2024, 1, 17), (10, 0));
    assert_eq!(codes(&e, (2024, 1, 1), (2024, 6, 30)), ["20240117", "20240221", "20240320", "20240417"]);
}

#[test]
fn second_to_last_thursday_of_month() {
    let e = ny_series("FREQ=MONTHLY;BYDAY=-2TH;COUNT=3", (2024, 1, 18), (10, 0));
    assert_eq!(codes(&e, (2024, 1, 1), (2024, 6, 30)), ["20240118", "20240222", "20240321"]);
}

#[test]
fn every_weekday() {
    let e = ny_series("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;COUNT=10", (2024, 1, 1), (9, 0));
    assert_eq!(
        codes(&e, (2024, 1, 1), (2024, 1, 31)),
        [
            "20240101", "20240102", "20240103", "20240104", "20240105", "20240108", "20240109", "20240110", "20240111",
            "20240112"
        ]
    );
}

#[test]
fn non_recurring_event_returns_itself() {
    let e = event(zoned(NY, 2023, 12, 15, 10, 0), None);
    assert_eq!(december(&e).len(), 1);
}

#[test]
fn non_recurring_event_outside_range_is_returned_unfiltered() {
    // `expand` leaves range filtering of single events to the caller.
    let e = event(zoned(NY, 2023, 11, 15, 10, 0), None);
    assert_eq!(day_codes(&december(&e)), ["20231115"]);
}

#[test]
fn duration_is_preserved_across_occurrences() {
    let e =
        event_until(zoned(NY, 2023, 12, 1, 10, 0), zoned(NY, 2023, 12, 1, 12, 30), Some(rule("FREQ=DAILY;COUNT=3")));
    let occ = december(&e);
    assert_eq!(occ.len(), 3);
    for o in &occ {
        assert_eq!(o.dt_end.expect("DTEND").timestamp - o.dt_start.timestamp, 150 * 60_000);
    }
}

#[test]
fn occurrence_starts_are_unique() {
    let e = ny_series("FREQ=DAILY;COUNT=5", (2023, 12, 1), (10, 0));
    let mut starts: Vec<i64> = december(&e).iter().map(|o| o.dt_start.timestamp).collect();
    starts.dedup();
    assert_eq!(starts.len(), 5);
}

#[test]
fn all_day_instance_at_window_end_is_included() {
    // An all-day instance that starts exactly at the window end is returned.
    let e = event(date(2024, 1, 1), Some(rule("FREQ=DAILY")));
    let occ = expand_all(&e, utc("2024-01-01T00:00:00Z"), utc("2024-01-03T00:00:00Z"));
    assert_eq!(day_codes(&occ), ["20240101", "20240102", "20240103"]);
    assert_eq!(times_in(&occ, Tz::UTC), ["0:00"; 3]);
}
