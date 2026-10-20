//! Serializes events to RFC 5545 iCalendar text for CalDAV storage PUTs.
//!
//! iCloud rejects a body without CALSCALE, STATUS or SEQUENCE (HTTP 400), so
//! those are always written. No METHOD line is emitted: these are plain
//! resource bodies, not iTIP messages.

use chrono::Utc;

use super::content::{escape_text, push_folded, push_line};
use super::datetime::ICalDateTime;
use super::duration::format_duration;
use super::model::{AlarmAction, EventStatus, ICalAlarm, ICalEvent, Transparency};
use super::vtimezone::{UTC_TZIDS, generate_vtimezone};

/// PRODID written on every generated calendar.
pub const PRODID: &str = "-//OnekashMCP//AppleCalendarMCP 1.0//EN";

/// A VCALENDAR holding `events` (a master and its overrides share one resource),
/// preceded by a VTIMEZONE for every zone the events reference.
pub fn generate_calendar(events: &[ICalEvent]) -> String {
    let mut out = String::new();
    push_line(&mut out, "BEGIN:VCALENDAR");
    push_line(&mut out, "VERSION:2.0");
    push_line(&mut out, &format!("PRODID:{PRODID}"));
    push_line(&mut out, "CALSCALE:GREGORIAN");
    for tzid in collect_timezones(events) {
        out.push_str(&generate_vtimezone(&tzid));
    }
    for event in events {
        push_event(&mut out, event);
    }
    push_line(&mut out, "END:VCALENDAR");
    out
}

/// Non-UTC TZIDs referenced by the events' date properties, in first-seen order.
pub fn collect_timezones(events: &[ICalEvent]) -> Vec<String> {
    let mut tzids: Vec<String> = Vec::new();
    for event in events {
        let dates = [Some(event.dt_start), event.dt_end, event.recurrence_id]
            .into_iter()
            .flatten()
            .chain(event.exdates.iter().copied())
            .chain(event.rdates.iter().copied());
        for tzid in dates.filter_map(|dt| dt.tzid()) {
            if !UTC_TZIDS.contains(&tzid) && !tzids.iter().any(|t| t == tzid) {
                tzids.push(tzid.to_string());
            }
        }
    }
    tzids
}

fn push_event(out: &mut String, event: &ICalEvent) {
    push_line(out, "BEGIN:VEVENT");
    push_folded(out, &format!("UID:{}", event.uid));
    push_line(out, &format!("DTSTAMP:{}", Utc::now().format("%Y%m%dT%H%M%SZ")));
    push_date_property(out, "DTSTART", &event.dt_start, "");
    match (event.dt_end, event.duration) {
        (Some(end), _) => push_date_property(out, "DTEND", &end, ""),
        (None, Some(duration)) => push_line(out, &format!("DURATION:{}", format_duration(duration))),
        (None, None) => {}
    }
    if let Some(recurrence_id) = &event.recurrence_id {
        let range = event.recurrence_id_range.as_deref().map(|r| format!(";RANGE={r}")).unwrap_or_default();
        push_date_property(out, "RECURRENCE-ID", recurrence_id, &range);
    } else if let Some(rrule) = &event.rrule {
        push_line(out, &format!("RRULE:{}", rrule.to_ical_string()));
    }
    for exdate in &event.exdates {
        push_date_property(out, "EXDATE", exdate, "");
    }
    for rdate in &event.rdates {
        push_date_property(out, "RDATE", rdate, "");
    }
    for (name, value) in [("SUMMARY", &event.summary), ("DESCRIPTION", &event.description), ("LOCATION", &event.location)] {
        if let Some(value) = value {
            push_folded(out, &format!("{name}:{}", escape_text(value)));
        }
    }
    push_line(out, &format!("STATUS:{}", event.status.unwrap_or(EventStatus::Confirmed).as_str()));
    push_line(out, &format!("SEQUENCE:{}", event.sequence));
    if event.priority > 0 {
        push_line(out, &format!("PRIORITY:{}", event.priority));
    }
    if event.transparency == Transparency::Transparent {
        push_line(out, "TRANSP:TRANSPARENT");
    }
    if !event.categories.is_empty() {
        let joined: Vec<String> = event.categories.iter().map(|c| escape_text(c)).collect();
        push_folded(out, &format!("CATEGORIES:{}", joined.join(",")));
    }
    if let Some(url) = &event.url {
        push_folded(out, &format!("URL:{url}"));
    }
    if let Some(created) = &event.created {
        push_line(out, &format!("CREATED:{}", created.to_ical_string()));
    }
    if let Some(last_modified) = &event.last_modified {
        push_line(out, &format!("LAST-MODIFIED:{}", last_modified.to_ical_string()));
    }
    for raw in &event.raw_properties {
        push_folded(out, &raw.raw);
    }
    for alarm in &event.alarms {
        push_alarm(out, alarm);
    }
    push_line(out, "END:VEVENT");
}

fn push_alarm(out: &mut String, alarm: &ICalAlarm) {
    let uid = alarm.uid.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string().to_uppercase());
    let has_raw = |name: &str| alarm.raw_properties.iter().any(|p| p.name == name);
    let is_default_alarm = alarm
        .raw_properties
        .iter()
        .any(|p| p.name == "DEFAULT-ALARM" && p.value.trim().eq_ignore_ascii_case("TRUE"));

    push_line(out, "BEGIN:VALARM");
    push_folded(out, &format!("UID:{uid}"));
    // iCloud's alarm identifier; without X-APPLE-DEFAULT-ALARM:FALSE an iPhone
    // may merge the alarm into the calendar's default alarms.
    push_folded(out, &format!("X-WR-ALARMUID:{uid}"));
    if !is_default_alarm && !has_raw("X-APPLE-DEFAULT-ALARM") {
        push_line(out, "X-APPLE-DEFAULT-ALARM:FALSE");
    }
    push_line(out, &format!("ACTION:{}", alarm.action.as_str()));
    if let Some(trigger) = alarm.trigger {
        let related = if alarm.trigger_related_to_end { ";RELATED=END" } else { "" };
        push_line(out, &format!("TRIGGER{related}:{}", format_duration(trigger)));
    } else if let Some(absolute) = &alarm.trigger_absolute {
        push_line(out, &format!("TRIGGER;VALUE=DATE-TIME:{}", absolute.to_ical_string()));
    }
    // RFC 5545 §3.6.6: DISPLAY and EMAIL require DESCRIPTION; EMAIL requires SUMMARY.
    if matches!(alarm.action, AlarmAction::Display | AlarmAction::Email) {
        let description = alarm.description.as_deref().unwrap_or("Reminder");
        push_folded(out, &format!("DESCRIPTION:{}", escape_text(description)));
    }
    let summary = alarm.summary.as_deref().or((alarm.action == AlarmAction::Email).then_some("Reminder"));
    if let Some(summary) = summary {
        push_folded(out, &format!("SUMMARY:{}", escape_text(summary)));
    }
    if alarm.repeat_count > 0 {
        push_line(out, &format!("REPEAT:{}", alarm.repeat_count));
        if let Some(duration) = alarm.repeat_duration {
            push_line(out, &format!("DURATION:{}", format_duration(duration)));
        }
    }
    for raw in &alarm.raw_properties {
        push_folded(out, &raw.raw);
    }
    push_line(out, "END:VALARM");
}

/// `NAME;VALUE=DATE:…`, `NAME:…Z`, or `NAME;TZID=…:…`, with `extra` params
/// (already `;`-prefixed) before the value.
fn push_date_property(out: &mut String, name: &str, dt: &ICalDateTime, extra: &str) {
    let value = dt.to_ical_string();
    let line = if dt.is_date {
        format!("{name};VALUE=DATE{extra}:{value}")
    } else if let Some(tzid) = dt.tzid() {
        format!("{name};TZID={tzid}{extra}:{value}")
    } else {
        format!("{name}{extra}:{value}")
    };
    push_line(out, &line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::parser::parse_all_events;
    use crate::ical::rrule::RRule;
    use chrono::TimeDelta;
    use chrono_tz::Tz;

    fn sample() -> ICalEvent {
        let start = ICalDateTime::parse("20240115T090000", Some("America/New_York")).unwrap();
        let mut e = ICalEvent::new("uid-1", start);
        e.dt_end = Some(ICalDateTime::parse("20240115T100000", Some("America/New_York")).unwrap());
        e.summary = Some("Plan, review; ship".into());
        e.description = Some("multi\nline".into());
        e
    }

    #[test]
    fn writes_required_calendar_properties() {
        let ics = generate_calendar(&[sample()]);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//OnekashMCP//AppleCalendarMCP 1.0//EN\r\nCALSCALE:GREGORIAN\r\n"));
        assert!(!ics.contains("METHOD:"));
        assert!(ics.contains("STATUS:CONFIRMED\r\n"));
        assert!(ics.contains("SEQUENCE:0\r\n"));
        assert!(ics.contains("DTSTART;TZID=America/New_York:20240115T090000\r\n"));
        assert!(ics.contains("SUMMARY:Plan\\, review\\; ship\r\n"));
        assert!(ics.contains("DESCRIPTION:multi\\nline\r\n"));
        assert!(ics.contains("BEGIN:VTIMEZONE\r\nTZID:America/New_York"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(!ics.replace("\r\n", "").contains('\n'), "only CRLF line endings");
    }

    #[test]
    fn utc_and_date_values_need_no_vtimezone() {
        let mut e = ICalEvent::new("u", ICalDateTime::parse("20240115", None).unwrap());
        e.dt_end = Some(ICalDateTime::parse("20240116", None).unwrap());
        e.exdates.push(ICalDateTime::parse("20240117T100000Z", None).unwrap());
        let ics = generate_calendar(&[e]);
        assert!(ics.contains("DTSTART;VALUE=DATE:20240115\r\n"));
        assert!(ics.contains("EXDATE:20240117T100000Z\r\n"));
        assert!(!ics.contains("VTIMEZONE"));
    }

    #[test]
    fn duration_used_only_without_dtend() {
        let mut e = sample();
        e.dt_end = None;
        e.duration = Some(TimeDelta::minutes(45));
        e.rrule = Some(RRule::parse("FREQ=WEEKLY;BYDAY=MO").unwrap());
        let ics = generate_calendar(&[e]);
        assert!(ics.contains("DURATION:PT45M\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=MO\r\n"));
    }

    #[test]
    fn alarms_carry_apple_extensions() {
        let mut e = sample();
        let mut alarm = ICalAlarm::new(AlarmAction::Display);
        alarm.trigger = Some(TimeDelta::minutes(-15));
        e.alarms.push(alarm);
        let ics = generate_calendar(&[e]);
        assert!(ics.contains("X-WR-ALARMUID:"));
        assert!(ics.contains("X-APPLE-DEFAULT-ALARM:FALSE\r\n"));
        assert!(ics.contains("TRIGGER:-PT15M\r\n"));
        assert!(ics.contains("DESCRIPTION:Reminder\r\n"));
    }

    #[test]
    fn round_trip_preserves_model_and_raw_lines() {
        let source = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:rt\r\nDTSTART:20240101T100000Z\r\nDTEND:20240101T110000Z\r\nSUMMARY:Round trip\r\nCATEGORIES:A,B\\,C\r\nATTENDEE;CN=\"Doe, J\";PARTSTAT=ACCEPTED:mailto:j@example.com\r\nX-CUSTOM;X-P=1:value\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse_all_events(source).unwrap();
        let regenerated = generate_calendar(&events);
        assert!(regenerated.contains("ATTENDEE;CN=\"Doe, J\";PARTSTAT=ACCEPTED:mailto:j@example.com\r\n"));
        assert!(regenerated.contains("X-CUSTOM;X-P=1:value\r\n"));
        let again = parse_all_events(&regenerated).unwrap();
        assert_eq!(again[0].summary, events[0].summary);
        assert_eq!(again[0].categories, vec!["A", "B,C"]);
        assert_eq!(again[0].attendees, events[0].attendees);
        assert_eq!(again[0].dt_start, events[0].dt_start);
    }

    #[test]
    fn collects_distinct_zones_in_order() {
        let mut e = sample();
        e.dt_end = Some(ICalDateTime::from_zoned(
            chrono::TimeZone::with_ymd_and_hms(&Tz::America__Los_Angeles, 2024, 1, 15, 12, 0, 0).unwrap(),
            false,
        ));
        assert_eq!(collect_timezones(&[e]), vec!["America/New_York", "America/Los_Angeles"]);
    }
}
