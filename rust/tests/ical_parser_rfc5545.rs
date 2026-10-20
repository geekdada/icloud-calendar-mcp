//! Port of icaldav-core `ICalParserRfc5545ComplianceTest`.

mod ical_support;

use chrono::TimeDelta;
use ical_support::*;
use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ical::{EventStatus, Frequency, ICalEvent, Transparency};

fn event_with(lines: &str) -> ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:rfc@test.com\nDTSTART:20231215T140000Z\nSUMMARY:Test\n{}\nEND:VEVENT",
        trim_indent(lines)
    )))
}

#[test]
fn all_common_properties() {
    let e = event_with(
        "
        DTEND:20231215T150000Z
        DTSTAMP:20231201T120000Z
        DESCRIPTION:This is a detailed description.
        LOCATION:Conference Room A
        STATUS:CONFIRMED
        TRANSP:OPAQUE
        CLASS:PUBLIC
        SEQUENCE:2",
    );
    assert_eq!(e.uid, "rfc@test.com");
    assert_eq!(e.description.as_deref(), Some("This is a detailed description."));
    assert_eq!(e.location.as_deref(), Some("Conference Room A"));
    assert_eq!(e.status, Some(EventStatus::Confirmed));
    assert_eq!(e.transparency, Transparency::Opaque);
    assert_eq!(e.sequence, 2);
    assert_eq!(e.dtstamp.unwrap().to_ical_string(), "20231201T120000Z");
    assert_eq!(raw(&e, "CLASS").unwrap().value, "PUBLIC");
}

#[test]
fn status_and_sequence_values() {
    assert_eq!(event_with("STATUS:TENTATIVE").status, Some(EventStatus::Tentative));
    assert_eq!(event_with("STATUS:CANCELLED").status, Some(EventStatus::Cancelled));
    assert_eq!(event_with("SEQUENCE:42").sequence, 42);
}

#[test]
fn durations() {
    for (value, seconds) in [("PT2H30M", 9000), ("P1W2D", 777_600), ("P1D", 86_400), ("PT15M", 900)] {
        assert_eq!(event_with(&format!("DURATION:{value}")).duration, Some(TimeDelta::seconds(seconds)), "{value}");
    }
}

#[test]
fn line_folding() {
    let crlf = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:f\r\nDTSTART:20231215T140000Z\r\nSUMMARY:This is a very long event title that spans multiple lines bec\r\n ause it is longer than 75 characters\r\nEND:VEVENT\r\nEND:VCALENDAR";
    assert_eq!(
        parse_all_events(crlf).unwrap()[0].summary.as_deref(),
        Some("This is a very long event title that spans multiple lines because it is longer than 75 characters")
    );
    let lf = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:f\nDTSTART:20231215T140000Z\nSUMMARY:Long title\n folded\nEND:VEVENT\nEND:VCALENDAR";
    assert_eq!(parse_all_events(lf).unwrap()[0].summary.as_deref(), Some("Long titlefolded"));
}

#[test]
fn text_unescaping() {
    let e = event_with(
        r"LOCATION:Room\; Building A
DESCRIPTION:Line 1\nLine 2\nLine 3
COMMENT:x",
    );
    assert_eq!(e.location.as_deref(), Some("Room; Building A"));
    assert_eq!(e.description.as_deref(), Some("Line 1\nLine 2\nLine 3"));
    let comma = parse_one(&vcal(
        r"BEGIN:VEVENT
UID:c
DTSTART:20231215T140000Z
SUMMARY:Meeting\, Important!
END:VEVENT",
    ));
    assert_eq!(comma.summary.as_deref(), Some("Meeting, Important!"));
    let backslash = parse_one(&vcal(
        r"BEGIN:VEVENT
UID:b
DTSTART:20231215T140000Z
SUMMARY:Path: C:\\Users\\Test
END:VEVENT",
    ));
    assert_eq!(backslash.summary.as_deref(), Some(r"Path: C:\Users\Test"));
}

#[test]
fn rrules() {
    let daily = event_with("RRULE:FREQ=DAILY;COUNT=30").rrule.unwrap();
    assert_eq!((daily.freq, daily.count), (Frequency::Daily, Some(30)));
    let weekly = event_with("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR").rrule.unwrap();
    assert_eq!(weekly.freq, Frequency::Weekly);
    assert_eq!(weekly.by_day.unwrap().len(), 3);
    let monthly = event_with("RRULE:FREQ=MONTHLY;BYMONTHDAY=15").rrule.unwrap();
    assert_eq!(monthly.by_month_day, Some(vec![15]));
    assert!(event_with("RRULE:FREQ=DAILY;UNTIL=20240301T000000Z").rrule.unwrap().until.is_some());
    assert_eq!(event_with("RRULE:FREQ=WEEKLY;INTERVAL=2;COUNT=10").rrule.unwrap().interval, 2);
}

#[test]
fn exdates() {
    assert_eq!(event_with("RRULE:FREQ=DAILY;COUNT=10\nEXDATE:20231220T140000Z").exdates.len(), 1);
    assert_eq!(
        event_with("RRULE:FREQ=DAILY;COUNT=30\nEXDATE:20231220T140000Z,20231225T140000Z,20231231T140000Z")
            .exdates
            .len(),
        3
    );
}

#[test]
fn master_and_exceptions() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:recid-001@test.com\nDTSTART:20231201T100000Z\nRRULE:FREQ=WEEKLY;COUNT=10\nSUMMARY:Weekly\nEND:VEVENT\nBEGIN:VEVENT\nUID:recid-001@test.com\nRECURRENCE-ID:20231208T100000Z\nDTSTART:20231208T140000Z\nRRULE:FREQ=DAILY\nSUMMARY:Moved\nEND:VEVENT\nBEGIN:VEVENT\nUID:recid-001@test.com\nRECURRENCE-ID:20231215T100000Z\nDTSTART:20231215T100000Z\nSTATUS:CANCELLED\nSUMMARY:Cancelled\nEND:VEVENT",
    ));
    assert_eq!(events.len(), 3);
    assert!(events[0].rrule.is_some());
    assert!(events[1].rrule.is_none(), "an exception never carries an RRULE");
    assert_eq!(events[2].status, Some(EventStatus::Cancelled));
}

#[test]
fn alarms() {
    let e = event_with(
        "BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT1H\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nEND:VALARM",
    );
    let triggers: Vec<_> = e.alarms.iter().map(|a| a.trigger.unwrap()).collect();
    assert_eq!(triggers, [TimeDelta::minutes(-15), TimeDelta::hours(-1), TimeDelta::days(-1)]);
}

#[test]
fn organizer_and_missing_optional_text() {
    let e = event_with("ORGANIZER;CN=John Doe:mailto:john@example.com");
    let organizer = e.organizer.unwrap();
    assert_eq!(organizer.email, "john@example.com");
    assert_eq!(organizer.name.as_deref(), Some("John Doe"));
    let bare = event_with("X-A:1");
    assert_eq!(bare.description, None);
    assert_eq!(bare.location, None);
}
