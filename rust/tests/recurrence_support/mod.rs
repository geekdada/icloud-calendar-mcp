//! Fixtures shared by the recurrence test suites (ported from icaldav-core).

#![allow(dead_code)]

use chrono::{DateTime, Datelike, NaiveDate, TimeDelta, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::expander::expand;
use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent, RRule, WeekdayNum};

pub const NY: Tz = Tz::America__New_York;
/// Stand-in for the JVM default zone the Kotlin suites used; any DST zone works.
pub const LOCAL: Tz = Tz::Europe__Paris;

pub fn zdt(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Tz> {
    tz.with_ymd_and_hms(y, m, d, h, min, 0).earliest().expect("valid local time")
}

/// A zoned local DATE-TIME (`TZID=...`).
pub fn zoned(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> ICalDateTime {
    ICalDateTime::from_zoned(zdt(tz, y, m, d, h, min), false)
}

/// A UTC-form DATE-TIME (`...Z`).
pub fn utc_dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> ICalDateTime {
    ICalDateTime::utc(zdt(Tz::UTC, y, m, d, h, min).timestamp_millis())
}

/// A DATE value.
pub fn date(y: i32, m: u32, d: u32) -> ICalDateTime {
    ICalDateTime::from_local_date(NaiveDate::from_ymd_opt(y, m, d).unwrap())
}

pub fn instant(dt: DateTime<Tz>) -> DateTime<Utc> {
    dt.with_timezone(&Utc)
}

pub fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// `[first of month, first of next month]` in `tz`, like icaldav's `TimeRange.forMonth`.
pub fn month_range(tz: Tz, y: i32, m: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (instant(zdt(tz, y, m, 1, 0, 0)), instant(zdt(tz, ny, nm, 1, 0, 0)))
}

pub fn rule(s: &str) -> RRule {
    RRule::parse(s).unwrap()
}

pub fn wd(weekday: Weekday) -> WeekdayNum {
    WeekdayNum { weekday, ordinal: None }
}

pub fn nth(n: i32, weekday: Weekday) -> WeekdayNum {
    WeekdayNum { weekday, ordinal: Some(n) }
}

/// An event starting at `start` lasting one hour (one day when `start` is a DATE).
pub fn event(start: ICalDateTime, rrule: Option<RRule>) -> ICalEvent {
    let length = if start.is_date { 86_400_000 } else { 3_600_000 };
    event_until(start, ICalDateTime { timestamp: start.timestamp + length, ..start }, rrule)
}

pub fn event_until(start: ICalDateTime, end: ICalDateTime, rrule: Option<RRule>) -> ICalEvent {
    let mut e = ICalEvent::new("test-event", start);
    e.summary = Some("Test Event".into());
    e.dt_end = Some(end);
    e.rrule = rrule;
    e
}

/// An event with a DURATION instead of a DTEND.
pub fn event_for(start: ICalDateTime, duration: TimeDelta, rrule: Option<RRule>) -> ICalEvent {
    let mut e = ICalEvent::new("test-event", start);
    e.summary = Some("Test Event".into());
    e.duration = Some(duration);
    e.rrule = rrule;
    e
}

pub fn expand_all(event: &ICalEvent, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<ICalEvent> {
    expand(event, start, end, &[]).expect("expansion within bounds")
}

pub fn day_codes(occurrences: &[ICalEvent]) -> Vec<String> {
    occurrences.iter().map(|o| o.dt_start.to_day_code()).collect()
}

/// Local `HH:MM` of each occurrence start in `tz`.
pub fn times_in(occurrences: &[ICalEvent], tz: Tz) -> Vec<String> {
    occurrences
        .iter()
        .map(|o| {
            let t = o.dt_start.to_utc().with_timezone(&tz);
            format!("{}:{:02}", t.hour(), t.minute())
        })
        .collect()
}

/// Local dates (`YYYY-MM-DD`) of each occurrence start in its own zone.
pub fn dates(occurrences: &[ICalEvent]) -> Vec<String> {
    occurrences.iter().map(|o| o.dt_start.to_local_date().format("%Y-%m-%d").to_string()).collect()
}

pub fn local_day(o: &ICalEvent) -> u32 {
    o.dt_start.to_local_date().day()
}

pub fn vcal(body: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//Test//EN\r\n{body}END:VCALENDAR\r\n")
}

/// Parse an ICS document written with `\n` line endings.
pub fn parse(ics: &str) -> Vec<ICalEvent> {
    parse_all_events(ics).expect("parseable calendar")
}

pub fn parse_one(ics: &str) -> ICalEvent {
    let mut events = parse(ics);
    assert!(!events.is_empty(), "no VEVENT parsed");
    events.remove(0)
}
