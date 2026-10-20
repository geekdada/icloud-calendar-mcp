//! Port of icaldav-core `ICalParserAttendeeTest`.
//!
//! ATTENDEE is a verbatim raw line with an email/CN view, so MEMBER and RSVP
//! are not modelled. The Kotlin field assertions become "the parameter is
//! readable from the raw line and survives a round trip unchanged". The
//! generator cases that build an `Attendee` with `member`/`rsvp` fields are
//! skipped: there is no structured attendee to generate from.
//!
//! The Kotlin multi-value MEMBER case is disabled because ical4j corrupts the
//! property; the raw-line model keeps it intact, so it runs here.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::ICalEvent;
use icloud_calendar_mcp::ical::content::ContentLine;

fn event_with(attendee_line: &str) -> ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:attendee-test-1\nDTSTAMP:20231215T100000Z\nDTSTART:20231215T140000Z\nDTEND:20231215T150000Z\nSUMMARY:Attendee Test\n{attendee_line}\nEND:VEVENT"
    )))
}

fn attendee_line(event: &ICalEvent) -> &ContentLine {
    raw(event, "ATTENDEE").expect("ATTENDEE kept raw")
}

fn assert_round_trips_verbatim(line: &str) {
    let e = event_with(line);
    assert!(lines(&generate(&e)).iter().any(|l| l == line), "{line} did not survive");
}

#[test]
fn single_member_value() {
    let line = r#"ATTENDEE;CN=Alice;MEMBER="mailto:dlist@example.com":mailto:alice@example.com"#;
    let e = event_with(line);
    assert_eq!(attendee_line(&e).param("MEMBER"), Some("mailto:dlist@example.com"));
    assert_eq!(e.attendees[0].email, "alice@example.com");
    assert_round_trips_verbatim(line);
}

#[test]
fn comma_separated_member_values_keep_the_address_intact() {
    let line =
        r#"ATTENDEE;CN=Alice;MEMBER="mailto:dlist1@example.com","mailto:dlist2@example.com":mailto:alice@example.com"#;
    let e = event_with(line);
    assert_eq!(e.attendees.len(), 1);
    assert_eq!(e.attendees[0].email, "alice@example.com");
    assert_eq!(e.attendees[0].name.as_deref(), Some("Alice"));
    let member = attendee_line(&e).param("MEMBER").unwrap();
    assert!(member.contains("dlist1@example.com") && member.contains("dlist2@example.com"), "{member}");
    assert_round_trips_verbatim(line);
}

#[test]
fn missing_member() {
    assert_eq!(attendee_line(&event_with("ATTENDEE;CN=Alice:mailto:alice@example.com")).param("MEMBER"), None);
}

#[test]
fn rsvp_is_three_state() {
    for (line, expected) in [
        ("ATTENDEE;CN=Alice;RSVP=TRUE:mailto:alice@example.com", Some("TRUE")),
        ("ATTENDEE;CN=Alice;RSVP=FALSE:mailto:alice@example.com", Some("FALSE")),
        ("ATTENDEE;CN=Alice:mailto:alice@example.com", None),
    ] {
        assert_eq!(attendee_line(&event_with(line)).param("RSVP"), expected, "{line}");
        assert_round_trips_verbatim(line);
    }
}
