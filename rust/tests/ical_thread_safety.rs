//! Port of icaldav-core `ThreadSafetyTest` as concurrency smoke tests.
//!
//! The parser, generator and expander are free functions over immutable input,
//! so "shared instance" and "separate instances" collapse into the same case.
//! The Kotlin double-checked-locking test becomes a barrier-released race on
//! the first use of the lazily built regexes and tables.

mod ical_support;

use std::sync::Barrier;
use std::thread;

use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use ical_support::*;
use icloud_calendar_mcp::ical::expander::expand;
use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ical::{Frequency, ICalDateTime, ICalEvent, RRule};

fn unique_event(thread: usize, iteration: usize) -> String {
    let hour = iteration % 24;
    // Hour 23 yields DTEND T240000Z, as in the Kotlin fixture; it is dropped as
    // unreadable and the event still parses.
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//Thread Safety//EN\r\nBEGIN:VEVENT\r\nUID:thread-{thread}-event-{iteration}\r\nDTSTAMP:20231215T100000Z\r\nDTSTART:20231215T{hour:02}0000Z\r\nDTEND:20231215T{:02}0000Z\r\nSUMMARY:Thread {thread} Event {iteration}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        hour + 1
    )
}

fn complex_event(thread: usize, iteration: usize) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//Thread Safety//EN\r\nBEGIN:VEVENT\r\nUID:complex-{thread}-{iteration}\r\nDTSTAMP:20231215T100000Z\r\nDTSTART;TZID=America/New_York:20231215T140000\r\nDTEND;TZID=America/New_York:20231215T150000\r\nSUMMARY:Complex Event {thread}-{iteration}\r\nDESCRIPTION:Description with special chars\\, and newlines\\n\r\nLOCATION:Room {}\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=10\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        iteration % 10
    )
}

fn daily_ten(uid: &str, zone: Tz, year: i32, month: u32, day: u32) -> ICalEvent {
    let at = |hour| ICalDateTime::from_zoned(zone.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap(), false);
    let mut e = ICalEvent::new(uid, at(10));
    e.dt_end = Some(at(11));
    let mut rule = RRule::new(Frequency::Daily);
    rule.count = Some(10);
    e.rrule = Some(rule);
    e
}

fn instant(zone: Tz, year: i32, month: u32, day: u32) -> DateTime<Utc> {
    zone.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap().with_timezone(&Utc)
}

/// Run `work(thread_index)` on `threads` threads and propagate any panic.
fn run_concurrently(threads: usize, work: impl Fn(usize) + Sync) {
    let work = &work;
    thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|t| scope.spawn(move || work(t))).collect();
        for handle in handles {
            handle.join().expect("worker thread panicked");
        }
    });
}

#[test]
fn concurrent_parsing_of_different_events() {
    run_concurrently(10, |t| {
        for i in 0..100 {
            let events = parse_all_events(&unique_event(t, i)).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].uid, format!("thread-{t}-event-{i}"));
        }
    });
}

#[test]
fn concurrent_parsing_of_the_same_text() {
    let shared = vcal(
        "BEGIN:VEVENT\nUID:shared-event-uid\nDTSTAMP:20231215T100000Z\nDTSTART:20231215T140000Z\nDTEND:20231215T150000Z\nSUMMARY:Shared Event for Thread Safety Test\nDESCRIPTION:This event is parsed concurrently by many threads\nRRULE:FREQ=DAILY;COUNT=10\nEND:VEVENT",
    );
    run_concurrently(20, |_| {
        for _ in 0..50 {
            let events = parse_all_events(&shared).unwrap();
            assert_eq!(events[0].uid, "shared-event-uid");
            assert_eq!(events[0].rrule.as_ref().unwrap().freq, Frequency::Daily);
        }
    });
}

#[test]
fn simultaneous_first_use_of_lazy_statics() {
    let simple = vcal(
        "BEGIN:VEVENT\nUID:simple-test\nDTSTAMP:20231215T100000Z\nDTSTART;TZID=W. Europe Standard Time:20231215T140000\nDTEND:20231215T150000Z\nDURATION:P1W2D\nBEGIN:VALARM\nTRIGGER:-PT2D\nEND:VALARM\nEND:VEVENT",
    );
    let barrier = Barrier::new(50);
    run_concurrently(50, |_| {
        barrier.wait();
        let events = parse_all_events(&simple).unwrap();
        assert_eq!(events[0].dt_start.timestamp, millis("2023-12-15T14:00:00+01:00"));
        assert!(generate(&events[0]).contains("BEGIN:VTIMEZONE"));
    });
}

#[test]
fn concurrent_expansion_of_different_events() {
    let zone = Tz::America__New_York;
    run_concurrently(10, |t| {
        for i in 0..50 {
            let event = daily_ten(&format!("thread-{t}-recurring-{i}"), zone, 2024, 1, (i % 28) as u32 + 1);
            let occurrences = expand(&event, instant(zone, 2024, 1, 1), instant(zone, 2024, 12, 31), &[]).unwrap();
            assert_eq!(occurrences.len(), 10, "COUNT=10 fits in the range");
        }
    });
}

#[test]
fn concurrent_expansion_of_a_shared_event() {
    let zone = Tz::UTC;
    let shared = daily_ten("shared-recurring", zone, 2024, 6, 15);
    run_concurrently(20, |_| {
        for _ in 0..100 {
            let occurrences = expand(&shared, instant(zone, 2024, 1, 1), instant(zone, 2024, 12, 31), &[]).unwrap();
            assert_eq!(occurrences.len(), 10);
            assert_eq!(occurrences[0].dt_start.to_day_code(), "20240615");
        }
    });
}

#[test]
fn concurrent_expansion_across_dst_keeps_wall_time() {
    let zone = Tz::America__New_York;
    let dst = daily_ten("dst-event", zone, 2024, 3, 1);
    run_concurrently(10, |_| {
        for _ in 0..50 {
            let occurrences = expand(&dst, instant(zone, 2024, 3, 1), instant(zone, 2024, 3, 31), &[]).unwrap();
            assert_eq!(occurrences.len(), 10);
            assert!(
                occurrences.iter().all(|o| o.dt_start.to_zoned().hour() == 10),
                "10:00 local across the March 10 change"
            );
        }
    });
}

#[test]
fn concurrent_generation() {
    run_concurrently(10, |t| {
        for i in 0..50 {
            let event = parse_all_events(&unique_event(t, i)).unwrap().remove(0);
            let ics = generate(&event);
            assert!(ics.starts_with("BEGIN:VCALENDAR\r\n") && ics.ends_with("END:VCALENDAR\r\n"));
            assert!(ics.contains(&format!("UID:thread-{t}-event-{i}\r\n")));
        }
    });
}

#[test]
fn concurrent_round_trips() {
    run_concurrently(10, |t| {
        for i in 0..30 {
            let first = parse_all_events(&complex_event(t, i)).unwrap().remove(0);
            let second = round_trip(&first);
            assert_eq!(second.uid, first.uid);
            assert_eq!(second.summary, first.summary);
            assert_eq!(second.description, first.description);
            assert_eq!(second.is_all_day, first.is_all_day);
            assert_eq!(second.rrule, first.rrule);
        }
    });
}
