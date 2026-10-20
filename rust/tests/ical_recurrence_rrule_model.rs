//! Port of icaldav-core `model/RRuleTest`: RRULE construction, parsing,
//! serialization and round trips (RFC 5545 §3.3.10).
//!
//! Kotlin's data-class `copy` and default-argument constructor cases map to
//! struct-update syntax over `RRule::new`.

mod recurrence_support;

use chrono::Weekday;
use icloud_calendar_mcp::ical::{Frequency, ICalDateTime, RRule, WeekdayNum};
use recurrence_support::*;

fn base(freq: Frequency) -> RRule {
    RRule::new(freq)
}

fn assert_round_trip(rrule: &RRule) {
    assert_eq!(&rule(&rrule.to_ical_string()), rrule);
}

// ---- construction ----

#[test]
fn new_rule_has_defaults() {
    let r = base(Frequency::Daily);
    assert_eq!(r.freq, Frequency::Daily);
    assert_eq!(r.interval, 1);
    assert_eq!(r.wkst, Weekday::Mon);
    assert!(r.count.is_none() && r.until.is_none());
    assert!(r.by_day.is_none() && r.by_month_day.is_none() && r.by_month.is_none());
    assert!(r.by_week_no.is_none() && r.by_year_day.is_none() && r.by_set_pos.is_none());
    assert!(r.by_hour.is_none() && r.by_minute.is_none() && r.by_second.is_none());
}

#[test]
fn construct_with_until() {
    let until = ICalDateTime::parse("20241231T235959Z", None).unwrap();
    let r = RRule { until: Some(until), ..base(Frequency::Yearly) };
    assert!(r.count.is_none());
    assert_eq!(r.until, Some(until));
}

#[test]
fn construct_with_every_by_part() {
    let r = RRule {
        by_day: Some(vec![wd(Weekday::Mon)]),
        by_month_day: Some(vec![1]),
        by_month: Some(vec![1]),
        by_week_no: Some(vec![1]),
        by_year_day: Some(vec![1]),
        by_set_pos: Some(vec![1]),
        ..base(Frequency::Yearly)
    };
    assert_eq!(r.to_ical_string(), "FREQ=YEARLY;BYDAY=MO;BYMONTHDAY=1;BYMONTH=1;BYWEEKNO=1;BYYEARDAY=1;BYSETPOS=1");
    assert_round_trip(&r);
}

#[test]
fn struct_update_leaves_original_untouched() {
    let original = base(Frequency::Daily);
    let modified = RRule { interval: 2, ..original.clone() };
    assert_eq!(original.interval, 1);
    assert_eq!(modified.interval, 2);
    assert_eq!(
        RRule { interval: 2, count: Some(10), ..base(Frequency::Daily) },
        RRule { interval: 2, count: Some(10), ..base(Frequency::Daily) }
    );
}

// ---- parsing ----

#[test]
fn parse_simple_and_interval_and_count() {
    assert_eq!(rule("FREQ=DAILY"), base(Frequency::Daily));
    assert_eq!(rule("FREQ=DAILY;INTERVAL=2").interval, 2);
    assert_eq!(rule("FREQ=WEEKLY;COUNT=10").count, Some(10));
}

#[test]
fn parse_until_keeps_utc_form() {
    let r = rule("FREQ=MONTHLY;UNTIL=20241231T235959Z");
    assert_eq!(r.until.expect("UNTIL").to_ical_string(), "20241231T235959Z");
}

#[test]
fn parse_date_only_until() {
    assert!(rule("FREQ=DAILY;UNTIL=20241231").until.expect("UNTIL").is_date);
}

#[test]
fn parse_by_day_lists_and_ordinals() {
    assert_eq!(
        rule("FREQ=WEEKLY;BYDAY=MO,WE,FR").by_day,
        Some(vec![wd(Weekday::Mon), wd(Weekday::Wed), wd(Weekday::Fri)])
    );
    assert_eq!(rule("FREQ=MONTHLY;BYDAY=2TU").by_day, Some(vec![nth(2, Weekday::Tue)]));
    assert_eq!(rule("FREQ=MONTHLY;BYDAY=-1FR").by_day, Some(vec![nth(-1, Weekday::Fri)]));
    assert_eq!(rule("FREQ=MONTHLY;BYDAY=5MO").by_day, Some(vec![nth(5, Weekday::Mon)]));
}

#[test]
fn parse_numeric_by_lists() {
    assert_eq!(rule("FREQ=MONTHLY;BYMONTHDAY=15").by_month_day, Some(vec![15]));
    assert_eq!(rule("FREQ=MONTHLY;BYMONTHDAY=1,15,-1").by_month_day, Some(vec![1, 15, -1]));
    let yearly = rule("FREQ=YEARLY;BYMONTH=12;BYMONTHDAY=25");
    assert_eq!((yearly.by_month, yearly.by_month_day), (Some(vec![12]), Some(vec![25])));
    assert_eq!(rule("FREQ=YEARLY;BYWEEKNO=1,52").by_week_no, Some(vec![1, 52]));
    assert_eq!(rule("FREQ=YEARLY;BYYEARDAY=1,100,200,-1").by_year_day, Some(vec![1, 100, 200, -1]));
    assert_eq!(rule("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1").by_set_pos, Some(vec![-1]));
}

#[test]
fn parse_wkst() {
    assert_eq!(rule("FREQ=WEEKLY;WKST=SU").wkst, Weekday::Sun);
}

#[test]
fn parse_thanksgiving_rule() {
    let r = rule("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH");
    assert_eq!(r.freq, Frequency::Yearly);
    assert_eq!(r.by_month, Some(vec![11]));
    assert_eq!(r.by_day, Some(vec![nth(4, Weekday::Thu)]));
}

#[test]
fn parse_rejects_missing_or_unknown_freq() {
    assert!(RRule::parse("INTERVAL=2;COUNT=10").is_err());
    assert!(RRule::parse("FREQ=INVALID").is_err());
}

// ---- serialization ----

#[test]
fn serialize_omits_defaults() {
    assert_eq!(base(Frequency::Daily).to_ical_string(), "FREQ=DAILY");
    assert_eq!(RRule { interval: 2, ..base(Frequency::Weekly) }.to_ical_string(), "FREQ=WEEKLY;INTERVAL=2");
    assert!(!RRule { wkst: Weekday::Mon, ..base(Frequency::Weekly) }.to_ical_string().contains("WKST"));
}

#[test]
fn serialize_includes_each_part() {
    let until = ICalDateTime::parse("20241231T235959Z", None).unwrap();
    let cases = [
        (RRule { count: Some(10), ..base(Frequency::Weekly) }, "COUNT=10"),
        (RRule { until: Some(until), ..base(Frequency::Monthly) }, "UNTIL=20241231T235959Z"),
        (RRule { by_day: Some(vec![wd(Weekday::Mon), wd(Weekday::Fri)]), ..base(Frequency::Weekly) }, "BYDAY=MO,FR"),
        (RRule { by_day: Some(vec![nth(2, Weekday::Tue)]), ..base(Frequency::Monthly) }, "BYDAY=2TU"),
        (RRule { by_day: Some(vec![nth(-1, Weekday::Fri)]), ..base(Frequency::Monthly) }, "BYDAY=-1FR"),
        (RRule { by_month_day: Some(vec![15, -1]), ..base(Frequency::Monthly) }, "BYMONTHDAY=15,-1"),
        (RRule { by_month: Some(vec![1, 7]), ..base(Frequency::Yearly) }, "BYMONTH=1,7"),
        (RRule { wkst: Weekday::Sun, ..base(Frequency::Weekly) }, "WKST=SU"),
        (RRule { by_hour: Some(vec![9, 12, 15]), ..base(Frequency::Daily) }, "BYHOUR=9,12,15"),
    ];
    for (r, part) in cases {
        let out = r.to_ical_string();
        assert!(out.contains(part), "expected {part} in {out}");
    }
}

#[test]
fn serialize_orders_time_parts_before_set_pos() {
    let r = RRule {
        by_hour: Some(vec![9, 12]),
        by_minute: Some(vec![0, 30]),
        by_second: Some(vec![0]),
        by_set_pos: Some(vec![1]),
        ..base(Frequency::Daily)
    };
    assert_eq!(r.to_ical_string(), "FREQ=DAILY;BYSECOND=0;BYMINUTE=0,30;BYHOUR=9,12;BYSETPOS=1");
}

#[test]
fn serialize_omits_unset_time_parts() {
    assert_eq!(RRule { count: Some(5), ..base(Frequency::Daily) }.to_ical_string(), "FREQ=DAILY;COUNT=5");
}

// ---- WeekdayNum ----

#[test]
fn weekday_num_parses_every_day_case_insensitively() {
    let days = [
        ("MO", Weekday::Mon),
        ("TU", Weekday::Tue),
        ("WE", Weekday::Wed),
        ("TH", Weekday::Thu),
        ("FR", Weekday::Fri),
        ("SA", Weekday::Sat),
        ("SU", Weekday::Sun),
        ("mo", Weekday::Mon),
        ("Mo", Weekday::Mon),
    ];
    for (code, day) in days {
        assert_eq!(WeekdayNum::parse(code).unwrap(), wd(day), "{code}");
    }
}

#[test]
fn weekday_num_parses_ordinals() {
    assert_eq!(WeekdayNum::parse("2TU").unwrap(), nth(2, Weekday::Tue));
    assert_eq!(WeekdayNum::parse("-1FR").unwrap(), nth(-1, Weekday::Fri));
    assert_eq!(WeekdayNum::parse("+3WE").unwrap(), nth(3, Weekday::Wed));
}

#[test]
fn weekday_num_rejects_unknown_day() {
    assert!(WeekdayNum::parse("XX").is_err());
    assert!(RRule::parse("FREQ=WEEKLY;BYDAY=MO,XX").is_err());
}

#[test]
fn weekday_num_formats() {
    assert_eq!(wd(Weekday::Mon).to_string(), "MO");
    assert_eq!(wd(Weekday::Sun).to_string(), "SU");
    assert_eq!(nth(2, Weekday::Tue).to_string(), "2TU");
    assert_eq!(nth(-1, Weekday::Fri).to_string(), "-1FR");
}

// ---- round trips ----

#[test]
fn parsed_rules_round_trip() {
    for value in [
        "FREQ=DAILY;INTERVAL=3;COUNT=30",
        "FREQ=WEEKLY;BYDAY=MO,WE,FR",
        "FREQ=MONTHLY;BYDAY=2TU",
        "FREQ=YEARLY;COUNT=10;BYDAY=4TH;BYMONTH=11",
        "FREQ=DAILY;BYMINUTE=0,20,40;BYHOUR=9,10,11,12,13,14,15,16",
    ] {
        let parsed = rule(value);
        assert_round_trip(&parsed);
        assert_eq!(parsed.to_ical_string(), value);
    }
}

#[test]
fn constructed_time_parts_round_trip() {
    assert_round_trip(&RRule {
        by_hour: Some(vec![9, 12, 15]),
        by_minute: Some(vec![0, 30]),
        by_second: Some(vec![0, 15, 30]),
        ..base(Frequency::Daily)
    });
}

#[test]
fn rfc_5545_example_36_parses() {
    let r = rule("FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40");
    assert_eq!(r.by_hour, Some((9..=16).collect()));
    assert_eq!(r.by_minute, Some(vec![0, 20, 40]));
}

// ---- tolerance ----

#[test]
fn parse_time_lists() {
    assert_eq!(rule("FREQ=DAILY;BYMINUTE=0,20,40").by_minute, Some(vec![0, 20, 40]));
    assert_eq!(rule("FREQ=MINUTELY;BYSECOND=0,30").by_second, Some(vec![0, 30]));
}

#[test]
fn parse_skips_non_numeric_list_entries() {
    assert_eq!(rule("FREQ=DAILY;BYHOUR=9,foo,12").by_hour, Some(vec![9, 12]));
}

#[test]
fn parse_keeps_out_of_range_values() {
    let r = rule("FREQ=DAILY;BYHOUR=24;BYSECOND=-1");
    assert_eq!(r.by_hour, Some(vec![24]));
    assert_eq!(r.by_second, Some(vec![-1]));
}

#[test]
fn all_seven_frequencies_round_trip_by_name() {
    let all = [
        Frequency::Secondly,
        Frequency::Minutely,
        Frequency::Hourly,
        Frequency::Daily,
        Frequency::Weekly,
        Frequency::Monthly,
        Frequency::Yearly,
    ];
    for freq in all {
        assert_eq!(freq.as_str().parse::<Frequency>().unwrap(), freq);
    }
}
