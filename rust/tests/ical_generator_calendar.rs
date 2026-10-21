//! Port of the VEVENT parts of icaldav-core `ICalGeneratorCalendarTest`.
//!
//! Skipped: calendar-level metadata (NAME, SOURCE, COLOR, REFRESH-INTERVAL,
//! X-WR-CALNAME, X-APPLE-CALENDAR-COLOR, IMAGE, METHOD), PRODID precedence and
//! its escaping/folding, VTODO / VJOURNAL mixing and TZID collection, the
//! `includeMethod` / `includeVTimezone` switches, and batch-vs-calendar
//! equivalence (there is a single `generate_calendar` entry point). The pinned
//! snapshot keeps its header order minus the METHOD line.

mod ical_support;

use chrono::TimeZone;
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::generator::{PRODID, generate_calendar};
use icloud_calendar_mcp::ical::{EventStatus, ICalDateTime, ICalEvent};

fn new_york_event(uid: &str) -> ICalEvent {
    let zone = Tz::America__New_York;
    let at = |hour| ICalDateTime::from_zoned(zone.with_ymd_and_hms(2024, 3, 6, hour, 0, 0).unwrap(), false);
    let mut e = ICalEvent::new(uid, at(12));
    e.dt_end = Some(at(13));
    e.summary = Some("Event in NY".into());
    e.status = Some(EventStatus::Confirmed);
    e
}

fn physical_lines(ics: &str) -> Vec<&str> {
    ics.split("\r\n").filter(|l| !l.is_empty()).collect()
}

#[test]
fn empty_calendar_has_envelope_only() {
    let ics = generate_calendar(&[]);
    assert_eq!(
        physical_lines(&ics),
        ["BEGIN:VCALENDAR", "VERSION:2.0", &format!("PRODID:{PRODID}"), "CALSCALE:GREGORIAN", "END:VCALENDAR"]
    );
}

#[test]
fn vtimezone_precedes_the_first_event() {
    let ics = generate_calendar(&[new_york_event("ev-1")]);
    let vtimezone = ics.find("BEGIN:VTIMEZONE").unwrap();
    assert!(vtimezone > 0 && vtimezone < ics.find("BEGIN:VEVENT").unwrap());
}

#[test]
fn shared_tzid_is_emitted_once() {
    let ics = generate_calendar(&[new_york_event("ev-a"), new_york_event("ev-b")]);
    assert_eq!(ics.matches("TZID:America/New_York\r\n").count(), 1);
    assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
}

#[test]
fn pinned_snapshot() {
    let ics = generate_calendar(&[new_york_event("snap-pin")]);
    let lines = physical_lines(&ics);
    let prodid = format!("PRODID:{PRODID}");
    assert_eq!(
        lines[..5],
        ["BEGIN:VCALENDAR", "VERSION:2.0", prodid.as_str(), "CALSCALE:GREGORIAN", "BEGIN:VTIMEZONE"]
    );

    let end_vtimezone = lines.iter().position(|l| *l == "END:VTIMEZONE").unwrap();
    let begin_vevent = lines.iter().position(|l| *l == "BEGIN:VEVENT").unwrap();
    assert!(end_vtimezone < begin_vevent);

    for expected in [
        "UID:snap-pin",
        "DTSTART;TZID=America/New_York:20240306T120000",
        "DTEND;TZID=America/New_York:20240306T130000",
        "SUMMARY:Event in NY",
        "STATUS:CONFIRMED",
        "SEQUENCE:0",
        "END:VEVENT",
    ] {
        assert!(lines.contains(&expected), "{expected}");
    }
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("DTSTAMP:") && l.ends_with('Z') && l.len() == "DTSTAMP:20240101T000000Z".len())
    );
    assert_eq!(lines.last(), Some(&"END:VCALENDAR"));
}
