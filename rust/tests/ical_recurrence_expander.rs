//! Port of icaldav-core `RRuleExpanderTest`.

mod recurrence_support;

use chrono::Weekday;
use icloud_calendar_mcp::ical::expander::expand;
use icloud_calendar_mcp::ical::{Frequency, ICalDateTime, RRule};
use recurrence_support::*;

fn daily(count: i32) -> RRule {
    RRule { count: Some(count), ..RRule::new(Frequency::Daily) }
}

#[test]
fn non_recurring_event_returns_itself() {
    let e = event(zoned(NY, 2023, 12, 15, 10, 0), None);
    let (start, end) = month_range(NY, 2023, 12);
    let occ = expand_all(&e, start, end);
    assert_eq!(occ.len(), 1);
    assert_eq!(occ[0].uid, e.uid);
}

#[test]
fn daily_recurrence_generates_correct_occurrences() {
    let e = event(zoned(NY, 2023, 12, 1, 10, 0), Some(daily(5)));
    let (start, end) = month_range(NY, 2023, 12);
    assert_eq!(day_codes(&expand_all(&e, start, end)), ["20231201", "20231202", "20231203", "20231204", "20231205"]);
}

#[test]
fn weekly_recurrence_on_specific_days() {
    let r = RRule {
        count: Some(6),
        by_day: Some(vec![wd(Weekday::Mon), wd(Weekday::Wed), wd(Weekday::Fri)]),
        ..RRule::new(Frequency::Weekly)
    };
    let e = event(zoned(NY, 2023, 12, 4, 10, 0), Some(r));
    let (start, end) = month_range(NY, 2023, 12);
    assert_eq!(
        day_codes(&expand_all(&e, start, end)),
        ["20231204", "20231206", "20231208", "20231211", "20231213", "20231215"]
    );
}

#[test]
fn exdate_excludes_occurrences() {
    let mut e = event(zoned(NY, 2023, 12, 1, 10, 0), Some(daily(5)));
    // 15:00Z is the 10:00 New York occurrence.
    e.exdates = vec![
        ICalDateTime::parse("20231202T150000Z", None).unwrap(),
        ICalDateTime::parse("20231204T150000Z", None).unwrap(),
    ];
    let (start, end) = month_range(NY, 2023, 12);
    assert_eq!(day_codes(&expand_all(&e, start, end)), ["20231201", "20231203", "20231205"]);
}

#[test]
fn overrides_replace_original_occurrences() {
    let mut master = event(zoned(NY, 2023, 12, 1, 10, 0), Some(daily(5)));
    master.uid = "recurring-123".into();
    master.summary = Some("Daily Standup".into());
    let mut over = event(zoned(NY, 2023, 12, 3, 15, 0), None);
    over.uid = "recurring-123".into();
    over.summary = Some("Daily Standup (Rescheduled)".into());
    over.recurrence_id = Some(ICalDateTime::parse("20231203T150000Z", None).unwrap());

    let (start, end) = month_range(NY, 2023, 12);
    let occ = expand(&master, start, end, &[over]).unwrap();
    assert_eq!(occ.len(), 5);
    let dec3 = occ.iter().find(|o| o.dt_start.to_day_code() == "20231203").unwrap();
    assert_eq!(dec3.summary.as_deref(), Some("Daily Standup (Rescheduled)"));
}

#[test]
fn occurrences_have_distinct_starts() {
    // Kotlin asserted unique importIds; the Rust model identifies an occurrence by its start.
    let e = event(zoned(NY, 2023, 12, 1, 10, 0), Some(daily(5)));
    let (start, end) = month_range(NY, 2023, 12);
    let occ = expand_all(&e, start, end);
    let mut starts: Vec<i64> = occ.iter().map(|o| o.dt_start.timestamp).collect();
    starts.dedup();
    assert_eq!(starts.len(), 5);
    assert!(occ.iter().all(|o| o.uid == "test-event" && o.recurrence_id.is_none()));
}

#[test]
fn occurrences_preserve_event_duration() {
    let e = event_until(zoned(NY, 2023, 12, 1, 10, 0), zoned(NY, 2023, 12, 1, 11, 30), Some(daily(3)));
    let (start, end) = month_range(NY, 2023, 12);
    for o in expand_all(&e, start, end) {
        assert_eq!(o.dt_end.unwrap().timestamp - o.dt_start.timestamp, 90 * 60 * 1000);
    }
}

#[test]
fn monthly_recurrence_on_specific_day() {
    let r = RRule { count: Some(3), by_month_day: Some(vec![15]), ..RRule::new(Frequency::Monthly) };
    let e = event(zoned(NY, 2023, 10, 15, 10, 0), Some(r));
    let occ = expand_all(&e, utc("2023-01-01T00:00:00Z"), utc("2024-12-31T00:00:00Z"));
    assert_eq!(day_codes(&occ), ["20231015", "20231115", "20231215"]);
}

#[test]
fn daily_byhour_produces_three_occurrences_per_day() {
    let r = RRule { count: Some(6), by_hour: Some(vec![9, 12, 15]), ..RRule::new(Frequency::Daily) };
    let e = event(zoned(NY, 2025, 1, 1, 9, 0), Some(r));
    let (start, end) = month_range(NY, 2025, 1);
    let occ = expand_all(&e, start, end);
    assert_eq!(times_in(&occ, NY), ["9:00", "12:00", "15:00", "9:00", "12:00", "15:00"]);
    assert_eq!(day_codes(&occ), ["20250101", "20250101", "20250101", "20250102", "20250102", "20250102"]);
}

#[test]
fn minutely_interval_15_count_6() {
    // RFC 5545 §3.8.5.3: every 15 minutes for 6 occurrences.
    let r = RRule { interval: 15, count: Some(6), ..RRule::new(Frequency::Minutely) };
    let e = event(zoned(NY, 2025, 1, 1, 9, 0), Some(r));
    let (start, end) = month_range(NY, 2025, 1);
    assert_eq!(times_in(&expand_all(&e, start, end), NY), ["9:00", "9:15", "9:30", "9:45", "10:00", "10:15"]);
}

#[test]
fn daily_byhour_byminute_grid() {
    let r = RRule {
        count: Some(6),
        by_hour: Some(vec![9]),
        by_minute: Some(vec![0, 20, 40]),
        ..RRule::new(Frequency::Daily)
    };
    let e = event(zoned(NY, 2025, 1, 1, 9, 0), Some(r));
    let (start, end) = month_range(NY, 2025, 1);
    assert_eq!(times_in(&expand_all(&e, start, end), NY), ["9:00", "9:20", "9:40", "9:00", "9:20", "9:40"]);
}

#[test]
fn daily_two_hours_two_minutes_grid() {
    let r = RRule {
        count: Some(8),
        by_hour: Some(vec![9, 12]),
        by_minute: Some(vec![0, 30]),
        ..RRule::new(Frequency::Daily)
    };
    let e = event(zoned(NY, 2025, 1, 1, 9, 0), Some(r));
    let (start, end) = month_range(NY, 2025, 1);
    assert_eq!(
        times_in(&expand_all(&e, start, end), NY),
        ["9:00", "9:30", "12:00", "12:30", "9:00", "9:30", "12:00", "12:30"]
    );
}

#[test]
fn rfc_example_36_every_20_minutes_9_to_16_40() {
    // RFC 5545 §3.8.5.3: FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40
    let e = event(zoned(NY, 1997, 9, 2, 9, 0), Some(rule("FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40")));
    let occ = expand_all(&e, instant(zdt(NY, 1997, 9, 2, 0, 0)), instant(zdt(NY, 1997, 9, 2, 23, 59)));
    assert_eq!(occ.len(), 24);
    assert_eq!(times_in(&occ[..3], NY), ["9:00", "9:20", "9:40"]);
    assert_eq!(times_in(&occ[23..], NY), ["16:40"]);
}
