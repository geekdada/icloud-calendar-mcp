//! Port of the VEVENT parts of icaldav-core `ICalParserEmailFallbackTest`.
//!
//! Skipped: the VTODO, VJOURNAL and VFREEBUSY siblings (components not ported).

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::ICalEvent;

fn vevent(props: &[&str]) -> ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:test-event\nDTSTAMP:20260315T100000Z\nDTSTART:20260315T100000Z\nDTEND:20260315T110000Z\nSUMMARY:Test\n{}\nEND:VEVENT",
        props.join("\n")
    )))
}

fn organizer_email(prop: &str) -> String {
    vevent(&[prop]).organizer.expect("organizer").email
}

#[test]
fn mailto_organizer_is_used_directly() {
    assert_eq!(organizer_email("ORGANIZER;CN=Alice:mailto:alice@example.com"), "alice@example.com");
}

#[test]
fn non_mailbox_organizer_falls_back_to_email_param() {
    for value in [
        "/aNjQ2NjkxODM5NjQ2NjkxOLtVspI40y1Fxa98zI6-5H8FhO_dSJwJc-N39P2tilHW/principal/",
        "urn:uuid:12345-67890",
        "https://caldav.example.com/principals/alice/",
        "https://caldav.example.com/principals/users/alice/",
    ] {
        assert_eq!(
            organizer_email(&format!("ORGANIZER;CN=Alice;EMAIL=alice@example.com:{value}")),
            "alice@example.com"
        );
    }
}

#[test]
fn non_mailbox_organizer_without_email_param_keeps_value() {
    assert_eq!(organizer_email("ORGANIZER;CN=Alice:urn:uuid:12345-67890"), "urn:uuid:12345-67890");
}

#[test]
fn attendees_fall_back_per_row() {
    assert_eq!(
        vevent(&["ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED:mailto:bob@example.com"]).attendees[0].email,
        "bob@example.com"
    );
    let accepted = vevent(&["ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED;EMAIL=bob@example.com:/aNjQ2NjkxODM5/principal/"]);
    assert_eq!(accepted.attendees.len(), 1);
    assert_eq!(accepted.attendees[0].email, "bob@example.com");

    let mixed = vevent(&[
        "ATTENDEE;CN=Alice;PARTSTAT=ACCEPTED:mailto:alice@example.com",
        "ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED;EMAIL=bob@example.com:/aNjQ2NjkxODM5/principal/",
        "ATTENDEE;CN=Carol;PARTSTAT=NEEDS-ACTION:mailto:carol@example.com",
    ]);
    let emails: Vec<_> = mixed.attendees.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(emails, ["alice@example.com", "bob@example.com", "carol@example.com"]);
}

#[test]
fn empty_email_param_keeps_primary_value() {
    assert_eq!(organizer_email("ORGANIZER;CN=Alice;EMAIL=:/aNjQ2NjkxODM5/principal/"), "/aNjQ2NjkxODM5/principal/");
}

#[test]
fn mailto_with_empty_local_part_falls_back() {
    assert_eq!(organizer_email("ORGANIZER;CN=Alice;EMAIL=alice@example.com:mailto:@host"), "alice@example.com");
}

#[test]
fn email_param_name_is_case_insensitive() {
    assert_eq!(
        organizer_email("ORGANIZER;CN=Alice;email=alice@example.com:/aNjQ2NjkxODM5/principal/"),
        "alice@example.com"
    );
}

#[test]
fn principal_href_in_both_places_keeps_primary() {
    assert_eq!(
        organizer_email("ORGANIZER;CN=Alice;EMAIL=/another/principal/:/aNjQ2NjkxODM5/principal/"),
        "/aNjQ2NjkxODM5/principal/"
    );
}

#[test]
fn principal_href_rows_round_trip_verbatim() {
    let line = "ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED;EMAIL=bob@example.com:/aNjQ2NjkxODM5/principal/";
    let e = vevent(&[line]);
    assert!(lines(&generate(&e)).iter().any(|l| l == line));
}
