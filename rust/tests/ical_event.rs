//! Port of icaldav-core `ICalEventTest`.
//!
//! Skipped: importId generation/parsing and `effectiveEnd()` (neither is part
//! of the ported model), and the Classification / PartStat / AttendeeRole /
//! CUType enums (CLASS and ATTENDEE are verbatim raw lines). `isModifiedInstance`
//! and `masterUid` map to `recurrence_id.is_some()` and `uid`. The RFC 7986 /
//! RFC 9253 / attendee / organizer / rawProperties cases are expressed as raw
//! lines and checked through parse and generate.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::content::ContentLine;
use icloud_calendar_mcp::ical::{EventStatus, Frequency, ICalDateTime, ICalEvent, RRule, Transparency};

fn minimal(uid: &str) -> ICalEvent {
    let mut e = ICalEvent::new(uid, ICalDateTime::parse("20231215T100000Z", None).unwrap());
    e.summary = Some("Test Event".into());
    e.status = Some(EventStatus::Confirmed);
    e
}

fn with_raw(lines: &[&str]) -> ICalEvent {
    let mut e = minimal("test-uid-123");
    e.raw_properties = lines.iter().map(|l| ContentLine::parse(l).unwrap()).collect();
    e
}

#[test]
fn new_event_defaults() {
    let mut e = minimal("test-uid-123");
    e.summary = None;
    assert_eq!((e.summary.as_ref(), e.description.as_ref(), e.location.as_ref()), (None, None, None));
    assert_eq!((e.dt_end, e.duration, e.rrule.as_ref(), e.organizer.as_ref()), (None, None, None, None));
    assert!(!e.is_all_day);
    assert_eq!(e.transparency, Transparency::Opaque);

    let all_day = ICalEvent::new("d", ICalDateTime::parse("20231215", None).unwrap());
    assert!(all_day.is_all_day, "a DATE start makes the event all-day");
}

#[test]
fn is_recurring() {
    let single = minimal("s");
    assert!(!single.is_recurring());

    let mut rrule_only = single.clone();
    rrule_only.rrule = Some(RRule::new(Frequency::Daily));
    assert!(rrule_only.is_recurring());

    let mut rdate_only = single.clone();
    rdate_only.rdates = vec![ICalDateTime::parse("20231216T100000Z", None).unwrap()];
    assert!(rdate_only.is_recurring());

    let mut both = rrule_only.clone();
    both.rdates = rdate_only.rdates.clone();
    assert!(both.is_recurring());
}

#[test]
fn modified_instance_keeps_the_master_uid() {
    let master = minimal("master-uid");
    assert!(master.recurrence_id.is_none());
    let mut instance = master.clone();
    instance.recurrence_id = Some(ICalDateTime::parse("20231215T100000Z", None).unwrap());
    assert!(instance.recurrence_id.is_some());
    assert_eq!(instance.uid, "master-uid");
}

#[test]
fn status_values() {
    for (value, expected) in [
        ("CONFIRMED", EventStatus::Confirmed),
        ("confirmed", EventStatus::Confirmed),
        ("TENTATIVE", EventStatus::Tentative),
        ("tentative", EventStatus::Tentative),
        ("CANCELLED", EventStatus::Cancelled),
        ("cancelled", EventStatus::Cancelled),
        ("", EventStatus::Confirmed),
        ("UNKNOWN", EventStatus::Confirmed),
    ] {
        assert_eq!(EventStatus::from_value(value), expected, "{value:?}");
    }
    assert_eq!(EventStatus::Confirmed.as_str(), "CONFIRMED");
    assert_eq!(EventStatus::Tentative.as_str(), "TENTATIVE");
    assert_eq!(EventStatus::Cancelled.as_str(), "CANCELLED");
}

#[test]
fn transparency_values() {
    for (value, expected) in [
        (Some("OPAQUE"), Transparency::Opaque),
        (Some("opaque"), Transparency::Opaque),
        (Some("TRANSPARENT"), Transparency::Transparent),
        (Some("transparent"), Transparency::Transparent),
        (None, Transparency::Opaque),
        (Some(""), Transparency::Opaque),
        (Some("UNKNOWN"), Transparency::Opaque),
    ] {
        assert_eq!(Transparency::from_value(value), expected, "{value:?}");
    }
    let mut free = minimal("t");
    free.transparency = Transparency::Transparent;
    assert!(generate(&free).contains("TRANSP:TRANSPARENT\r\n"));
    // OPAQUE is the RFC 5545 default and is left implicit.
    assert!(!generate(&minimal("o")).contains("TRANSP:"));
}

#[test]
fn equality_and_clone() {
    assert_eq!(minimal("test-uid-123"), minimal("test-uid-123"));
    assert_ne!(minimal("uid-1"), minimal("uid-2"));
    let original = minimal("c");
    assert_eq!(original.clone(), original);
    let mut modified = original.clone();
    modified.summary = Some("Modified Title".into());
    assert_ne!(modified, original);
}

#[test]
fn empty_summary_is_kept() {
    let mut e = minimal("e");
    e.summary = Some(String::new());
    assert_eq!(e.summary.as_deref(), Some(""));
}

#[test]
fn rfc_7986_and_9253_properties_are_raw_lines() {
    let lines = [
        "IMAGE;VALUE=URI:https://example.com/image.png",
        "CONFERENCE;VALUE=URI:https://zoom.us/j/123",
        "COLOR:crimson",
        "LINK;VALUE=URI:https://example.com",
        "RELATED-TO:related-uid",
    ];
    let ics = generate(&with_raw(&lines));
    let emitted = ical_support::lines(&ics);
    for line in lines {
        assert!(emitted.iter().any(|l| l == line), "{line}");
    }
    let parsed = parse_one(&ics);
    for name in ["IMAGE", "CONFERENCE", "COLOR", "LINK", "RELATED-TO"] {
        assert!(raw(&parsed, name).is_some(), "{name}");
    }
}

#[test]
fn attendees_and_organizer_come_from_raw_lines() {
    let e = with_raw(&[
        "ORGANIZER;CN=Event Organizer:mailto:organizer@example.com",
        "ATTENDEE;CN=User 1;PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT;RSVP=TRUE:mailto:user1@example.com",
        "ATTENDEE;CN=User 2;PARTSTAT=TENTATIVE;ROLE=OPT-PARTICIPANT:mailto:user2@example.com",
    ]);
    let parsed = round_trip(&e);
    let organizer = parsed.organizer.unwrap();
    assert_eq!(organizer.email, "organizer@example.com");
    assert_eq!(organizer.name.as_deref(), Some("Event Organizer"));
    let emails: Vec<_> = parsed.attendees.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(emails, ["user1@example.com", "user2@example.com"]);
}

#[test]
fn unknown_properties_are_preserved() {
    let parsed = round_trip(&with_raw(&["X-CUSTOM-PROP:custom value", "X-ANOTHER:another value"]));
    let custom: Vec<_> = parsed.raw_properties.iter().filter(|p| p.name.starts_with("X-")).collect();
    assert_eq!(custom.len(), 2);
    assert_eq!(raw(&parsed, "X-CUSTOM-PROP").unwrap().value, "custom value");
}
