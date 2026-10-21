//! Port of icaldav-core `RRuleAdvancedPatternsTest`.
//!
//! The Kotlin suite only checked that these rules parse; each case here also
//! expands the rule and checks the occurrence dates (verified against
//! python-dateutil for synchronized DTSTARTs).

mod recurrence_support;

use chrono::{TimeDelta, Weekday};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::{Frequency, ICalDateTime, RRule};
use recurrence_support::*;

fn expand_ics(ics: &str, years: i64) -> Vec<String> {
    let e = parse_one(ics);
    let start = e.dt_start.to_utc();
    dates(&expand_all(&e, start, start + TimeDelta::days(366 * years)))
}

fn one_event(uid: &str, dtstart: &str, rrule: &str) -> String {
    vcal(&format!("BEGIN:VEVENT\nUID:{uid}\nDTSTART:{dtstart}\nRRULE:{rrule}\nSUMMARY:{uid}\nEND:VEVENT\n"))
}

fn expand_rule(r: &str, start: ICalDateTime, years: i64) -> Vec<String> {
    let e = event(start, Some(rule(r)));
    dates(&expand_all(&e, start.to_utc(), start.to_utc() + TimeDelta::days(366 * years)))
}

// ───────────── BYSETPOS ─────────────

#[test]
fn bysetpos_parses() {
    let r = rule("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=3");
    assert_eq!(r.freq, Frequency::Monthly);
    assert_eq!(r.by_set_pos, Some(vec![-1]));
    assert_eq!(rule("FREQ=MONTHLY;BYDAY=MO;BYSETPOS=1,-1;COUNT=6").by_set_pos, Some(vec![1, -1]));
    assert_eq!(rule("FREQ=MONTHLY;BYDAY=FR;BYSETPOS=-2;COUNT=3").by_set_pos, Some(vec![-2]));
}

#[test]
fn bysetpos_minus_one_selects_last_weekday() {
    let ics = one_event(
        "bysetpos-last@test.com",
        "20240131T100000Z",
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=3",
    );
    assert_eq!(parse_one(&ics).rrule.unwrap().by_set_pos, Some(vec![-1]));
    assert_eq!(expand_ics(&ics, 1), ["2024-01-31", "2024-02-29", "2024-03-29"]);
}

#[test]
fn bysetpos_first_and_last_monday() {
    assert_eq!(
        expand_rule("FREQ=MONTHLY;BYDAY=MO;BYSETPOS=1,-1;COUNT=6", utc_dt(2024, 1, 1, 10, 0), 1),
        ["2024-01-01", "2024-01-29", "2024-02-05", "2024-02-26", "2024-03-04", "2024-03-25"]
    );
}

#[test]
fn bysetpos_second_to_last_friday() {
    assert_eq!(
        expand_rule("FREQ=MONTHLY;BYDAY=FR;BYSETPOS=-2;COUNT=3", utc_dt(2024, 1, 19, 10, 0), 1),
        ["2024-01-19", "2024-02-16", "2024-03-22"]
    );
}

#[test]
fn bysetpos_first_workday_of_month() {
    let ics = one_event(
        "first-workday@test.com",
        "20240101T090000Z",
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=1;COUNT=12",
    );
    let r = parse_one(&ics).rrule.unwrap();
    assert_eq!(r.by_day.as_ref().map(Vec::len), Some(5));
    assert_eq!(r.by_set_pos, Some(vec![1]));
    assert_eq!(
        expand_ics(&ics, 1),
        [
            "2024-01-01",
            "2024-02-01",
            "2024-03-01",
            "2024-04-01",
            "2024-05-01",
            "2024-06-03",
            "2024-07-01",
            "2024-08-01",
            "2024-09-02",
            "2024-10-01",
            "2024-11-01",
            "2024-12-02"
        ]
    );
}

#[test]
fn unsynchronized_dtstart_counts_as_first_instance() {
    // Kotlin's case started on Jan 2 (the first workday is Jan 1). RFC 5545
    // §3.3.10: DTSTART always counts as the first occurrence.
    let got = expand_rule("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=1;COUNT=3", utc_dt(2024, 1, 2, 9, 0), 1);
    assert_eq!(got, ["2024-01-02", "2024-02-01", "2024-03-01"]);
}

// ───────────── BYYEARDAY ─────────────

#[test]
fn byyearday_positive_values() {
    assert_eq!(rule("FREQ=YEARLY;BYYEARDAY=1,100,200;COUNT=9").by_year_day, Some(vec![1, 100, 200]));
    assert_eq!(
        expand_rule("FREQ=YEARLY;BYYEARDAY=1,100,200;COUNT=9", utc_dt(2024, 1, 1, 10, 0), 4),
        [
            "2024-01-01",
            "2024-04-09",
            "2024-07-18",
            "2025-01-01",
            "2025-04-10",
            "2025-07-19",
            "2026-01-01",
            "2026-04-10",
            "2026-07-19"
        ]
    );
}

#[test]
fn byyearday_minus_one_is_dec_31() {
    assert_eq!(rule("FREQ=YEARLY;BYYEARDAY=-1;COUNT=3").by_year_day, Some(vec![-1]));
    assert_eq!(
        expand_rule("FREQ=YEARLY;BYYEARDAY=-1;COUNT=3", utc_dt(2024, 12, 31, 10, 0), 4),
        ["2024-12-31", "2025-12-31", "2026-12-31"]
    );
}

#[test]
fn byyearday_from_event_parsing() {
    let ics = one_event("yearday@test.com", "20240101T000000Z", "FREQ=YEARLY;BYYEARDAY=1,365;COUNT=6");
    assert_eq!(parse_one(&ics).rrule.unwrap().by_year_day, Some(vec![1, 365]));
    // 2024 is a leap year, so day 365 is Dec 30.
    assert_eq!(
        expand_ics(&ics, 4),
        ["2024-01-01", "2024-12-30", "2025-01-01", "2025-12-31", "2026-01-01", "2026-12-31"]
    );
}

#[test]
fn byyearday_last_seven_days() {
    let r = rule("FREQ=YEARLY;BYYEARDAY=-7,-6,-5,-4,-3,-2,-1;COUNT=14");
    assert_eq!(r.by_year_day.as_ref().unwrap().len(), 7);
    let got = expand_rule("FREQ=YEARLY;BYYEARDAY=-7,-6,-5,-4,-3,-2,-1;COUNT=14", utc_dt(2024, 12, 25, 10, 0), 3);
    assert_eq!(got.len(), 14);
    assert_eq!(got.first().unwrap(), "2024-12-25");
    assert_eq!(got.last().unwrap(), "2025-12-31");
}

// ───────────── BYWEEKNO ─────────────

#[test]
fn byweekno_one_monday_spills_into_previous_year() {
    assert_eq!(rule("FREQ=YEARLY;BYWEEKNO=1;BYDAY=MO;COUNT=3").by_week_no, Some(vec![1]));
    // Week 1 of 2025 starts Monday 2024-12-30.
    assert_eq!(
        expand_rule("FREQ=YEARLY;BYWEEKNO=1;BYDAY=MO;COUNT=3", utc_dt(2024, 1, 1, 10, 0), 4),
        ["2024-01-01", "2024-12-30", "2025-12-29"]
    );
}

#[test]
fn byweekno_minus_one_friday() {
    assert_eq!(rule("FREQ=YEARLY;BYWEEKNO=1,52;COUNT=6").by_week_no, Some(vec![1, 52]));
    assert_eq!(rule("FREQ=YEARLY;BYWEEKNO=-1;BYDAY=FR;COUNT=3").by_week_no, Some(vec![-1]));
    // 2026 has 53 weeks; its last week's Friday is 2027-01-01.
    assert_eq!(
        expand_rule("FREQ=YEARLY;BYWEEKNO=-1;BYDAY=FR;COUNT=3", utc_dt(2024, 12, 27, 10, 0), 4),
        ["2024-12-27", "2025-12-26", "2027-01-01"]
    );
}

#[test]
fn byweekno_twenty_meetings() {
    let ics = one_event("weekno@test.com", "20240513T100000Z", "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO,WE,FR;COUNT=9");
    assert_eq!(parse_one(&ics).rrule.unwrap().by_week_no, Some(vec![20]));
    assert_eq!(
        expand_ics(&ics, 4),
        [
            "2024-05-13",
            "2024-05-15",
            "2024-05-17",
            "2025-05-12",
            "2025-05-14",
            "2025-05-16",
            "2026-05-11",
            "2026-05-13",
            "2026-05-15"
        ]
    );
}

// ───────────── WKST ─────────────

#[test]
fn wkst_defaults_to_monday_and_parses() {
    assert_eq!(rule("FREQ=WEEKLY;COUNT=10").wkst, Weekday::Mon);
    assert_eq!(rule("FREQ=WEEKLY;WKST=SU;COUNT=10").wkst, Weekday::Sun);
    assert_eq!(rule("FREQ=YEARLY;BYWEEKNO=1;WKST=MO;COUNT=1").wkst, Weekday::Mon);
    assert_eq!(rule("FREQ=YEARLY;BYWEEKNO=1;WKST=SU;COUNT=1").wkst, Weekday::Sun);
}

#[test]
fn biweekly_tue_thu_with_sunday_week_start() {
    let ics = one_event("wkst@test.com", "20240109T100000Z", "FREQ=WEEKLY;INTERVAL=2;WKST=SU;BYDAY=TU,TH;COUNT=8");
    assert_eq!(parse_one(&ics).rrule.unwrap().wkst, Weekday::Sun);
    assert_eq!(
        expand_ics(&ics, 1),
        [
            "2024-01-09",
            "2024-01-11",
            "2024-01-23",
            "2024-01-25",
            "2024-02-06",
            "2024-02-08",
            "2024-02-20",
            "2024-02-22"
        ]
    );
}

// ───────────── complex combinations ─────────────

#[test]
fn monthly_last_friday() {
    let ics = one_event("last-friday@test.com", "20240126T170000Z", "FREQ=MONTHLY;BYDAY=-1FR;COUNT=12");
    let r = parse_one(&ics).rrule.unwrap();
    assert_eq!(r.by_day, Some(vec![nth(-1, Weekday::Fri)]));
    assert_eq!(expand_ics(&ics, 1)[..3], ["2024-01-26", "2024-02-23", "2024-03-29"]);
}

#[test]
fn yearly_third_monday_of_november() {
    let ics = one_event("third-monday-nov@test.com", "20231120T090000Z", "FREQ=YEARLY;BYMONTH=11;BYDAY=3MO;COUNT=5");
    let r = parse_one(&ics).rrule.unwrap();
    assert_eq!(r.by_month, Some(vec![11]));
    assert_eq!(r.by_day.as_ref().unwrap()[0].ordinal, Some(3));
    assert_eq!(expand_ics(&ics, 5), ["2023-11-20", "2024-11-18", "2025-11-17", "2026-11-16", "2027-11-15"]);
}

#[test]
fn us_thanksgiving_fourth_thursday() {
    let ics = one_event("thanksgiving@test.com", "20231123T000000", "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH;COUNT=10");
    let r = parse_one(&ics).rrule.unwrap();
    assert_eq!(r.by_day, Some(vec![nth(4, Weekday::Thu)]));
    assert_eq!(
        expand_ics(&ics, 11),
        [
            "2023-11-23",
            "2024-11-28",
            "2025-11-27",
            "2026-11-26",
            "2027-11-25",
            "2028-11-23",
            "2029-11-22",
            "2030-11-28",
            "2031-11-27",
            "2032-11-25"
        ]
    );
}

#[test]
fn last_day_of_every_month() {
    let ics = one_event("last-day@test.com", "20240131T170000Z", "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=12");
    assert_eq!(parse_one(&ics).rrule.unwrap().by_month_day, Some(vec![-1]));
    assert_eq!(expand_ics(&ics, 1)[..4], ["2024-01-31", "2024-02-29", "2024-03-31", "2024-04-30"]);
}

#[test]
fn mid_and_end_of_month() {
    let ics = one_event("multi-monthday@test.com", "20240115T100000Z", "FREQ=MONTHLY;BYMONTHDAY=15,-1;COUNT=24");
    let by = parse_one(&ics).rrule.unwrap().by_month_day.unwrap();
    assert!(by.contains(&15) && by.contains(&-1));
    assert_eq!(
        expand_ics(&ics, 2)[..6],
        ["2024-01-15", "2024-01-31", "2024-02-15", "2024-02-29", "2024-03-15", "2024-03-31"]
    );
}

#[test]
fn every_friday_the_13th() {
    let ics = one_event("friday-13@test.com", "20241213T130000Z", "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13;COUNT=10");
    let r = parse_one(&ics).rrule.unwrap();
    assert_eq!(r.by_month_day, Some(vec![13]));
    assert_eq!(r.by_day.as_ref().unwrap()[0].weekday, Weekday::Fri);
    assert_eq!(
        expand_ics(&ics, 7),
        [
            "2024-12-13",
            "2025-06-13",
            "2026-02-13",
            "2026-03-13",
            "2026-11-13",
            "2027-08-13",
            "2028-10-13",
            "2029-04-13",
            "2029-07-13",
            "2030-09-13"
        ]
    );
}

// ───────────── serialization ─────────────

#[test]
fn bysetpos_serializes() {
    let r = RRule {
        by_day: Some(vec![wd(Weekday::Mon), wd(Weekday::Fri)]),
        by_set_pos: Some(vec![-1]),
        count: Some(12),
        ..RRule::new(Frequency::Monthly)
    };
    let s = r.to_ical_string();
    assert!(s.contains("BYSETPOS=-1"));
    assert_eq!(RRule::parse(&s).unwrap().by_set_pos, Some(vec![-1]));
}

#[test]
fn byyearday_serializes() {
    let r = RRule { by_year_day: Some(vec![1, 100, -1]), count: Some(9), ..RRule::new(Frequency::Yearly) };
    let s = r.to_ical_string();
    assert!(s.contains("BYYEARDAY=1,100,-1"));
    assert_eq!(RRule::parse(&s).unwrap().by_year_day, Some(vec![1, 100, -1]));
}

#[test]
fn byweekno_serializes() {
    let r = RRule { by_week_no: Some(vec![1, 26, 52]), count: Some(9), ..RRule::new(Frequency::Yearly) };
    let s = r.to_ical_string();
    assert!(s.contains("BYWEEKNO=1,26,52"));
    assert_eq!(RRule::parse(&s).unwrap().by_week_no, Some(vec![1, 26, 52]));
}

#[test]
fn wkst_serializes() {
    let r = RRule { wkst: Weekday::Sun, count: Some(10), ..RRule::new(Frequency::Weekly) };
    let s = r.to_ical_string();
    assert!(s.contains("WKST=SU"));
    assert_eq!(RRule::parse(&s).unwrap().wkst, Weekday::Sun);
}

#[test]
fn complex_rule_serializes() {
    let r = RRule {
        by_month: Some(vec![11]),
        by_day: Some(vec![nth(4, Weekday::Thu)]),
        count: Some(10),
        ..RRule::new(Frequency::Yearly)
    };
    let s = r.to_ical_string();
    assert!(s.contains("FREQ=YEARLY") && s.contains("BYMONTH=11") && s.contains("BYDAY=4TH"));
    let back = RRule::parse(&s).unwrap();
    assert_eq!(back.freq, Frequency::Yearly);
    assert_eq!(back.by_month, Some(vec![11]));
    assert_eq!(back.by_day.unwrap()[0].ordinal, Some(4));
}

// ───────────── CalConnect edge cases ─────────────

#[test]
fn single_rrule_per_component() {
    let ics = one_event("single-rrule@test.com", "20240101T100000Z", "FREQ=WEEKLY;COUNT=10");
    assert!(parse_one(&ics).rrule.is_some());
}

#[test]
fn dtstart_defines_first_instance() {
    let ics = one_event("dtstart-pattern@test.com", "20240115T100000Z", "FREQ=MONTHLY;BYMONTHDAY=15;COUNT=12");
    let e = parse_one(&ics);
    assert_eq!(e.dt_start.to_ical_string(), "20240115T100000Z");
    assert_eq!(expand_ics(&ics, 1)[0], "2024-01-15");
}

#[test]
fn exception_event_has_no_rrule() {
    let ics = vcal(
        "BEGIN:VEVENT\nUID:master@test.com\nDTSTART:20240101T100000Z\nRRULE:FREQ=DAILY;COUNT=10\nSUMMARY:Master\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:master@test.com\nRECURRENCE-ID:20240105T100000Z\nDTSTART:20240105T140000Z\nSUMMARY:Exception - Moved\nEND:VEVENT\n",
    );
    let events = parse(&ics);
    let exception = events.iter().find(|e| e.recurrence_id.is_some()).unwrap();
    assert!(exception.rrule.is_none());
}

#[test]
fn count_and_until_both_parse() {
    let r = rule("FREQ=DAILY;COUNT=5;UNTIL=20241231T235959Z");
    assert_eq!(r.count, Some(5));
    assert!(r.until.is_some());
    // Both bounds apply; COUNT is the tighter one here.
    let got = expand_rule("FREQ=DAILY;COUNT=5;UNTIL=20241231T235959Z", utc_dt(2024, 1, 1, 10, 0), 1);
    assert_eq!(got.len(), 5);
}

// ───────────── frequencies ─────────────

#[test]
fn sub_daily_frequencies_parse() {
    let s = rule("FREQ=SECONDLY;INTERVAL=30;COUNT=10");
    assert_eq!((s.freq, s.interval), (Frequency::Secondly, 30));
    let m = rule("FREQ=MINUTELY;INTERVAL=15;COUNT=8");
    assert_eq!((m.freq, m.interval), (Frequency::Minutely, 15));
    let h = rule("FREQ=HOURLY;INTERVAL=2;COUNT=12");
    assert_eq!((h.freq, h.interval), (Frequency::Hourly, 2));
}

#[test]
fn all_frequencies_parse_from_events_and_expand_three() {
    let freqs = [
        ("SECONDLY", Frequency::Secondly),
        ("MINUTELY", Frequency::Minutely),
        ("HOURLY", Frequency::Hourly),
        ("DAILY", Frequency::Daily),
        ("WEEKLY", Frequency::Weekly),
        ("MONTHLY", Frequency::Monthly),
        ("YEARLY", Frequency::Yearly),
    ];
    for (name, freq) in freqs {
        let ics = one_event(&format!("freq-{name}@test.com"), "20240101T100000Z", &format!("FREQ={name};COUNT=3"));
        let e = parse_one(&ics);
        assert_eq!(e.rrule.as_ref().unwrap().freq, freq);
        let start = e.dt_start.to_utc();
        assert_eq!(expand_all(&e, start, start + TimeDelta::days(1200)).len(), 3, "{name}");
    }
}

#[test]
fn floating_dtstart_expands_in_its_zone() {
    let e = parse_one(&one_event("float@test.com", "20240101T100000", "FREQ=DAILY;COUNT=2"));
    assert!(!e.dt_start.is_utc && e.dt_start.timezone.is_some());
    let start = e.dt_start.to_utc();
    let occ = expand_all(&e, start, start + TimeDelta::days(3));
    let tz: Tz = e.dt_start.timezone.unwrap();
    assert_eq!(times_in(&occ, tz), ["10:00", "10:00"]);
}
