//! Port of the VEVENT parts of icaldav-core `ICalParserCalConnectComplianceTest`.
//!
//! Skipped: VTODO/VJOURNAL status, GEO on VTODO, ATTACH on VJOURNAL, mixed
//! component parsing, and VCALENDAR-level VERSION/PRODID/METHOD/CALSCALE
//! accessors (none are modelled). CLASS and GEO are raw lines.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::{EventStatus, ICalEvent, Transparency};

fn event(uid: &str, lines: &str) -> ICalEvent {
    parse_one(&vcal(&format!("BEGIN:VEVENT\nUID:{uid}\nDTSTART:20231215T140000Z\nSUMMARY:x\n{lines}\nEND:VEVENT")))
}

#[test]
fn uid_forms_are_preserved() {
    for uid in [
        "unique-123@example.com",
        "550e8400-e29b-41d4-a716-446655440000",
        "event-2023/12/15-meeting#001@corp.example.com",
    ] {
        assert_eq!(event(uid, "X-A:1").uid, uid);
    }
    let generated = parse_one(&vcal("BEGIN:VEVENT\nDTSTART:20231215T140000Z\nEND:VEVENT"));
    assert!(!generated.uid.trim().is_empty());
}

#[test]
fn recurrence_set_shares_uid() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:series@example.com\nDTSTART:20231201T100000Z\nRRULE:FREQ=WEEKLY;COUNT=4\nEND:VEVENT\nBEGIN:VEVENT\nUID:series@example.com\nRECURRENCE-ID:20231208T100000Z\nDTSTART:20231208T140000Z\nEND:VEVENT",
    ));
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.uid == "series@example.com"));
}

#[test]
fn start_end_requirements() {
    assert!(parse(&vcal("BEGIN:VEVENT\nUID:n\nSUMMARY:no start\nEND:VEVENT")).is_empty());
    let timed = event("t", "DTEND:20231215T150000Z");
    assert!(timed.dt_start.timestamp < timed.dt_end.unwrap().timestamp);
    assert!(!timed.dt_start.is_date && !timed.dt_end.unwrap().is_date);
    let all_day =
        parse_one(&vcal("BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20231225\nDTEND;VALUE=DATE:20231226\nEND:VEVENT"));
    assert!(all_day.dt_start.is_date && all_day.dt_end.unwrap().is_date);
    assert!(event("d", "DURATION:PT1H30M").duration.is_some());
    let both = event("b", "DTEND:20231215T160000Z\nDURATION:PT1H");
    assert!(both.dt_end.is_some());
}

#[test]
fn dtstamp_is_utc() {
    let stamp = event("s", "DTSTAMP:20231215T120000Z").dtstamp.unwrap();
    assert_eq!(stamp.to_ical_string(), "20231215T120000Z");
    assert!(stamp.is_utc);
}

#[test]
fn sequence_values_and_round_trip() {
    assert_eq!(event("s0", "X-A:1").sequence, 0);
    assert_eq!(event("s5", "SEQUENCE:5").sequence, 5);
    assert_eq!(round_trip(&event("s42", "DTEND:20231215T150000Z\nSEQUENCE:42")).sequence, 42);
}

#[test]
fn status_and_transparency() {
    assert_eq!(event("t", "STATUS:TENTATIVE").status, Some(EventStatus::Tentative));
    assert_eq!(event("c", "STATUS:CONFIRMED").status, Some(EventStatus::Confirmed));
    assert_eq!(event("x", "STATUS:CANCELLED").status, Some(EventStatus::Cancelled));
    assert_eq!(event("o", "TRANSP:OPAQUE").transparency, Transparency::Opaque);
    assert_eq!(event("f", "TRANSP:TRANSPARENT").transparency, Transparency::Transparent);
    assert!(generate(&event("f", "TRANSP:TRANSPARENT")).contains("TRANSP:TRANSPARENT\r\n"));
}

#[test]
fn categories() {
    assert_eq!(event("c1", "CATEGORIES:Work").categories, ["Work"]);
    assert_eq!(event("c2", "CATEGORIES:Work,Meeting,Important").categories, ["Work", "Meeting", "Important"]);
    assert_eq!(event("c3", "CATEGORIES:Work\nCATEGORIES:Personal").categories, ["Work", "Personal"]);
}

#[test]
fn class_is_kept_verbatim() {
    assert!(raw(&event("d", "X-A:1"), "CLASS").is_none());
    for class in ["PRIVATE", "CONFIDENTIAL"] {
        let e = event("c", &format!("CLASS:{class}"));
        assert_eq!(raw(&e, "CLASS").unwrap().value, class);
        assert!(generate(&e).contains(&format!("CLASS:{class}\r\n")));
    }
}

#[test]
fn created_and_last_modified() {
    let e = event("ts", "CREATED:20231201T100000Z\nLAST-MODIFIED:20231210T150000Z");
    let (created, modified) = (e.created.unwrap(), e.last_modified.unwrap());
    assert!(created.timestamp <= modified.timestamp);
    assert_eq!(created.to_ical_string(), "20231201T100000Z");
}

#[test]
fn geo_and_url() {
    let e = event("g", "GEO:37.386013;-122.082932\nLOCATION:Googleplex\nURL:https://example.com/meeting/123");
    assert_eq!(raw(&e, "GEO").unwrap().value, "37.386013;-122.082932");
    assert_eq!(e.url.as_deref(), Some("https://example.com/meeting/123"));
}

#[test]
fn events_keep_document_order() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:first@test.com\nDTSTART:20231215T100000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:second@test.com\nDTSTART:20231215T110000Z\nEND:VEVENT\nBEGIN:VTODO\nUID:todo@test.com\nEND:VTODO\nBEGIN:VEVENT\nUID:third@test.com\nDTSTART:20231215T120000Z\nEND:VEVENT",
    ));
    let uids: Vec<_> = events.iter().map(|e| e.uid.as_str()).collect();
    assert_eq!(uids, ["first@test.com", "second@test.com", "third@test.com"], "VTODO is ignored");
}
