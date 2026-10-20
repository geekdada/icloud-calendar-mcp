//! Port of icaldav-core `ICalParserRfc9074Test`.
//!
//! Only the alarm UID is modelled. ACKNOWLEDGED, RELATED-TO, DEFAULT-ALARM and
//! PROXIMITY stay verbatim raw alarm lines, so the `AlarmProximity` enum tests
//! are skipped and the field assertions become raw-line assertions.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::{ICalAlarm, ICalEvent};

fn event_with_alarm(alarm_lines: &str) -> ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:event-1\nDTSTAMP:20231215T100000Z\nDTSTART:20231215T140000Z\nSUMMARY:x\nBEGIN:VALARM\n{alarm_lines}\nEND:VALARM\nEND:VEVENT"
    )))
}

fn raw_value<'a>(alarm: &'a ICalAlarm, name: &str) -> Option<&'a str> {
    alarm.raw_properties.iter().find(|p| p.name == name).map(|p| p.value.as_str())
}

const ALL_RFC_9074: &str = "UID:full-alarm-uid\nACTION:DISPLAY\nTRIGGER:-PT30M\nDESCRIPTION:Full test\nACKNOWLEDGED:20231215T130000Z\nRELATED-TO:parent-alarm\nDEFAULT-ALARM:TRUE\nPROXIMITY:DEPART";

#[test]
fn alarm_uid_is_parsed() {
    let e = event_with_alarm("UID:alarm-uid-123\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder");
    assert_eq!(e.alarms.len(), 1);
    assert_eq!(e.alarms[0].uid.as_deref(), Some("alarm-uid-123"));
}

#[test]
fn rfc_9074_properties_are_kept_raw() {
    let alarm = &event_with_alarm(ALL_RFC_9074).alarms[0];
    assert_eq!(alarm.uid.as_deref(), Some("full-alarm-uid"));
    assert_eq!(raw_value(alarm, "ACKNOWLEDGED"), Some("20231215T130000Z"));
    assert_eq!(raw_value(alarm, "RELATED-TO"), Some("parent-alarm"));
    assert_eq!(raw_value(alarm, "DEFAULT-ALARM"), Some("TRUE"));
    assert_eq!(raw_value(alarm, "PROXIMITY"), Some("DEPART"));
}

#[test]
fn proximity_values_are_kept() {
    for value in ["ARRIVE", "DEPART"] {
        let e = event_with_alarm(&format!("ACTION:DISPLAY\nTRIGGER:-PT0M\nDESCRIPTION:d\nPROXIMITY:{value}"));
        assert_eq!(raw_value(&e.alarms[0], "PROXIMITY"), Some(value));
    }
}

#[test]
fn alarm_uid_survives_round_trip() {
    let once = event_with_alarm("UID:alarm-roundtrip-123\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder");
    assert_eq!(round_trip(&once).alarms[0].uid.as_deref(), Some("alarm-roundtrip-123"));
}

#[test]
fn all_rfc_9074_properties_survive_round_trip_once() {
    let once = event_with_alarm(ALL_RFC_9074);
    let twice = round_trip(&once);
    let alarm = &twice.alarms[0];
    assert_eq!(alarm.uid.as_deref(), Some("full-alarm-uid"));
    for name in ["ACKNOWLEDGED", "RELATED-TO", "DEFAULT-ALARM", "PROXIMITY"] {
        assert_eq!(raw_value(alarm, name), raw_value(&once.alarms[0], name), "{name}");
        assert_eq!(alarm.raw_properties.iter().filter(|p| p.name == name).count(), 1, "{name} is not duplicated");
    }
    assert!(raw_value(alarm, "X-APPLE-DEFAULT-ALARM").is_none(), "a default alarm is not marked non-default");
}

#[test]
fn plain_alarm_has_no_extensions_and_round_trips() {
    let once = event_with_alarm("ACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Basic Reminder");
    let alarm = &once.alarms[0];
    assert_eq!(alarm.uid, None);
    assert!(alarm.raw_properties.is_empty());
    let twice = round_trip(&once);
    assert_eq!(twice.alarms.len(), 1);
    // The generator assigns the UID iCloud needs, and it is stable from then on.
    let assigned = twice.alarms[0].uid.clone().expect("generated alarm UID");
    assert_eq!(round_trip(&twice).alarms[0].uid, Some(assigned));
}
