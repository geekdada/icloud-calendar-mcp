//! Maps parsed VEVENT/VALARM components onto the model.
//!
//! Property-level failures are tolerant: a property whose value cannot be read
//! is treated as absent, the way a relaxed parser suppresses invalid
//! properties. An event is dropped only when it has neither DTSTART nor a
//! usable DTEND to stand in for it.

use std::sync::LazyLock;

use chrono::TimeDelta;
use regex::Regex;

use super::ICalError;
use super::content::{Component, ContentLine, parse_calendar, split_text_list, unescape_text};
use super::datetime::ICalDateTime;
use super::duration::parse_duration;
use super::model::{AlarmAction, Attendee, EventStatus, ICalAlarm, ICalEvent, Organizer, Transparency};
use super::rrule::RRule;

/// Properties mapped onto dedicated [`ICalEvent`] fields; everything else is raw.
const HANDLED_EVENT_PROPERTIES: &[&str] = &[
    "UID",
    "DTSTART",
    "DTEND",
    "DURATION",
    "DTSTAMP",
    "RECURRENCE-ID",
    "RRULE",
    "EXDATE",
    "RDATE",
    "SUMMARY",
    "DESCRIPTION",
    "LOCATION",
    "STATUS",
    "SEQUENCE",
    "TRANSP",
    "URL",
    "PRIORITY",
    "LAST-MODIFIED",
    "CREATED",
    "CATEGORIES",
];

/// Properties mapped onto dedicated [`ICalAlarm`] fields; X-WR-ALARMUID is
/// regenerated from the alarm UID on output.
const HANDLED_ALARM_PROPERTIES: &[&str] =
    &["ACTION", "TRIGGER", "DESCRIPTION", "SUMMARY", "REPEAT", "DURATION", "UID", "X-WR-ALARMUID"];

/// A mailbox-shaped CAL-ADDRESS (as opposed to a principal href or `urn:uuid:`).
pub static MAILTO_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$").expect("valid regex"));

/// Parse every VEVENT in `ics`. Fails only when the data is not a well-formed
/// VCALENDAR; individual events that cannot be read are skipped.
pub fn parse_all_events(ics: &str) -> Result<Vec<ICalEvent>, ICalError> {
    let calendar = parse_calendar(ics)?;
    Ok(calendar.children_named("VEVENT").filter_map(parse_vevent).collect())
}

/// Map one VEVENT, or `None` when it has no usable start.
pub fn parse_vevent(vevent: &Component) -> Option<ICalEvent> {
    let start_prop = ["DTSTART", "DTEND"]
        .into_iter()
        .filter_map(|name| vevent.property(name))
        .find(|prop| parse_date_property(prop).is_some())?;
    let dt_start = parse_date_property(start_prop)?;
    let start_value = start_prop.value.trim();
    let is_all_day = start_prop.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || is_eight_digit_date(start_value)
        || !start_value.contains('T');

    let uid =
        vevent.property("UID").map(|p| p.value.trim().to_string()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let recurrence_prop = vevent.property("RECURRENCE-ID");
    let recurrence_id = recurrence_prop.and_then(parse_date_property);
    let recurrence_id_range = recurrence_prop
        .and_then(|p| p.param("RANGE"))
        .filter(|r| r.eq_ignore_ascii_case("THISANDFUTURE"))
        .map(|_| "THISANDFUTURE".to_string());

    let mut event = ICalEvent::new(uid, dt_start);
    event.is_all_day = is_all_day;
    event.dt_end = vevent.property("DTEND").and_then(parse_date_property);
    event.duration = vevent.property("DURATION").and_then(|p| parse_duration(&p.value));
    event.recurrence_id = recurrence_id;
    event.recurrence_id_range = recurrence_id.and(recurrence_id_range);
    event.rrule =
        if recurrence_id.is_none() { vevent.property("RRULE").and_then(|p| RRule::parse(&p.value).ok()) } else { None };
    event.exdates = vevent.properties_named("EXDATE").flat_map(|p| parse_date_list(p).unwrap_or_default()).collect();
    event.rdates = vevent
        .properties_named("RDATE")
        .filter(|p| !is_period(p))
        .flat_map(|p| {
            p.value.split(',').filter_map(|v| ICalDateTime::parse(v.trim(), p.param("TZID")).ok()).collect::<Vec<_>>()
        })
        .collect();
    event.alarms = vevent.children_named("VALARM").filter_map(parse_valarm).collect();
    event.categories = vevent
        .properties_named("CATEGORIES")
        .flat_map(|p| split_text_list(&p.value))
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    event.summary = text_property(vevent, "SUMMARY");
    event.description = text_property(vevent, "DESCRIPTION");
    event.location = text_property(vevent, "LOCATION");
    event.status = vevent.property("STATUS").map(|p| EventStatus::from_value(&p.value));
    event.sequence = vevent.property("SEQUENCE").and_then(|p| p.value.trim().parse().ok()).unwrap_or(0);
    event.transparency = Transparency::from_value(vevent.property("TRANSP").map(|p| p.value.as_str()));
    event.url = vevent.property("URL").map(|p| p.value.clone());
    event.priority = vevent
        .property("PRIORITY")
        .and_then(|p| p.value.trim().parse::<i32>().ok())
        .map(|p| p.clamp(0, 9))
        .unwrap_or(0);
    event.organizer = vevent
        .property("ORGANIZER")
        .map(|p| Organizer { email: cal_address_email(p), name: p.param("CN").map(str::to_string) });
    event.attendees = vevent
        .properties_named("ATTENDEE")
        .map(|p| Attendee { email: cal_address_email(p), name: p.param("CN").map(str::to_string) })
        .filter(|a| !a.email.trim().is_empty())
        .collect();
    event.dtstamp = vevent.property("DTSTAMP").and_then(parse_date_property);
    event.last_modified = vevent.property("LAST-MODIFIED").and_then(parse_date_property);
    event.created = vevent.property("CREATED").and_then(parse_date_property);
    event.raw_properties = vevent
        .properties
        .iter()
        .filter(|p| !HANDLED_EVENT_PROPERTIES.contains(&p.name.as_str()) || (p.name == "RDATE" && is_period(p)))
        .cloned()
        .collect();

    // Some servers write DTEND before DTSTART; swap rather than emit a negative span.
    if let Some(end) = event.dt_end
        && end.timestamp < event.dt_start.timestamp
    {
        event.dt_end = Some(event.dt_start);
        event.dt_start = end;
    }
    Some(event)
}

/// Map one VALARM, or `None` when its trigger cannot be read.
pub fn parse_valarm(valarm: &Component) -> Option<ICalAlarm> {
    let mut alarm = ICalAlarm::new(AlarmAction::from_value(valarm.property("ACTION").map(|p| p.value.as_str())));
    match valarm.property("TRIGGER") {
        Some(prop) => {
            let value = prop.value.trim();
            if value.starts_with(['-', '+', 'P', 'p']) {
                alarm.trigger = Some(parse_duration(value)?);
                alarm.trigger_related_to_end = prop.param("RELATED").is_some_and(|r| r.eq_ignore_ascii_case("END"));
            } else {
                alarm.trigger_absolute = Some(ICalDateTime::parse(value, prop.param("TZID")).ok()?);
            }
        }
        None => alarm.trigger = Some(TimeDelta::minutes(-15)),
    }
    alarm.description = valarm.property("DESCRIPTION").map(|p| unescape_text(&p.value));
    alarm.summary = valarm.property("SUMMARY").map(|p| unescape_text(&p.value));
    alarm.repeat_count = valarm.property("REPEAT").and_then(|p| p.value.trim().parse().ok()).unwrap_or(0);
    alarm.repeat_duration = valarm.property("DURATION").and_then(|p| parse_duration(&p.value));
    alarm.uid = valarm
        .property("UID")
        .or_else(|| valarm.property("X-WR-ALARMUID"))
        .map(|p| p.value.trim().to_string())
        .filter(|uid| !uid.is_empty());
    alarm.raw_properties =
        valarm.properties.iter().filter(|p| !HANDLED_ALARM_PROPERTIES.contains(&p.name.as_str())).cloned().collect();
    Some(alarm)
}

/// Read a DATE or DATE-TIME property, honoring TZID and VALUE=DATE.
pub fn parse_date_property(prop: &ContentLine) -> Option<ICalDateTime> {
    let value = prop.value.trim();
    let date_only = prop.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) || is_eight_digit_date(value);
    let value = if date_only { value.split('T').next().unwrap_or(value) } else { value };
    ICalDateTime::parse(value, prop.param("TZID")).ok()
}

/// The values of a comma-separated date list (EXDATE); `None` if any is unreadable.
fn parse_date_list(prop: &ContentLine) -> Option<Vec<ICalDateTime>> {
    prop.value.split(',').map(|v| ICalDateTime::parse(v.trim(), prop.param("TZID")).ok()).collect()
}

fn is_period(prop: &ContentLine) -> bool {
    prop.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("PERIOD"))
}

fn is_eight_digit_date(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit())
}

fn text_property(component: &Component, name: &str) -> Option<String> {
    component.property(name).map(|p| unescape_text(&p.value))
}

/// The mailbox of an ORGANIZER/ATTENDEE value.
///
/// iCloud rewrites the account's own address to a principal href and keeps the
/// mailto in an `EMAIL=` parameter, so that parameter is the fallback when the
/// value itself is not mailbox-shaped. Otherwise the bare value is returned.
pub fn cal_address_email(prop: &ContentLine) -> String {
    let primary = strip_mailto(&prop.value);
    if MAILTO_SHAPE.is_match(&primary) {
        return primary;
    }
    prop.param("EMAIL")
        .filter(|v| !v.trim().is_empty())
        .map(strip_mailto)
        .filter(|email| MAILTO_SHAPE.is_match(email))
        .unwrap_or(primary)
}

fn strip_mailto(value: &str) -> String {
    let trimmed = value.trim().trim_start_matches('"').trim_end_matches('"').trim();
    let without_scheme =
        if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("mailto:") { &trimmed[7..] } else { trimmed };
    without_scheme.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Tz;

    fn wrap(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//t//EN\r\n{body}END:VCALENDAR\r\n")
    }

    fn one(body: &str) -> ICalEvent {
        let mut events = parse_all_events(&wrap(body)).unwrap();
        assert_eq!(events.len(), 1);
        events.remove(0)
    }

    #[test]
    fn parses_basic_timed_event() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:abc@x\r\nDTSTART;TZID=America/New_York:20240115T090000\r\nDTEND;TZID=America/New_York:20240115T100000\r\nSUMMARY:Stand\\, up\r\nDESCRIPTION:Line1\\nLine2\r\nLOCATION:Room 1\r\nSTATUS:TENTATIVE\r\nSEQUENCE:3\r\nPRIORITY:12\r\nEND:VEVENT\r\n",
        );
        assert_eq!(e.uid, "abc@x");
        assert!(!e.is_all_day);
        assert_eq!(e.dt_start.timezone, Some(Tz::America__New_York));
        assert_eq!(e.summary.as_deref(), Some("Stand, up"));
        assert_eq!(e.description.as_deref(), Some("Line1\nLine2"));
        assert_eq!(e.status, Some(EventStatus::Tentative));
        assert_eq!(e.sequence, 3);
        assert_eq!(e.priority, 9);
    }

    #[test]
    fn detects_all_day_and_missing_status() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART;VALUE=DATE:20240115\r\nDTEND;VALUE=DATE:20240116\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        assert!(e.is_all_day);
        assert!(e.dt_start.is_date);
        assert_eq!(e.status, None);
    }

    #[test]
    fn falls_back_to_dtend_and_swaps_inverted_range() {
        let e = one("BEGIN:VEVENT\r\nUID:a\r\nDTEND:20240115T100000Z\r\nSUMMARY:x\r\nEND:VEVENT\r\n");
        assert_eq!(e.dt_start.to_ical_string(), "20240115T100000Z");
        let swapped = one(
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20240115T110000Z\r\nDTEND:20240115T100000Z\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        assert_eq!(swapped.dt_start.to_ical_string(), "20240115T100000Z");
        assert_eq!(swapped.dt_end.unwrap().to_ical_string(), "20240115T110000Z");
    }

    #[test]
    fn drops_event_without_any_start() {
        let events = parse_all_events(&wrap("BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:x\r\nEND:VEVENT\r\n")).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn parses_recurrence_fields() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:r\r\nDTSTART:20240101T100000Z\r\nDURATION:PT30M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE:20240108T100000Z,20240115T100000Z\r\nRDATE;VALUE=DATE-TIME:20240103T100000Z\r\nRDATE;VALUE=PERIOD:20240104T100000Z/PT1H\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        assert_eq!(e.duration, Some(TimeDelta::minutes(30)));
        assert!(e.rrule.is_some());
        assert_eq!(e.exdates.len(), 2);
        assert_eq!(e.rdates.len(), 1);
        assert!(e.raw_properties.iter().any(|p| p.name == "RDATE"));
    }

    #[test]
    fn override_has_recurrence_id_and_no_rrule() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:r\r\nRECURRENCE-ID;RANGE=THISANDFUTURE:20240108T100000Z\r\nDTSTART:20240108T110000Z\r\nRRULE:FREQ=DAILY\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        assert!(e.recurrence_id.is_some());
        assert_eq!(e.recurrence_id_range.as_deref(), Some("THISANDFUTURE"));
        assert!(e.rrule.is_none());
    }

    #[test]
    fn unreadable_rrule_is_ignored_not_fatal() {
        let e =
            one("BEGIN:VEVENT\r\nUID:r\r\nDTSTART:20240101T100000Z\r\nRRULE:FREQ=NEVER\r\nSUMMARY:x\r\nEND:VEVENT\r\n");
        assert!(e.rrule.is_none());
    }

    #[test]
    fn parses_alarms_and_keeps_unknown_alarm_properties() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20240101T100000Z\r\nSUMMARY:x\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=END:-PT15M\r\nDESCRIPTION:Hi\r\nX-WR-ALARMUID:ABC\r\nX-APPLE-DEFAULT-ALARM:TRUE\r\nEND:VALARM\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER;VALUE=DATE-TIME:20240101T093000Z\r\nREPEAT:2\r\nDURATION:PT5M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        assert_eq!(e.alarms.len(), 2);
        let first = &e.alarms[0];
        assert_eq!(first.trigger, Some(TimeDelta::minutes(-15)));
        assert!(first.trigger_related_to_end);
        assert_eq!(first.uid.as_deref(), Some("ABC"));
        assert_eq!(first.raw_properties.len(), 1);
        let second = &e.alarms[1];
        assert_eq!(second.action, AlarmAction::Email);
        assert_eq!(second.trigger_absolute.unwrap().to_ical_string(), "20240101T093000Z");
        assert_eq!(second.repeat_count, 2);
        assert_eq!(second.repeat_duration, Some(TimeDelta::minutes(5)));
    }

    #[test]
    fn organizer_falls_back_to_email_param() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20240101T100000Z\r\nSUMMARY:x\r\nORGANIZER;CN=Jane;EMAIL=jane@example.com:/123/principal/\r\nATTENDEE;CN=Bob:mailto:bob@example.com\r\nATTENDEE:mailto:\r\nEND:VEVENT\r\n",
        );
        let org = e.organizer.unwrap();
        assert_eq!(org.email, "jane@example.com");
        assert_eq!(org.name.as_deref(), Some("Jane"));
        assert_eq!(e.attendees.len(), 1);
        assert_eq!(e.attendees[0].email, "bob@example.com");
        assert!(e.raw_properties.iter().any(|p| p.name == "ORGANIZER"));
    }

    #[test]
    fn categories_split_and_blank_dropped() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20240101T100000Z\r\nCATEGORIES:Work, Home,,A\\,B\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        assert_eq!(e.categories, vec!["Work", "Home", "A,B"]);
    }

    #[test]
    fn keeps_x_properties_raw() {
        let e = one(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20240101T100000Z\r\nSUMMARY:x\r\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\nEND:VEVENT\r\n",
        );
        assert_eq!(e.raw_properties[0].raw, "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC");
    }

    #[test]
    fn errors_on_non_calendar() {
        assert!(parse_all_events("garbage").is_err());
    }
}
