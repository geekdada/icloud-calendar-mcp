//! Port of icaldav-core `ICalParserTimezoneTest`.
//!
//! The Kotlin suite switches the JVM default zone per test. DATE values are
//! stored as UTC midnight and never resolved through the host zone, so the
//! cases collapse into data-driven checks that hold on any host.

mod ical_support;

use chrono_tz::Tz;
use ical_support::*;

fn all_day(start: &str, end: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\nUID:a@test.com\nDTSTAMP:20251221T153656Z\nDTSTART;VALUE=DATE:{start}\nDTEND;VALUE=DATE:{end}\nSUMMARY:x\nEND:VEVENT"
    ))
}

#[test]
fn all_day_dates_keep_their_calendar_day() {
    for (start, end) in [
        ("20251225", "20251226"),
        ("20241224", "20241227"),
        ("20260106", "20260107"),
        ("20260315", "20260316"),
        ("20260720", "20260721"),
        ("20260105", "20260108"),
        ("20251231", "20260102"),
        ("20251011", "20251013"),
    ] {
        let event = parse_one(&all_day(start, end));
        assert!(event.is_all_day);
        assert_eq!(event.dt_start.to_day_code(), start);
        assert_eq!(event.dt_end.unwrap().to_day_code(), end, "DTEND stays exclusive");
        assert_eq!(event.dt_start.timestamp % 86_400_000, 0, "stored as UTC midnight");
    }
}

#[test]
fn exdate_value_date_is_not_shifted() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:exdate-sydney@test.com
        DTSTART;VALUE=DATE:20260101
        DTEND;VALUE=DATE:20260102
        RRULE:FREQ=DAILY;COUNT=10
        EXDATE;VALUE=DATE:20260106
        SUMMARY:Daily with Exception
        END:VEVENT",
    ));
    assert_eq!(event.exdates.len(), 1);
    assert_eq!(event.exdates[0].to_day_code(), "20260106");
    assert!(event.exdates[0].is_date);
}

#[test]
fn recurrence_id_value_date_is_not_shifted() {
    let events = parse(&vcal(
        "
        BEGIN:VEVENT
        UID:recid-allday@test.com
        DTSTART;VALUE=DATE:20260101
        DTEND;VALUE=DATE:20260102
        RRULE:FREQ=WEEKLY;COUNT=10
        SUMMARY:Weekly All-Day
        END:VEVENT
        BEGIN:VEVENT
        UID:recid-allday@test.com
        RECURRENCE-ID;VALUE=DATE:20260108
        DTSTART;VALUE=DATE:20260109
        DTEND;VALUE=DATE:20260110
        SUMMARY:Moved to Friday
        END:VEVENT",
    ));
    assert_eq!(events.len(), 2);
    let exception = events.iter().find(|e| e.recurrence_id.is_some()).unwrap();
    assert_eq!(exception.recurrence_id.unwrap().to_day_code(), "20260108");
}

#[test]
fn timed_event_with_tzid() {
    let event = parse_one(&vcal(
        "
        BEGIN:VEVENT
        UID:tzid-ny@test.com
        DTSTART;TZID=America/New_York:20260115T140000
        DTEND;TZID=America/New_York:20260115T150000
        SUMMARY:New York Meeting
        END:VEVENT",
    ));
    assert_eq!(event.dt_start.timezone, Some(Tz::America__New_York));
    assert!(!event.is_all_day);
    assert_eq!(event.dt_start.timestamp, millis("2026-01-15T19:00:00Z"));
}

#[test]
fn utc_and_floating_datetimes() {
    let utc_event = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:u\nDTSTART:20260115T190000Z\nDTEND:20260115T200000Z\nSUMMARY:x\nEND:VEVENT",
    ));
    assert!(!utc_event.is_all_day);
    assert!(utc_event.dt_start.is_utc);
    assert_eq!(utc_event.dt_start.timezone, None);

    let floating =
        parse_one(&vcal("BEGIN:VEVENT\nUID:f\nDTSTART:20260115T140000\nDTEND:20260115T150000\nSUMMARY:x\nEND:VEVENT"));
    assert!(!floating.is_all_day);
    assert!(!floating.dt_start.is_utc);
    assert_eq!(floating.dt_start.timezone, Some(icloud_calendar_mcp::ical::timezone::system_zone()));
    assert_eq!(floating.dt_start.to_ical_string(), "20260115T140000");
}

fn outlook(tzid: &str, vtimezone: &str, local: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Microsoft Corporation//Outlook 16.0//EN\n{vtimezone}BEGIN:VEVENT\nDTSTART;TZID=\"{tzid}\":{local}\nDTSTAMP:20260115T100000Z\nUID:outlook@test.com\nSUMMARY:Meeting\nEND:VEVENT\nEND:VCALENDAR"
    )
}

#[test]
fn windows_timezone_names_resolve() {
    let eastern_vtz = "BEGIN:VTIMEZONE\nTZID:Eastern Standard Time\nBEGIN:STANDARD\nDTSTART:16011104T020000\nRRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11\nTZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nEND:STANDARD\nEND:VTIMEZONE\n";
    for (tzid, vtz, local, expected) in [
        ("Eastern Standard Time", eastern_vtz, "20260115T140000", "2026-01-15T19:00:00Z"),
        ("Eastern Standard Time", "", "20260115T140000", "2026-01-15T19:00:00Z"),
        ("India Standard Time", "", "20260115T143000", "2026-01-15T09:00:00Z"),
        ("W. Europe Standard Time", "", "20260115T140000", "2026-01-15T13:00:00Z"),
        ("Tokyo Standard Time", "", "20260115T140000", "2026-01-15T05:00:00Z"),
        ("AUS Eastern Standard Time", "", "20260115T140000", "2026-01-15T03:00:00Z"),
    ] {
        let event = parse_one(&outlook(tzid, vtz, local));
        assert_eq!(event.dt_start.timestamp, millis(expected), "{tzid}");
    }
}
