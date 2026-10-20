//! Helpers shared by the ported icaldav-core suites (`tests/ical_*.rs`).

#![allow(dead_code)]

use icloud_calendar_mcp::ical::content::ContentLine;
use icloud_calendar_mcp::ical::generator::generate_calendar;
use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent};

/// Strip the common leading indentation and surrounding blank lines, like
/// Kotlin's `trimIndent`, so fixtures read the same as the Kotlin originals.
pub fn trim_indent(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(start, |i| i + 1);
    let body = &lines[start..end];
    let indent =
        body.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    body.iter().map(|l| if l.len() >= indent { &l[indent..] } else { l.trim_start() }).collect::<Vec<_>>().join("\n")
}

/// Wrap VEVENT text (indented however) in a VCALENDAR.
pub fn vcal(events: &str) -> String {
    format!("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\n{}\nEND:VCALENDAR", trim_indent(events))
}

/// Parse and unwrap every event.
pub fn parse(ics: &str) -> Vec<ICalEvent> {
    parse_all_events(&trim_indent(ics)).expect("calendar parses")
}

/// Parse a calendar expected to hold exactly one event.
pub fn parse_one(ics: &str) -> ICalEvent {
    let mut events = parse(ics);
    assert_eq!(events.len(), 1, "expected exactly one event");
    events.remove(0)
}

/// Generate a calendar for one event.
pub fn generate(event: &ICalEvent) -> String {
    generate_calendar(std::slice::from_ref(event))
}

/// Generate then re-parse one event.
pub fn round_trip(event: &ICalEvent) -> ICalEvent {
    let mut events = parse_all_events(&generate(event)).expect("generated calendar parses");
    assert_eq!(events.len(), 1);
    events.remove(0)
}

/// The first raw (unmodelled) property called `name`.
pub fn raw<'a>(event: &'a ICalEvent, name: &str) -> Option<&'a ContentLine> {
    event.raw_properties.iter().find(|p| p.name == name)
}

/// Epoch millis of an RFC 3339 instant.
pub fn millis(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp_millis()
}

/// A UTC DATE-TIME from an RFC 3339 instant.
pub fn utc(rfc3339: &str) -> ICalDateTime {
    ICalDateTime::utc(millis(rfc3339))
}

/// The unfolded content lines of generated output.
pub fn lines(ics: &str) -> Vec<String> {
    icloud_calendar_mcp::ical::content::unfold(ics).split("\r\n").map(str::to_string).collect()
}
