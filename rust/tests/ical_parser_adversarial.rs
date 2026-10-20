//! Port of icaldav-core `ICalParserAdversarialTest`.

mod ical_support;

use chrono::TimeDelta;
use ical_support::*;
use icloud_calendar_mcp::ical::ICalEvent;
use icloud_calendar_mcp::ical::parser::parse_all_events;

fn events_of(body: &str) -> Vec<ICalEvent> {
    parse_all_events(&vcal(body)).expect("calendar parses")
}

fn event_with(lines: &str) -> ICalEvent {
    let mut events =
        events_of(&format!("BEGIN:VEVENT\nUID:adv@test.com\nDTSTART:20240101T090000Z\n{lines}\nEND:VEVENT"));
    assert_eq!(events.len(), 1);
    events.remove(0)
}

#[test]
fn empty_and_non_calendar_input() {
    for input in ["", "null", "   \n\t\r\n   "] {
        assert!(parse_all_events(input).is_err(), "{input:?}");
    }
}

#[test]
fn broken_structures_do_not_crash() {
    let missing_begin = "VERSION:2.0\nBEGIN:VEVENT\nUID:t\nDTSTART:20240101T090000Z\nEND:VEVENT\nEND:VCALENDAR";
    assert!(parse_all_events(missing_begin).is_err());
    let missing_end = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:t\nDTSTART:20240101T090000Z\nEND:VEVENT";
    assert!(parse_all_events(missing_end).is_err());
    let missing_event_end = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:t\nDTSTART:20240101T090000Z\nEND:VCALENDAR";
    assert!(parse_all_events(missing_event_end).is_err());
    let nested = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:n\nDTSTART:20240101T090000Z\nEND:VEVENT\nEND:VCALENDAR\nEND:VCALENDAR";
    assert!(parse_all_events(nested).unwrap().is_empty(), "only top-level VEVENTs count");
}

#[test]
fn vtimezone_rrule_does_not_pollute_event() {
    let events = events_of(
        "BEGIN:VTIMEZONE\nTZID:America/New_York\nBEGIN:DAYLIGHT\nDTSTART:19700308T020000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\nTZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nEND:DAYLIGHT\nEND:VTIMEZONE\nBEGIN:VEVENT\nUID:tz\nDTSTART;TZID=America/New_York:20240101T090000\nDTEND;TZID=America/New_York:20240101T100000\nSUMMARY:x\nEND:VEVENT",
    );
    assert_eq!(events.len(), 1);
    assert!(events[0].rrule.is_none());
}

#[test]
fn missing_properties_are_tolerated() {
    assert!(!events_of("BEGIN:VEVENT\nDTSTART:20240101T090000Z\nSUMMARY:x\nEND:VEVENT")[0].uid.is_empty());
    assert_eq!(events_of("BEGIN:VEVENT\nUID:a\nDTEND:20240101T100000Z\nEND:VEVENT").len(), 1);
    assert!(events_of("BEGIN:VEVENT\nUID:a\nDTSTART:20240101T090000Z\nEND:VEVENT")[0].dtstamp.is_none());
}

#[test]
fn large_values_and_many_properties() {
    let summary = "A".repeat(100_000);
    assert_eq!(event_with(&format!("SUMMARY:{summary}")).summary.unwrap().len(), 100_000);
    let description = "Test description line. ".repeat(10_000);
    assert_eq!(event_with(&format!("DESCRIPTION:{description}")).description.as_deref(), Some(description.as_str()));
    let attendees: Vec<String> = (1..=1000).map(|i| format!("ATTENDEE:mailto:user{i}@example.com")).collect();
    let event = event_with(&attendees.join("\n"));
    assert_eq!(event.attendees.len(), 1000);
    assert_eq!(event.attendees[999].email, "user1000@example.com");
    let alarms: Vec<String> = (1..=50)
        .map(|i| format!("BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT{i}M\nDESCRIPTION:Reminder {i}\nEND:VALARM"))
        .collect();
    let event = event_with(&alarms.join("\n"));
    assert_eq!(event.alarms.len(), 50);
    assert_eq!(event.alarms[49].trigger, Some(TimeDelta::minutes(-50)));
}

#[test]
fn line_folding_variants() {
    let crlf = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:f\r\nDTSTART:20240101T090000Z\r\nSUMMARY:This is folded across\r\n multiple lines\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    assert_eq!(parse_all_events(crlf).unwrap()[0].summary.as_deref(), Some("This is folded acrossmultiple lines"));
    let tab = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:t\r\nDTSTART:20240101T090000Z\r\nSUMMARY:Folded\r\n\twith tab\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    assert_eq!(parse_all_events(tab).unwrap()[0].summary.as_deref(), Some("Foldedwith tab"));

    let long = "This is a very long meeting description that contains important details about the meeting agenda and will be split across multiple folded lines.";
    let chunks: Vec<String> = long.chars().collect::<Vec<_>>().chunks(70).map(|c| c.iter().collect()).collect();
    let folded = format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:l\r\nDTSTART:20240101T090000Z\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        chunks.join("\r\n ")
    );
    assert_eq!(parse_all_events(&folded).unwrap()[0].description.as_deref(), Some(long));
}

#[test]
fn injection_payloads_are_literal_text() {
    let sql = events_of(
        "BEGIN:VEVENT\nUID:sql-inject'; DROP TABLE events;--@test.com\nDTSTART:20240101T090000Z\nSUMMARY:'; DROP TABLE events;--\nDESCRIPTION:Robert'); DROP TABLE events;--\nLOCATION:1'; DELETE FROM calendars WHERE '1'='1\nEND:VEVENT",
    );
    assert_eq!(sql[0].uid, "sql-inject'; DROP TABLE events;--@test.com");
    assert_eq!(sql[0].summary.as_deref(), Some("'; DROP TABLE events;--"));
    assert_eq!(
        event_with("DESCRIPTION:<script>alert('XSS')</script>").description.as_deref(),
        Some("<script>alert('XSS')</script>")
    );
    assert_eq!(event_with("SUMMARY:Test\u{1}\u{2}\u{3}Event").summary.as_deref(), Some("Test\u{1}\u{2}\u{3}Event"));
}

#[test]
fn invalid_dates_and_durations() {
    assert!(events_of("BEGIN:VEVENT\nUID:a\nDTSTART:not-a-date\nEND:VEVENT").is_empty());
    assert!(events_of("BEGIN:VEVENT\nUID:a\nDTSTART:20240230T090000Z\nEND:VEVENT").is_empty());
    let backwards = events_of("BEGIN:VEVENT\nUID:b\nDTSTART:20240101T100000Z\nDTEND:20240101T090000Z\nEND:VEVENT");
    assert!(backwards[0].dt_start.timestamp < backwards[0].dt_end.unwrap().timestamp, "inverted range is swapped");
    let negative = events_of("BEGIN:VEVENT\nUID:n\nDTSTART:20240101T100000Z\nDURATION:-PT1H\nEND:VEVENT");
    assert_eq!(negative[0].duration, Some(TimeDelta::hours(-1)));
}

#[test]
fn invalid_rrules() {
    assert!(event_with("RRULE:FREQ=BIWEEKLY;COUNT=10").rrule.is_none());
    let both = event_with("RRULE:FREQ=DAILY;COUNT=10;UNTIL=20240601T000000Z").rrule.unwrap();
    assert_eq!(both.count, Some(10));
}

#[test]
fn many_events_and_master_with_exception() {
    let pair = events_of(
        "BEGIN:VEVENT\nUID:same\nDTSTART:20240101T090000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:same\nRECURRENCE-ID:20240102T090000Z\nDTSTART:20240102T100000Z\nEND:VEVENT",
    );
    assert_eq!(pair.len(), 2);
    let bodies: Vec<String> = (1..=100)
        .map(|i| format!("BEGIN:VEVENT\r\nUID:bulk-{i}@test.com\r\nDTSTART:2024{:02}01T090000Z\r\nSUMMARY:Event {i}\r\nEND:VEVENT", i % 12 + 1))
        .collect();
    let ics = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}\r\nEND:VCALENDAR\r\n", bodies.join("\r\n"));
    assert_eq!(parse_all_events(&ics).unwrap().len(), 100);
}

#[test]
fn unicode_and_escapes() {
    assert_eq!(event_with("SUMMARY:会议 - Meeting 日本語 🎉").summary.as_deref(), Some("会议 - Meeting 日本語 🎉"));
    let escaped = event_with("SUMMARY:Test\\, with\\; special\\ncharacters");
    assert_eq!(escaped.summary.as_deref(), Some("Test, with; special\ncharacters"));
}
