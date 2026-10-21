//! Port of icaldav-core `ICalGeneratorCalAddressTest`.
//!
//! ORGANIZER and ATTENDEE are written back from their raw lines, never rebuilt
//! from a bare email, so the Kotlin "re-prepend mailto: only for mailboxes"
//! guard becomes "every CAL-ADDRESS form is emitted exactly as given".

mod ical_support;

use chrono::TimeZone;
use chrono_tz::Tz;
use ical_support::*;
use icloud_calendar_mcp::ical::content::ContentLine;
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent};

fn event_with(organizer: &str, attendee: &str) -> ICalEvent {
    let zone = Tz::America__New_York;
    let start = zone.with_ymd_and_hms(2026, 1, 15, 14, 0, 0).unwrap();
    let mut e = ICalEvent::new("cal-address@example.test", ICalDateTime::from_zoned(start, false));
    e.dt_end = Some(ICalDateTime::from_zoned(zone.with_ymd_and_hms(2026, 1, 15, 15, 0, 0).unwrap(), false));
    e.summary = Some("Sync".into());
    e.raw_properties = vec![ContentLine::parse(organizer).unwrap(), ContentLine::parse(attendee).unwrap()];
    e
}

fn line_starting(ics: &str, name: &str) -> String {
    lines(ics).into_iter().find(|l| l.starts_with(name)).unwrap()
}

const BOSS: &str = "ORGANIZER;CN=Boss:mailto:boss@example.test";

#[test]
fn urn_uuid_attendee_is_verbatim() {
    let ics = generate(&event_with(BOSS, "ATTENDEE;CN=Alice;RSVP=TRUE:urn:uuid:0c3f2d4e-9b1a-4f6e-8a2b-1c2d3e4f5061"));
    let line = line_starting(&ics, "ATTENDEE");
    assert!(line.ends_with(":urn:uuid:0c3f2d4e-9b1a-4f6e-8a2b-1c2d3e4f5061"), "{line}");
    assert!(!line.contains("mailto:urn:uuid:"));
}

#[test]
fn urn_uuid_organizer_is_verbatim() {
    let ics = generate(&event_with(
        "ORGANIZER;CN=Boss:urn:uuid:11112222-3333-4444-5555-666677778888",
        "ATTENDEE;CN=Alice:mailto:alice@example.test",
    ));
    let line = line_starting(&ics, "ORGANIZER");
    assert!(line.ends_with(":urn:uuid:11112222-3333-4444-5555-666677778888"), "{line}");
    assert!(!line.contains("mailto:urn:uuid:"));
}

#[test]
fn principal_href_attendee_is_verbatim() {
    let line = line_starting(&generate(&event_with(BOSS, "ATTENDEE;CN=Alice:/646691839/principal/")), "ATTENDEE");
    assert!(line.ends_with(":/646691839/principal/") && !line.contains("mailto:/646691839"), "{line}");
}

#[test]
fn mailbox_attendee_keeps_exactly_one_mailto() {
    let line = line_starting(&generate(&event_with(BOSS, "ATTENDEE;CN=Alice:mailto:alice@example.test")), "ATTENDEE");
    assert!(line.ends_with(":mailto:alice@example.test"), "{line}");
    assert!(!line.contains("mailto:mailto:"));
}

#[test]
fn urn_uuid_attendee_round_trips_through_parse_and_generate() {
    let server = vcal(
        "BEGIN:VEVENT\nUID:roundtrip@example.test\nDTSTAMP:20260101T100000Z\nDTSTART:20260301T100000Z\nDTEND:20260301T110000Z\nSUMMARY:Round trip\nORGANIZER:mailto:boss@example.test\nATTENDEE;CN=Alice;PARTSTAT=ACCEPTED:urn:uuid:abcd1234-0000-0000-0000-00000000aaaa\nEND:VEVENT",
    )
    .replace('\n', "\r\n");
    let parsed = parse_one(&server);
    assert_eq!(parsed.attendees[0].email, "urn:uuid:abcd1234-0000-0000-0000-00000000aaaa");
    let line = line_starting(&generate(&parsed), "ATTENDEE");
    assert_eq!(line, "ATTENDEE;CN=Alice;PARTSTAT=ACCEPTED:urn:uuid:abcd1234-0000-0000-0000-00000000aaaa");
}
