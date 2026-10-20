//! Port of the Kotlin `Rfc5545ComplianceAuditTest`: each test pins an RFC 5545
//! clause the server's producer and consumer paths exercise.

use chrono::{DateTime, Utc};
use icloud_calendar_mcp::ics::{AlarmSpec, NewEvent, build_event, parse_events};

fn timed(uid: &str, summary: &str) -> NewEvent {
    NewEvent {
        uid: Some(uid.into()),
        summary: summary.into(),
        start_time: Some("2026-01-15T10:00:00Z".into()),
        end_time: Some("2026-01-15T11:00:00Z".into()),
        ..NewEvent::default()
    }
}

fn instant(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// §3.1: content lines end with CRLF.
#[test]
fn section_3_1_produced_ics_uses_crlf_line_endings() {
    let ics = build_event(&timed("audit-3-1@example.com", "CRLF check")).unwrap();
    let crlf = ics.matches("\r\n").count();
    let lf = ics.matches('\n').count();
    assert_eq!(crlf, lf, "every LF must be preceded by CR:\n{ics}");
    assert!(crlf >= 6);
    let start = ics.find("BEGIN:VEVENT").unwrap();
    let end = ics.find("END:VEVENT").unwrap();
    assert!(!ics[start..end].contains("\r\n\r\n"), "no blank lines inside the VEVENT");
}

/// §3.6.1: an all-day DTEND is the day after the inclusive end.
#[test]
fn section_3_6_1_all_day_dtend_is_exclusive_on_the_wire() {
    let ics = build_event(&NewEvent {
        uid: Some("audit-3-6-1@example.com".into()),
        summary: "All-day exclusive end".into(),
        is_all_day: true,
        start_date: Some("2026-01-15".into()),
        end_date: Some("2026-01-15".into()),
        ..NewEvent::default()
    })
    .unwrap();
    assert!(ics.contains("DTSTART;VALUE=DATE:20260115"), "{ics}");
    assert!(ics.contains("DTEND;VALUE=DATE:20260116"), "{ics}");
}

/// §3.3.11: TEXT escapes are decoded exactly once.
#[test]
fn section_3_3_11_text_escapes_round_trip_without_double_decoding() {
    let ics = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//AuditTest//EN",
        "BEGIN:VEVENT",
        "UID:audit-3-3-11@example.com",
        "DTSTAMP:20260115T100000Z",
        "DTSTART:20260115T100000Z",
        "DTEND:20260115T110000Z",
        "SUMMARY:Escape test",
        "DESCRIPTION:back\\\\slash\\; comma\\, line\\nbreak",
        "END:VEVENT",
        "END:VCALENDAR",
    ]
    .join("\r\n")
        + "\r\n";
    let events = parse_events(&ics);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].description.as_deref(), Some("back\\slash; comma, line\nbreak"));
}

/// §3.8.5: a recurring event carries DURATION rather than DTEND.
#[test]
fn section_3_8_5_recurring_event_uses_duration_not_dtend() {
    let ics = build_event(&NewEvent {
        rrule: Some("FREQ=WEEKLY;BYDAY=TH".into()),
        ..timed("audit-3-8-5@example.com", "Weekly")
    })
    .unwrap();
    assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=TH"));
    assert!(ics.contains("DURATION:PT1H"), "{ics}");
    assert!(!ics.lines().any(|l| l.starts_with("DTEND")), "{ics}");
}

/// §3.8.7.1 / §3.8.7.3: CREATED and LAST-MODIFIED are emitted when known.
#[test]
fn section_3_8_7_builder_emits_created_and_last_modified() {
    let ics = build_event(&NewEvent {
        created_at: Some(instant("2026-01-10T08:30:00Z")),
        last_modified: Some(instant("2026-01-12T14:45:00Z")),
        ..timed("audit-3-8-7@example.com", "Timestamps")
    })
    .unwrap();
    assert!(ics.contains("CREATED:20260110T083000Z"), "{ics}");
    assert!(ics.contains("LAST-MODIFIED:20260112T144500Z"), "{ics}");
}

/// §3.6.6: VALARM carries ACTION and TRIGGER.
#[test]
fn section_3_6_6_builder_emits_valarm_with_action_and_trigger() {
    let ics = build_event(&NewEvent {
        alarms: Some(vec![AlarmSpec { trigger: "-PT15M".into(), ..AlarmSpec::default() }]),
        ..timed("audit-3-6-6@example.com", "With alarm")
    })
    .unwrap();
    for needle in ["BEGIN:VALARM", "ACTION:DISPLAY", "TRIGGER:-PT15M", "END:VALARM"] {
        assert!(ics.contains(needle), "missing {needle}:\n{ics}");
    }
}

/// §3.6.1 grammar: properties precede subcomponents inside a VEVENT.
#[test]
fn section_3_6_1_properties_precede_valarm_components() {
    let ics = build_event(&NewEvent {
        alarms: Some(vec![AlarmSpec { trigger: "-PT15M".into(), ..AlarmSpec::default() }]),
        created_at: Some(instant("2026-01-10T08:30:00Z")),
        ..timed("audit-order@example.com", "Ordering")
    })
    .unwrap();
    let alarm_end = ics.find("END:VALARM").unwrap();
    let event_end = ics.find("END:VEVENT").unwrap();
    assert_eq!(ics[alarm_end..event_end].trim_end(), "END:VALARM", "nothing between the last VALARM and END:VEVENT");
}
