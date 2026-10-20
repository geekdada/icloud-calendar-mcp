//! Port of the VEVENT goldens of icaldav-core
//! `ICalGeneratorDelegationEquivalenceTest`.
//!
//! The PRODID is the crate constant rather than a constructor argument, and
//! DTSTAMP is always the generation time (there is no `preserveDtstamp`), so
//! both are normalized before comparing. Skipped: the METHOD:REQUEST golden,
//! the VTODO / VJOURNAL goldens and `includeVTimezone = false`.

mod ical_support;

use chrono::{TimeDelta, TimeZone};
use chrono_tz::Tz;
use ical_support::*;
use icloud_calendar_mcp::ical::generator::PRODID;
use icloud_calendar_mcp::ical::{AlarmAction, EventStatus, ICalAlarm, ICalDateTime, ICalEvent};

fn ny_event(uid: &str) -> ICalEvent {
    let zone = Tz::America__New_York;
    let at = |hour| ICalDateTime::from_zoned(zone.with_ymd_and_hms(2024, 3, 6, hour, 0, 0).unwrap(), false);
    let mut e = ICalEvent::new(uid, at(12));
    e.dt_end = Some(at(13));
    e.summary = Some("Event Summary".into());
    e.description = Some("Event Description".into());
    e.location = Some("Event Location".into());
    e.status = Some(EventStatus::Confirmed);
    e.dtstamp = Some(ICalDateTime::parse("20240101T000000Z", None).unwrap());
    e
}

/// Keep VTIMEZONE markers and TZID only, and pin PRODID and DTSTAMP.
fn normalize(ics: &str) -> String {
    let mut out = Vec::new();
    let mut in_vtimezone = false;
    for line in ics.split("\r\n").filter(|l| !l.is_empty()) {
        match line {
            "BEGIN:VTIMEZONE" => {
                in_vtimezone = true;
                out.push(line.to_string());
            }
            "END:VTIMEZONE" => {
                in_vtimezone = false;
                out.push(line.to_string());
            }
            _ if in_vtimezone && line.starts_with("TZID:") => out.push(line.to_string()),
            _ if in_vtimezone => {}
            _ if line == format!("PRODID:{PRODID}") => out.push("PRODID:<fixed>".into()),
            _ if line.starts_with("DTSTAMP:") => out.push("DTSTAMP:<now>".into()),
            _ => out.push(line.to_string()),
        }
    }
    out.join("\n")
}

const ENVELOPE_AND_BODY: &str = "BEGIN:VCALENDAR
VERSION:2.0
PRODID:<fixed>
CALSCALE:GREGORIAN
BEGIN:VTIMEZONE
TZID:America/New_York
END:VTIMEZONE
BEGIN:VEVENT
UID:{uid}
DTSTAMP:<now>
DTSTART;TZID=America/New_York:20240306T120000
DTEND;TZID=America/New_York:20240306T130000
SUMMARY:Event Summary
DESCRIPTION:Event Description
LOCATION:Event Location
STATUS:CONFIRMED
SEQUENCE:0";

#[test]
fn zoned_event_matches_golden() {
    let expected = format!("{}\nEND:VEVENT\nEND:VCALENDAR", ENVELOPE_AND_BODY.replace("{uid}", "evt-1"));
    assert_eq!(normalize(&generate(&ny_event("evt-1"))), expected);
}

#[test]
fn apple_alarm_extensions_match_golden() {
    let mut alarm = ICalAlarm::new(AlarmAction::Display);
    alarm.trigger = Some(TimeDelta::minutes(-15));
    alarm.description = Some("Reminder".into());
    alarm.uid = Some("ALARM-FIXED-UID-1".into());
    let mut event = ny_event("evt-alarm");
    event.alarms.push(alarm);

    let expected = format!(
        "{}\nBEGIN:VALARM\nUID:ALARM-FIXED-UID-1\nX-WR-ALARMUID:ALARM-FIXED-UID-1\nX-APPLE-DEFAULT-ALARM:FALSE\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR",
        ENVELOPE_AND_BODY.replace("{uid}", "evt-alarm")
    );
    assert_eq!(normalize(&generate(&event)), expected);
}
