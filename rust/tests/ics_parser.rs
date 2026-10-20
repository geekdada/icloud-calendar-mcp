//! Port of the Kotlin `IcsParserTest`: flattening VEVENTs into `ParsedEvent`s.

use icloud_calendar_mcp::ics::parse_events;

/// Kotlin `trimIndent` for test fixtures: trims each line, drops blank ones,
/// joins with LF (the Kotlin raw strings were LF-only).
fn ics(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn event_body(body: &str) -> String {
    ics(&format!("BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\n{body}\nEND:VEVENT\nEND:VCALENDAR"))
}

#[test]
fn parse_simple_event_with_all_fields() {
    let events = parse_events(&ics("
        BEGIN:VCALENDAR
        VERSION:2.0
        PRODID:-//Test//Test//EN
        BEGIN:VEVENT
        UID:test-event-123@example.com
        DTSTART:20250115T090000Z
        DTEND:20250115T100000Z
        SUMMARY:Team Meeting
        DESCRIPTION:Weekly sync meeting
        LOCATION:Conference Room A
        END:VEVENT
        END:VCALENDAR
    "));
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.uid, "test-event-123@example.com");
    assert_eq!(e.summary, "Team Meeting");
    assert_eq!(e.description.as_deref(), Some("Weekly sync meeting"));
    assert_eq!(e.location.as_deref(), Some("Conference Room A"));
    assert!(!e.is_all_day);
    assert_eq!(e.start_time.as_deref(), Some("2025-01-15T09:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-01-15T10:00:00Z"));
}

#[test]
fn parse_event_with_missing_optional_fields() {
    let events = parse_events(&event_body(
        "UID:minimal@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Minimal Event",
    ));
    assert_eq!(events.len(), 1);
    assert!(events[0].description.is_none());
    assert!(events[0].location.is_none());
}

#[test]
fn parse_multiple_events() {
    let events = parse_events(&ics("
        BEGIN:VCALENDAR
        VERSION:2.0
        BEGIN:VEVENT
        UID:event1@test
        DTSTART:20250115T090000Z
        DTEND:20250115T100000Z
        SUMMARY:Event 1
        END:VEVENT
        BEGIN:VEVENT
        UID:event2@test
        DTSTART:20250115T100000Z
        DTEND:20250115T110000Z
        SUMMARY:Event 2
        END:VEVENT
        END:VCALENDAR
    "));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].summary, "Event 1");
    assert_eq!(events[1].summary, "Event 2");
}

#[test]
fn parse_returns_empty_list_for_invalid_or_empty_ics() {
    assert!(parse_events("not valid ics").is_empty());
    assert!(parse_events("").is_empty());
}

#[test]
fn parse_all_day_event_single_day() {
    let events = parse_events(&event_body(
        "UID:allday@test\nDTSTART;VALUE=DATE:20250115\nDTEND;VALUE=DATE:20250116\nSUMMARY:All Day Event",
    ));
    let e = &events[0];
    assert!(e.is_all_day);
    assert_eq!(e.start_date.as_deref(), Some("2025-01-15"));
    assert_eq!(e.end_date.as_deref(), Some("2025-01-15"), "DTEND is exclusive");
}

#[test]
fn parse_all_day_multi_day_event() {
    let events = parse_events(&event_body(
        "UID:multiday@test\nDTSTART;VALUE=DATE:20250115\nDTEND;VALUE=DATE:20250118\nSUMMARY:Three Day Event",
    ));
    assert_eq!(events[0].start_date.as_deref(), Some("2025-01-15"));
    assert_eq!(events[0].end_date.as_deref(), Some("2025-01-17"));
}

#[test]
fn all_day_event_year_boundary() {
    let events = parse_events(&event_body(
        "UID:yearend@test\nDTSTART;VALUE=DATE:20241231\nDTEND;VALUE=DATE:20250102\nSUMMARY:New Year",
    ));
    assert_eq!(events[0].start_date.as_deref(), Some("2024-12-31"));
    assert_eq!(events[0].end_date.as_deref(), Some("2025-01-01"));
}

#[test]
fn parse_utc_datetime() {
    let events =
        parse_events(&event_body("UID:utc@test\nDTSTART:20250115T140000Z\nDTEND:20250115T150000Z\nSUMMARY:UTC Event"));
    assert_eq!(events[0].start_time.as_deref(), Some("2025-01-15T14:00:00Z"));
}

#[test]
fn parse_event_with_tzid_converts_to_utc() {
    let events = parse_events(&event_body(
        "UID:tz@test\nDTSTART;TZID=America/New_York:20250115T090000\nDTEND;TZID=America/New_York:20250115T100000\nSUMMARY:NYC Event",
    ));
    assert_eq!(events[0].start_time.as_deref(), Some("2025-01-15T14:00:00Z"));
}

#[test]
fn parse_event_with_pacific_timezone() {
    let events = parse_events(&event_body(
        "UID:pacific@test\nDTSTART;TZID=America/Los_Angeles:20250115T090000\nDTEND;TZID=America/Los_Angeles:20250115T100000\nSUMMARY:LA Event",
    ));
    assert_eq!(events[0].start_time.as_deref(), Some("2025-01-15T17:00:00Z"));
}

#[test]
fn parse_event_with_folded_lines() {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:folded@test\r\nDTSTART:20250115T090000Z\r\nDTEND:20250115T100000Z\r\nSUMMARY:This is a very long event title that spans multiple lines becau\r\n se it exceeds 75 characters\r\nEND:VEVENT\r\nEND:VCALENDAR";
    let events = parse_events(ics);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].summary,
        "This is a very long event title that spans multiple lines because it exceeds 75 characters"
    );
}

#[test]
fn parse_description_with_folded_lines() {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:desc@test\r\nDTSTART:20250115T090000Z\r\nDTEND:20250115T100000Z\r\nSUMMARY:Meeting\r\nDESCRIPTION:This is a long description that needs to be folded across m\r\n ultiple lines according to RFC 5545 specification\r\nEND:VEVENT\r\nEND:VCALENDAR";
    let events = parse_events(ics);
    assert!(events[0].description.as_deref().unwrap().contains("multiple lines"));
}

#[test]
fn parse_unescapes_newlines() {
    let events = parse_events(&event_body(
        "UID:escape@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Event\nDESCRIPTION:Line 1\\nLine 2\\nLine 3",
    ));
    assert_eq!(events[0].description.as_deref(), Some("Line 1\nLine 2\nLine 3"));
}

#[test]
fn parse_unescapes_commas_and_semicolons() {
    let events = parse_events(&event_body(
        "UID:comma@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Meeting with A\\, B\\, and C\nLOCATION:Room 1\\; Building A",
    ));
    assert_eq!(events[0].summary, "Meeting with A, B, and C");
    assert_eq!(events[0].location.as_deref(), Some("Room 1; Building A"));
}

#[test]
fn parse_unescapes_backslashes() {
    let events = parse_events(&event_body(
        "UID:backslash@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Path: C:\\\\Users\\\\Name",
    ));
    assert_eq!(events[0].summary, "Path: C:\\Users\\Name");
}

#[test]
fn parse_recurring_event_with_rrule() {
    let events = parse_events(&event_body(
        "UID:recurring@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Weekly Meeting\nRRULE:FREQ=WEEKLY;BYDAY=WE;COUNT=10",
    ));
    let rrule = events[0].rrule.as_deref().unwrap();
    assert!(rrule.contains("FREQ=WEEKLY"));
    assert!(rrule.contains("BYDAY=WE"));
    assert!(rrule.contains("COUNT=10"));
}

#[test]
fn parse_recurring_daily_and_monthly_events() {
    let daily = parse_events(&event_body(
        "UID:daily@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Daily Standup\nRRULE:FREQ=DAILY;INTERVAL=1",
    ));
    assert!(daily[0].rrule.as_deref().unwrap().contains("FREQ=DAILY"));
    let monthly = parse_events(&event_body(
        "UID:monthly@test\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Monthly Review\nRRULE:FREQ=MONTHLY;BYMONTHDAY=15",
    ));
    assert!(monthly[0].rrule.as_deref().unwrap().contains("FREQ=MONTHLY"));
}

const BASE: &str = "DTSTART:20250115T090000Z\nDTEND:20250115T100000Z";

#[test]
fn parse_event_with_status_tentative() {
    let events =
        parse_events(&event_body(&format!("UID:status@test\n{BASE}\nSUMMARY:Maybe Meeting\nSTATUS:TENTATIVE")));
    assert_eq!(events[0].status.as_deref(), Some("TENTATIVE"));
}

#[test]
fn parse_event_with_url() {
    let events = parse_events(&event_body(&format!(
        "UID:url@test\n{BASE}\nSUMMARY:Web Meeting\nURL:https://meet.example.com/room"
    )));
    assert_eq!(events[0].url.as_deref(), Some("https://meet.example.com/room"));
}

#[test]
fn parse_event_with_categories() {
    let events =
        parse_events(&event_body(&format!("UID:cat@test\n{BASE}\nSUMMARY:Categorized\nCATEGORIES:Work,Meeting")));
    assert_eq!(events[0].categories, vec!["Work", "Meeting"]);
}

#[test]
fn parse_event_with_priority() {
    let events = parse_events(&event_body(&format!("UID:pri@test\n{BASE}\nSUMMARY:High Priority\nPRIORITY:1")));
    assert_eq!(events[0].priority, Some(1));
}

#[test]
fn parse_event_with_organizer() {
    let events = parse_events(&event_body(&format!(
        "UID:org@test\n{BASE}\nSUMMARY:Organized\nORGANIZER;CN=John Doe:mailto:john@example.com"
    )));
    assert_eq!(events[0].organizer.as_deref(), Some("John Doe <john@example.com>"));
}

#[test]
fn parse_event_with_attendee_count() {
    let events = parse_events(&event_body(&format!(
        "UID:att@test\n{BASE}\nSUMMARY:Team Meeting\nATTENDEE;CN=Alice:mailto:alice@example.com\nATTENDEE;CN=Bob:mailto:bob@example.com\nATTENDEE;CN=Charlie:mailto:charlie@example.com"
    )));
    assert_eq!(events[0].attendee_count, 3);
}

#[test]
fn parse_event_with_no_extended_fields_has_defaults() {
    let events = parse_events(&event_body(&format!("UID:default@test\n{BASE}\nSUMMARY:Plain Event")));
    let e = &events[0];
    assert!(e.status.is_none());
    assert!(e.url.is_none());
    assert!(e.categories.is_empty());
    assert!(e.priority.is_none());
    assert!(e.organizer.is_none());
    assert_eq!(e.attendee_count, 0);
}

#[test]
fn parse_event_with_international_characters() {
    let jp = parse_events(&event_body(
        "UID:japanese@test\nDTSTART;VALUE=DATE:20250101\nDTEND;VALUE=DATE:20250102\nSUMMARY:元日",
    ));
    assert_eq!(jp[0].summary, "元日");
    let de =
        parse_events(&event_body(&format!("UID:german@test\n{BASE}\nSUMMARY:Bürobesprechung\nLOCATION:Düsseldorf")));
    assert_eq!(de[0].summary, "Bürobesprechung");
    assert_eq!(de[0].location.as_deref(), Some("Düsseldorf"));
}

#[test]
fn parse_event_with_duration_instead_of_dtend() {
    let events = parse_events(&event_body(
        "UID:duration@test\nDTSTART:20250115T090000Z\nDURATION:PT2H30M\nSUMMARY:Long Meeting",
    ));
    assert_eq!(events[0].end_time.as_deref(), Some("2025-01-15T11:30:00Z"));
}

#[test]
fn parse_event_without_dtend_or_duration_defaults_to_one_hour() {
    let events = parse_events(&event_body("UID:noend@test\nDTSTART:20250115T090000Z\nSUMMARY:Quick Meeting"));
    assert_eq!(events[0].end_time.as_deref(), Some("2025-01-15T10:00:00Z"));
}

#[test]
fn skip_cancelled_events() {
    let events =
        parse_events(&event_body(&format!("UID:cancelled@test\n{BASE}\nSUMMARY:Cancelled Meeting\nSTATUS:CANCELLED")));
    assert!(events.is_empty());
}

#[test]
fn skip_events_without_summary() {
    let events = parse_events(&event_body(&format!("UID:nosummary@test\n{BASE}")));
    assert!(events.is_empty());
}

#[test]
fn distinct_dtend_timezone_populates_end_timezone() {
    let events = parse_events(&event_body(
        "UID:flight@test\nDTSTAMP:20260315T000000Z\nDTSTART;TZID=America/New_York:20260315T080000\nDTEND;TZID=America/Los_Angeles:20260315T110000\nSUMMARY:JFK to LAX",
    ));
    assert_eq!(events[0].timezone.as_deref(), Some("America/New_York"));
    assert_eq!(events[0].end_timezone.as_deref(), Some("America/Los_Angeles"));
}

#[test]
fn same_dtend_timezone_yields_no_end_timezone() {
    let events = parse_events(&event_body(
        "UID:single-tz@test\nDTSTAMP:20260315T000000Z\nDTSTART;TZID=America/New_York:20260315T080000\nDTEND;TZID=America/New_York:20260315T090000\nSUMMARY:NY meeting",
    ));
    assert_eq!(events[0].timezone.as_deref(), Some("America/New_York"));
    assert!(events[0].end_timezone.is_none());
}

#[test]
fn parse_extracts_rdate_and_exdate_values() {
    let events = parse_events(&event_body(
        "UID:rdates@test\nDTSTAMP:20260101T000000Z\nDTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nRRULE:FREQ=WEEKLY\nRDATE:20260214T100000Z,20260314T100000Z\nEXDATE:20260212T100000Z\nSUMMARY:Recurring with extras",
    ));
    let e = &events[0];
    assert_eq!(e.rdates, vec!["2026-02-14T10:00:00Z", "2026-03-14T10:00:00Z"]);
    assert_eq!(e.exdates, vec!["2026-02-12T10:00:00Z"]);
}

#[test]
fn parse_extracts_single_valarm_with_duration_trigger() {
    let events = parse_events(&event_body(
        "UID:alarm-1@test\nDTSTAMP:20260101T000000Z\nDTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nSUMMARY:With alarm\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder\nEND:VALARM",
    ));
    let alarms = &events[0].alarms;
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].trigger, "-PT15M");
    assert_eq!(alarms[0].action, "DISPLAY");
    assert_eq!(alarms[0].description.as_deref(), Some("Reminder"));
}

#[test]
fn parse_extracts_valarm_with_absolute_trigger() {
    let events = parse_events(&event_body(
        "UID:alarm-abs@test\nDTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nSUMMARY:Absolute trigger\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;VALUE=DATE-TIME:20260115T093000Z\nDESCRIPTION:Reminder\nEND:VALARM",
    ));
    assert_eq!(events[0].alarms[0].trigger, "20260115T093000Z");
}

#[test]
fn parse_extracts_two_valarms_in_order() {
    let events = parse_events(&event_body(
        "UID:alarms-2@test\nDTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nSUMMARY:Two alarms\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Primary\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nDESCRIPTION:Day before\nEND:VALARM",
    ));
    let triggers: Vec<_> = events[0].alarms.iter().map(|a| a.trigger.as_str()).collect();
    assert_eq!(triggers, vec!["-PT15M", "-P1D"]);
}
