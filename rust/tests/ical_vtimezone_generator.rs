//! Port of icaldav-core `VTimezoneGeneratorTest`.
//!
//! There is no batch `generate(Set<String>)`; the batch cases go through
//! `generate_calendar`, which emits one VTIMEZONE per collected TZID. The
//! Kotlin all-day case accepts either behavior; here an all-day DATE has no
//! zone, so it contributes no TZID. Kotlin only checks for "SU" somewhere in
//! the European rules; the exact last-Sunday rules are asserted instead.

mod ical_support;

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use icloud_calendar_mcp::ical::generator::{collect_timezones, generate_calendar};
use icloud_calendar_mcp::ical::vtimezone::{format_offset, generate_vtimezone};
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent};

fn zoned_event(tzid: &str) -> ICalEvent {
    let zone: Tz = tzid.parse().unwrap();
    let start = zone.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
    let mut e = ICalEvent::new(format!("test-{}", tzid.replace('/', "-")), ICalDateTime::from_zoned(start, false));
    e.dt_end = Some(ICalDateTime::from_zoned(zone.with_ymd_and_hms(2024, 6, 15, 11, 0, 0).unwrap(), false));
    e
}

fn assert_dst_zone(tzid: &str, standard: &str, daylight: &str) {
    let block = generate_vtimezone(tzid);
    assert!(block.contains(&format!("TZID:{tzid}\r\n")), "{block}");
    assert!(block.contains("BEGIN:STANDARD") && block.contains("END:STANDARD"), "{block}");
    assert!(block.contains("BEGIN:DAYLIGHT") && block.contains("END:DAYLIGHT"), "{block}");
    assert!(block.contains(&format!("TZOFFSETTO:{standard}\r\n")), "{tzid} standard: {block}");
    assert!(block.contains(&format!("TZOFFSETTO:{daylight}\r\n")), "{tzid} daylight: {block}");
    assert!(block.contains("RRULE:FREQ=YEARLY"), "{block}");
}

#[test]
fn fixed_offset_zones_have_standard_only() {
    for (tzid, offset) in
        [("Asia/Kolkata", "+0530"), ("Asia/Kathmandu", "+0545"), ("Asia/Tokyo", "+0900"), ("Asia/Shanghai", "+0800")]
    {
        let block = generate_vtimezone(tzid);
        assert!(block.starts_with("BEGIN:VTIMEZONE\r\n") && block.ends_with("END:VTIMEZONE\r\n"), "{block}");
        assert!(block.contains(&format!("TZID:{tzid}\r\n")));
        assert!(block.contains(&format!("TZOFFSETFROM:{offset}\r\nTZOFFSETTO:{offset}\r\n")), "{block}");
        assert_eq!(block.matches("BEGIN:STANDARD").count(), 1);
        assert!(!block.contains("BEGIN:DAYLIGHT"), "{tzid} has no DST");
    }
}

#[test]
fn us_zones() {
    assert_dst_zone("America/New_York", "-0500", "-0400");
    assert_dst_zone("America/Los_Angeles", "-0800", "-0700");
    assert_dst_zone("America/Chicago", "-0600", "-0500");
}

#[test]
fn european_zones() {
    assert_dst_zone("Europe/London", "+0000", "+0100");
    assert_dst_zone("Europe/Paris", "+0100", "+0200");
    assert_dst_zone("Europe/Berlin", "+0100", "+0200");
}

#[test]
fn southern_hemisphere_zones() {
    assert_dst_zone("Australia/Sydney", "+1000", "+1100");
    assert_dst_zone("Pacific/Auckland", "+1200", "+1300");
}

#[test]
fn utc_unknown_and_empty_ids_produce_nothing() {
    for tzid in ["Invalid/Timezone", "UTC", "Z", "Etc/UTC", "GMT", ""] {
        assert_eq!(generate_vtimezone(tzid), "", "{tzid:?}");
    }
}

#[test]
fn batch_emits_one_block_per_zone() {
    let events: Vec<_> = ["America/New_York", "Europe/London", "Asia/Tokyo"].map(zoned_event).into();
    let ics = generate_calendar(&events);
    for tzid in ["America/New_York", "Europe/London", "Asia/Tokyo"] {
        assert!(ics.contains(&format!("TZID:{tzid}\r\n")));
    }
    assert_eq!(ics.matches("BEGIN:VTIMEZONE").count(), 3);
    assert!(!generate_calendar(&[]).contains("BEGIN:VTIMEZONE"));
}

#[test]
fn us_rules_are_second_sunday_march_and_first_sunday_november() {
    let block = generate_vtimezone("America/New_York");
    assert!(block.contains("RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\n"), "{block}");
    assert!(block.contains("RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\n"), "{block}");
}

#[test]
fn european_rules_are_last_sunday_march_and_october() {
    let block = generate_vtimezone("Europe/London");
    assert!(block.contains("RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\n"), "{block}");
    assert!(block.contains("RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\n"), "{block}");
}

#[test]
fn offset_formatting() {
    for (seconds, expected) in [
        (0, "+0000"),
        (5 * 3600 + 1800, "+0530"),
        (-8 * 3600, "-0800"),
        (12 * 3600 + 2700, "+1245"),
        (-12 * 3600, "-1200"),
    ] {
        assert_eq!(format_offset(seconds), expected);
    }
}

#[test]
fn collect_timezones_dedupes_and_skips_utc_and_dates() {
    let tzids = collect_timezones(&[
        zoned_event("America/New_York"),
        zoned_event("Europe/London"),
        zoned_event("America/New_York"),
    ]);
    assert_eq!(tzids, ["America/New_York", "Europe/London"]);

    let utc = ICalEvent::new("test-utc", ICalDateTime::parse("20240615T100000Z", None).unwrap());
    assert!(collect_timezones(&[utc]).is_empty());

    let mut all_day =
        ICalEvent::new("test-allday", ICalDateTime::from_local_date(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()));
    all_day.is_all_day = true;
    assert!(collect_timezones(&[all_day]).is_empty());

    assert!(collect_timezones(&[]).is_empty());
}

#[test]
fn structure_and_1970_base() {
    let block = generate_vtimezone("America/New_York");
    for needle in
        ["BEGIN:VTIMEZONE", "END:VTIMEZONE", "TZID:", "DTSTART:1970", "TZOFFSETFROM:", "TZOFFSETTO:", "TZNAME:"]
    {
        assert!(block.contains(needle), "{needle}");
    }
    assert!(block.contains("TZNAME:EDT\r\n") && block.contains("TZNAME:EST\r\n"), "{block}");
}
