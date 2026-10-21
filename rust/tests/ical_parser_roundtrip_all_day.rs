//! Port of icaldav-core `ICalParserRoundtripAllDayTest`.

mod ical_support;

use ical_support::*;
use icloud_calendar_mcp::ical::generator::generate_calendar;
use icloud_calendar_mcp::ical::parser::parse_all_events;

fn all_day(start: &str, end: &str, extra: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\nUID:ad\nDTSTART;VALUE=DATE:{start}\nDTEND;VALUE=DATE:{end}\nSUMMARY:x\n{extra}END:VEVENT"
    ))
}

#[test]
fn all_day_dates_survive_round_trip() {
    for (start, end) in [("20241225", "20241226"), ("20241224", "20241227"), ("20251011", "20251013")] {
        let again = round_trip(&parse_one(&all_day(start, end, "")));
        assert_eq!(again.dt_start.to_day_code(), start);
        assert_eq!(again.dt_end.unwrap().to_day_code(), end);
    }
}

#[test]
fn recurring_all_day_keeps_rule_and_date() {
    let original = parse_one(&all_day("20240101", "20240102", "RRULE:FREQ=DAILY;COUNT=10\n"));
    let again = round_trip(&original);
    assert!(again.is_all_day);
    assert_eq!(again.rrule, original.rrule);
    assert_eq!(again.dt_start.to_day_code(), "20240101");
}

#[test]
fn timed_recurring_exdate_text_and_alarm_events_round_trip() {
    let timed = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:t\nDTSTART:20241215T140000Z\nDTEND:20241215T150000Z\nSUMMARY:Timed\nEND:VEVENT",
    ));
    let again = round_trip(&timed);
    assert_eq!((again.uid.as_str(), again.summary.as_deref(), again.is_all_day), ("t", Some("Timed"), false));

    let recurring = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:r\nDTSTART:20241201T100000Z\nDTEND:20241201T110000Z\nSUMMARY:x\nRRULE:FREQ=DAILY;COUNT=30\nEXDATE:20241205T100000Z,20241208T100000Z\nEND:VEVENT",
    ));
    let again = round_trip(&recurring);
    assert_eq!(again.rrule, recurring.rrule);
    assert_eq!(again.exdates, recurring.exdates);

    let text = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:u\nDTSTART:20241215T140000Z\nSUMMARY:会议 Important\\, 🎉 kickoff\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT10M\nDESCRIPTION:r\nEND:VALARM\nEND:VEVENT",
    ));
    let again = round_trip(&text);
    assert_eq!(again.summary.as_deref(), Some("会议 Important, 🎉 kickoff"));
    assert_eq!(again.alarms.len(), 1);
}

#[test]
fn multiple_events_round_trip() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:1\nDTSTART:20241215T100000Z\nDTEND:20241215T110000Z\nSUMMARY:a\nEND:VEVENT\nBEGIN:VEVENT\nUID:2\nDTSTART;VALUE=DATE:20241216\nDTEND;VALUE=DATE:20241217\nSUMMARY:b\nEND:VEVENT\nBEGIN:VEVENT\nUID:3\nDTSTART:20241217T140000Z\nRRULE:FREQ=WEEKLY;COUNT=5\nSUMMARY:c\nEND:VEVENT",
    ));
    assert_eq!(events.len(), 3);
    let again = parse_all_events(&generate_calendar(&events)).unwrap();
    assert_eq!(again.len(), 3);
    assert_eq!(again.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), ["1", "2", "3"]);
    assert!(again[1].is_all_day);
    assert!(again[2].rrule.is_some());
}
