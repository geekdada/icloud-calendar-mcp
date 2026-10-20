//! Port of icaldav-core `RecurringEventModificationTest`: a modified instance
//! is a VEVENT with the master's UID, a RECURRENCE-ID and no RRULE.
//!
//! The importId cases are not ported (the Rust model has no importId); the
//! "unique importIds" case is kept as "distinct RECURRENCE-IDs".

mod recurrence_support;

use icloud_calendar_mcp::ical::expander::expand;
use icloud_calendar_mcp::ical::generator::generate_calendar;
use icloud_calendar_mcp::ical::{Frequency, ICalEvent};
use recurrence_support::*;

fn master_of(events: &[ICalEvent]) -> &ICalEvent {
    events.iter().find(|e| e.recurrence_id.is_none()).expect("master event")
}

fn modified_of(events: &[ICalEvent]) -> &ICalEvent {
    events.iter().find(|e| e.recurrence_id.is_some()).expect("modified instance")
}

fn vevent_section(ics: &str) -> &str {
    let start = ics.find("BEGIN:VEVENT").expect("VEVENT");
    let end = ics[start..].find("END:VEVENT").expect("END:VEVENT");
    &ics[start..start + end]
}

#[test]
fn parses_master_and_modified_instance() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:daily-standup-123\nDTSTART:20241201T100000Z\nDTEND:20241201T103000Z\nRRULE:FREQ=DAILY;COUNT=10\n\
         SUMMARY:Daily Standup\nSTATUS:CONFIRMED\nSEQUENCE:0\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:daily-standup-123\nRECURRENCE-ID:20241205T100000Z\nDTSTART:20241205T140000Z\nDTEND:20241205T143000Z\n\
         SUMMARY:Daily Standup (Moved to 2pm)\nSTATUS:CONFIRMED\nSEQUENCE:1\nEND:VEVENT\n",
    ));
    assert_eq!(events.len(), 2);
    let master = master_of(&events);
    assert_eq!(master.uid, "daily-standup-123");
    assert!(master.rrule.is_some());
    assert_eq!(master.summary.as_deref(), Some("Daily Standup"));
    let modified = modified_of(&events);
    assert_eq!(modified.uid, "daily-standup-123");
    assert!(modified.rrule.is_none());
    assert_eq!(modified.summary.as_deref(), Some("Daily Standup (Moved to 2pm)"));
    assert_eq!(modified.sequence, 1);
}

#[test]
fn parses_recurrence_id_with_tzid() {
    let e = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:test-event\nRECURRENCE-ID;TZID=America/New_York:20241215T100000\n\
         DTSTART;TZID=America/New_York:20241215T140000\nDTEND;TZID=America/New_York:20241215T150000\n\
         SUMMARY:Modified Instance with TZ\nEND:VEVENT\n",
    ));
    let rid = e.recurrence_id.expect("RECURRENCE-ID");
    assert_eq!(rid.timezone, Some(NY));
    assert_eq!(rid.timestamp, instant(zdt(NY, 2024, 12, 15, 10, 0)).timestamp_millis());
}

#[test]
fn parses_date_recurrence_id_for_all_day_instance() {
    let e = parse_one(&vcal(
        "BEGIN:VEVENT\nUID:vacation\nRECURRENCE-ID;VALUE=DATE:20241225\nDTSTART;VALUE=DATE:20241226\n\
         DTEND;VALUE=DATE:20241227\nSUMMARY:Christmas (Moved to Dec 26)\nEND:VEVENT\n",
    ));
    assert!(e.recurrence_id.expect("RECURRENCE-ID").is_date);
    assert!(e.is_all_day);
}

#[test]
fn multiple_modified_instances_share_uid_with_distinct_recurrence_ids() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:daily-event\nDTSTART:20241201T100000Z\nRRULE:FREQ=DAILY;COUNT=10\nSUMMARY:Daily Event\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:daily-event\nRECURRENCE-ID:20241203T100000Z\nDTSTART:20241203T140000Z\nSUMMARY:Dec 3 moved\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:daily-event\nRECURRENCE-ID:20241205T100000Z\nDTSTART:20241205T160000Z\nSUMMARY:Dec 5 moved\nEND:VEVENT\n",
    ));
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|e| e.uid == "daily-event"));
    let mut ids: Vec<Option<i64>> = events.iter().map(|e| e.recurrence_id.map(|r| r.timestamp)).collect();
    ids.dedup();
    assert_eq!(ids.len(), 3);
}

fn daily_meeting() -> ICalEvent {
    let mut master = event(zoned(NY, 2024, 12, 1, 10, 0), Some(rule("FREQ=DAILY;COUNT=5")));
    master.uid = "daily-meeting".into();
    master.summary = Some("Daily Meeting".into());
    master
}

fn first_ten_days() -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    (instant(zdt(NY, 2024, 12, 1, 0, 0)), instant(zdt(NY, 2024, 12, 10, 0, 0)))
}

#[test]
fn expansion_replaces_original_occurrence_with_modified_instance() {
    let master = daily_meeting();
    let mut moved = event(zoned(NY, 2024, 12, 3, 14, 0), None);
    moved.uid = master.uid.clone();
    moved.summary = Some("Daily Meeting (Moved to 2pm)".into());
    moved.recurrence_id = Some(zoned(NY, 2024, 12, 3, 10, 0));

    let (start, end) = first_ten_days();
    let occ = expand(&master, start, end, &[moved]).unwrap();
    assert_eq!(occ.len(), 5);
    let dec3 = occ.iter().find(|o| local_day(o) == 3).unwrap();
    assert_eq!(dec3.summary.as_deref(), Some("Daily Meeting (Moved to 2pm)"));
    assert_eq!(times_in(std::slice::from_ref(dec3), NY), ["14:00"]);
}

#[test]
fn cancelled_occurrence_is_excluded() {
    let mut master = daily_meeting();
    master.exdates = vec![zoned(NY, 2024, 12, 3, 10, 0)];
    let (start, end) = first_ten_days();
    let occ = expand_all(&master, start, end);
    assert_eq!(day_codes(&occ), ["20241201", "20241202", "20241204", "20241205"]);
}

#[test]
fn master_generates_rrule_without_recurrence_id() {
    let mut e = event(zoned(NY, 2024, 12, 2, 10, 0), Some(rule("FREQ=WEEKLY;COUNT=8")));
    e.uid = "weekly-meeting".into();
    let ics = generate_calendar(&[e]);
    assert!(ics.contains("RRULE:FREQ=WEEKLY;COUNT=8"));
    assert!(!ics.contains("RECURRENCE-ID"));
}

#[test]
fn modified_instance_generates_recurrence_id_without_rrule() {
    let mut e = event(zoned(NY, 2024, 12, 9, 14, 0), None);
    e.uid = "weekly-meeting".into();
    e.recurrence_id = Some(zoned(NY, 2024, 12, 9, 10, 0));
    let ics = generate_calendar(&[e]);
    let vevent = vevent_section(&ics);
    assert!(vevent.contains("RECURRENCE-ID;TZID=America/New_York:20241209T100000"));
    // VTIMEZONE may carry RRULEs for DST; the VEVENT must not.
    assert!(!vevent.contains("RRULE"));
}

#[test]
fn utc_recurrence_id_is_generated_in_utc_form() {
    let mut e = event(zoned(NY, 2025, 12, 15, 9, 0), None);
    e.recurrence_id = Some(utc_dt(2025, 12, 15, 10, 0));
    let ics = generate_calendar(&[e]);
    assert!(ics.contains("RECURRENCE-ID:20251215T100000Z"), "{ics}");
    assert!(!ics.contains("RECURRENCE-ID;TZID="));
}

#[test]
fn master_and_modified_instance_survive_round_trip() {
    let events = parse(&vcal(
        "CALSCALE:GREGORIAN\nMETHOD:PUBLISH\n\
         BEGIN:VEVENT\nUID:roundtrip-test\nDTSTAMP:20241215T100000Z\nDTSTART:20241201T100000Z\nDTEND:20241201T110000Z\n\
         RRULE:FREQ=DAILY;COUNT=5\nSUMMARY:Daily Event\nSTATUS:CONFIRMED\nSEQUENCE:0\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:roundtrip-test\nDTSTAMP:20241215T100000Z\nRECURRENCE-ID:20241203T100000Z\n\
         DTSTART:20241203T150000Z\nDTEND:20241203T160000Z\nSUMMARY:Daily Event (Dec 3 Changed)\nSTATUS:CONFIRMED\nSEQUENCE:1\nEND:VEVENT\n",
    ));
    assert_eq!(events.len(), 2);
    let master = master_of(&events);
    let modified = modified_of(&events);

    let reparsed_master = parse_one(&generate_calendar(std::slice::from_ref(master)));
    let reparsed_modified = parse_one(&generate_calendar(std::slice::from_ref(modified)));

    assert_eq!(reparsed_master.uid, master.uid);
    assert_eq!(reparsed_master.rrule, master.rrule);
    assert!(reparsed_master.recurrence_id.is_none());
    assert_eq!(reparsed_modified.uid, modified.uid);
    assert!(reparsed_modified.rrule.is_none());
    assert_eq!(reparsed_modified.recurrence_id, modified.recurrence_id);

    let both = parse(&generate_calendar(&events));
    assert_eq!(both.len(), 2);
}

#[test]
fn vtimezone_rrules_are_not_taken_for_event_rrules() {
    let events = parse(
        "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Apple Inc.//iCloud//EN\n\
         BEGIN:VTIMEZONE\nTZID:America/New_York\n\
         BEGIN:STANDARD\nDTSTART:20071104T020000\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\nTZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nEND:STANDARD\n\
         BEGIN:DAYLIGHT\nDTSTART:20070311T020000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\nTZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nEND:DAYLIGHT\n\
         END:VTIMEZONE\n\
         BEGIN:VEVENT\nUID:icloud-event-123\nDTSTART;TZID=America/New_York:20241201T100000\nDTEND;TZID=America/New_York:20241201T110000\n\
         RRULE:FREQ=DAILY;COUNT=5\nSUMMARY:Team Standup\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:icloud-event-123\nRECURRENCE-ID;TZID=America/New_York:20241203T100000\n\
         DTSTART;TZID=America/New_York:20241203T140000\nDTEND;TZID=America/New_York:20241203T150000\nSUMMARY:Team Standup (Moved)\nEND:VEVENT\n\
         END:VCALENDAR\n",
    );
    assert_eq!(events.len(), 2);
    assert_eq!(master_of(&events).rrule.as_ref().map(|r| r.freq), Some(Frequency::Daily));
    assert!(modified_of(&events).rrule.is_none());
}

#[test]
fn modified_all_day_instance_stays_all_day() {
    let events = parse(&vcal(
        "BEGIN:VEVENT\nUID:vacation-2024\nDTSTART;VALUE=DATE:20241223\nDTEND;VALUE=DATE:20241227\nRRULE:FREQ=YEARLY\n\
         SUMMARY:Christmas Vacation\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:vacation-2024\nRECURRENCE-ID;VALUE=DATE:20251223\nDTSTART;VALUE=DATE:20251220\nDTEND;VALUE=DATE:20251228\n\
         SUMMARY:Christmas Vacation (Extended in 2025)\nEND:VEVENT\n",
    ));
    assert_eq!(events.len(), 2);
    let modified = modified_of(&events);
    assert!(modified.is_all_day);

    let occ = expand(
        master_of(&events),
        utc("2025-01-01T00:00:00Z"),
        utc("2026-01-01T00:00:00Z"),
        std::slice::from_ref(modified),
    )
    .unwrap();
    assert_eq!(day_codes(&occ), ["20251220"]);
    assert_eq!(occ[0].summary.as_deref(), Some("Christmas Vacation (Extended in 2025)"));
}
