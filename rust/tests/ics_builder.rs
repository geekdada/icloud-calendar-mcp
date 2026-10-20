//! Port of the Kotlin `IcsBuilderTest`: building new event bodies.

use chrono::{DateTime, Utc};
use icloud_calendar_mcp::ics::{AlarmSpec, NewEvent, build_event, parse_events};
use regex::Regex;

fn timed(uid: &str, summary: &str, start: &str, end: &str) -> NewEvent {
    NewEvent {
        uid: Some(uid.into()),
        summary: summary.into(),
        start_time: Some(start.into()),
        end_time: Some(end.into()),
        ..NewEvent::default()
    }
}

fn basic(uid: &str, summary: &str) -> NewEvent {
    timed(uid, summary, "2025-01-15T10:00:00Z", "2025-01-15T11:00:00Z")
}

fn all_day(uid: &str, summary: &str, start: &str, end: &str) -> NewEvent {
    NewEvent {
        uid: Some(uid.into()),
        summary: summary.into(),
        start_date: Some(start.into()),
        end_date: Some(end.into()),
        is_all_day: true,
        ..NewEvent::default()
    }
}

fn build(event: NewEvent) -> String {
    build_event(&event).expect("valid event builds")
}

fn instant(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// The text of the first VALARM block.
fn valarm_block(ics: &str) -> &str {
    let start = ics.find("BEGIN:VALARM").expect("VALARM present");
    let end = ics[start..].find("END:VALARM").unwrap() + start;
    &ics[start..end]
}

fn physical_lines(ics: &str) -> impl Iterator<Item = &str> {
    ics.split("\r\n").flat_map(|l| l.split('\n'))
}

fn lines_starting<'a>(ics: &'a str, prefix: &'a str) -> Vec<&'a str> {
    physical_lines(ics).filter(|l| l.starts_with(prefix)).collect()
}

#[test]
fn build_simple_timed_event() {
    let ics = build(basic("test-123@example.com", "Team Meeting"));
    for needle in [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:",
        "BEGIN:VEVENT",
        "UID:test-123@example.com",
        "SUMMARY:Team Meeting",
        "DTSTART:20250115T100000Z",
        "DTEND:20250115T110000Z",
        "END:VEVENT",
        "END:VCALENDAR",
    ] {
        assert!(ics.contains(needle), "missing {needle} in:\n{ics}");
    }
}

#[test]
fn build_generates_dtstamp() {
    let ics = build(basic("test-123@example.com", "Test Event"));
    assert!(Regex::new(r"DTSTAMP:\d{8}T\d{6}Z").unwrap().is_match(&ics));
}

#[test]
fn build_all_day_single_day_event() {
    let ics = build(all_day("all-day-123@example.com", "Holiday", "2025-01-15", "2025-01-15"));
    assert!(ics.contains("DTSTART;VALUE=DATE:20250115"));
    assert!(ics.contains("DTEND;VALUE=DATE:20250116"));
    assert!(!ics.contains("DTSTART:"));
}

#[test]
fn build_all_day_multi_day_event() {
    let ics = build(all_day("multi-day-123@example.com", "Conference", "2025-01-15", "2025-01-17"));
    assert!(ics.contains("DTSTART;VALUE=DATE:20250115"));
    assert!(ics.contains("DTEND;VALUE=DATE:20250118"));
}

#[test]
fn build_event_with_description_and_location() {
    let ics = build(NewEvent {
        description: Some("Event with all fields".into()),
        location: Some("Room 101".into()),
        ..timed("complete-event@example.com", "Full Event", "2025-01-15T14:00:00Z", "2025-01-15T15:30:00Z")
    });
    assert!(ics.contains("SUMMARY:Full Event"));
    assert!(ics.contains("DESCRIPTION:Event with all fields"));
    assert!(ics.contains("LOCATION:Room 101"));
}

#[test]
fn build_event_with_timezone() {
    let ics = build(NewEvent {
        timezone: Some("America/New_York".into()),
        ..timed("tz-event@example.com", "Local Meeting", "2025-01-15T10:00:00", "2025-01-15T11:00:00")
    });
    assert!(ics.contains("DTSTART;TZID=America/New_York:20250115T100000"));
    assert!(ics.contains("DTEND;TZID=America/New_York:20250115T110000"));
}

#[test]
fn build_utc_event_ignores_timezone_parameter() {
    let ics = build(NewEvent { timezone: Some("America/New_York".into()), ..basic("utc-event@example.com", "UTC") });
    assert!(ics.contains("DTSTART:20250115T100000Z"));
    assert!(ics.contains("DTEND:20250115T110000Z"));
    assert!(!ics.contains("TZID="));
}

#[test]
fn build_offset_event_stores_correct_utc_instant() {
    let ics =
        build(timed("offset-event@example.com", "Offset", "2026-08-06T18:30:00+09:00", "2026-08-06T19:30:00+09:00"));
    assert!(ics.contains("DTSTART:20260806T093000Z"), "{ics}");
    assert!(ics.contains("DTEND:20260806T103000Z"), "{ics}");
    assert!(!ics.contains("TZID="));
}

#[test]
fn build_anchors_naive_end_to_end_timezone_without_timezone() {
    let ics = build(NewEvent {
        end_timezone: Some("Asia/Tokyo".into()),
        ..timed("end-tz@example.com", "Split Zone", "2025-01-15T00:00:00", "2025-01-15T18:00:00")
    });
    assert!(ics.contains("DTSTART:20250115T000000Z"), "{ics}");
    assert!(ics.contains("DTEND;TZID=Asia/Tokyo:20250115T180000"), "{ics}");
}

#[test]
fn build_offset_event_ignores_timezone_parameter() {
    let ics = build(NewEvent {
        timezone: Some("America/New_York".into()),
        ..timed("offset-wins@example.com", "Offset Wins", "2026-08-06T18:30:00+09:00", "2026-08-06T19:30:00+09:00")
    });
    assert!(ics.contains("DTSTART:20260806T093000Z"), "{ics}");
    assert!(ics.contains("DTEND:20260806T103000Z"), "{ics}");
    assert!(!ics.contains("TZID="));
}

#[test]
fn escape_commas_semicolons_newlines_and_backslashes() {
    let commas = build(basic("e1@example.com", "Meeting with Alice, Bob, Charlie"));
    assert!(commas.contains("SUMMARY:Meeting with Alice\\, Bob\\, Charlie"));
    let semis = build(basic("e2@example.com", "Test; Important"));
    assert!(semis.contains("SUMMARY:Test\\; Important"));
    let newlines =
        build(NewEvent { description: Some("Line 1\nLine 2\nLine 3".into()), ..basic("e3@example.com", "Notes") });
    assert!(newlines.contains("DESCRIPTION:Line 1\\nLine 2\\nLine 3"));
    let backslash = build(basic("e4@example.com", "Path: C:\\Users\\test"));
    assert!(backslash.contains("SUMMARY:Path: C:\\\\Users\\\\test"));
}

#[test]
fn preserve_unicode_characters() {
    let ics = build(NewEvent {
        description: Some("Café résumé naïve".into()),
        ..basic("unicode-test@example.com", "会議")
    });
    assert!(ics.contains("SUMMARY:会議"));
    assert!(ics.contains("DESCRIPTION:Café résumé naïve"));
}

#[test]
fn fold_long_lines_at_75_octets() {
    let ics = build(NewEvent { description: Some("A".repeat(100)), ..basic("fold-test@example.com", "Test") });
    for line in physical_lines(&ics) {
        assert!(line.len() <= 75, "line too long: {} chars", line.len());
    }
}

#[test]
fn continuation_lines_start_with_space() {
    let ics = build(basic("continuation-test@example.com", &"B".repeat(100)));
    let lines: Vec<&str> = ics.split("\r\n").collect();
    let summary_at = lines.iter().position(|l| l.starts_with("SUMMARY:")).unwrap();
    assert!(lines[summary_at + 1].starts_with(' '), "continuation must start with a space");
}

#[test]
fn build_recurring_event_with_rrule() {
    let ics = build(NewEvent {
        rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE,FR".into()),
        ..timed("recurring-123@example.com", "Weekly Standup", "2025-01-15T09:00:00Z", "2025-01-15T09:30:00Z")
    });
    assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR"));
}

#[test]
fn build_recurring_all_day_event() {
    let ics = build(NewEvent {
        rrule: Some("FREQ=MONTHLY;BYMONTHDAY=1".into()),
        ..all_day("recurring-allday@example.com", "Monthly Review", "2025-01-01", "2025-01-01")
    });
    assert!(ics.contains("DTSTART;VALUE=DATE:20250101"));
    assert!(ics.contains("RRULE:FREQ=MONTHLY;BYMONTHDAY=1"));
}

#[test]
fn generate_uid_when_not_provided() {
    let ics = build(NewEvent { uid: None, ..basic("ignored", "Auto UID Event") });
    assert!(Regex::new(r"UID:[a-f0-9-]+@icloud-calendar-mcp").unwrap().is_match(&ics), "{ics}");
}

#[test]
fn emoji_and_cjk_fold_at_octet_boundaries() {
    for summary in ["🎉".repeat(18), "会".repeat(30), "A".repeat(100)] {
        let ics = build(basic("fold@test", &summary));
        for line in physical_lines(&ics) {
            assert!(line.len() <= 75, "line has {} octets", line.len());
        }
        assert_eq!(parse_events(&ics)[0].summary, summary, "folding must not split characters");
    }
}

#[test]
fn build_event_with_extended_fields() {
    let ics = build(NewEvent {
        status: Some("TENTATIVE".into()),
        url: Some("https://meet.example.com/room".into()),
        categories: Some(vec!["Work".into(), "Meeting".into()]),
        priority: Some(1),
        transp: Some("TRANSPARENT".into()),
        ..basic("ext@test", "Extended")
    });
    assert!(ics.contains("STATUS:TENTATIVE"));
    assert!(ics.contains("URL:https://meet.example.com/room"));
    assert!(ics.contains("CATEGORIES:Work,Meeting"));
    assert!(ics.contains("PRIORITY:1"));
    assert!(ics.contains("TRANSP:TRANSPARENT"));
}

#[test]
fn build_event_with_categories_containing_semicolons() {
    let ics =
        build(NewEvent { categories: Some(vec!["Work;Important".into(), "Meeting".into()]), ..basic("c@t", "Esc") });
    assert!(ics.contains("CATEGORIES:Work\\;Important,Meeting"));
}

#[test]
fn null_optional_fields_produce_no_lines() {
    let ics = build(basic("minimal@test", "Minimal"));
    assert!(ics.contains("STATUS:CONFIRMED"));
    assert!(ics.contains("SEQUENCE:"));
    for absent in ["URL:", "CATEGORIES:", "PRIORITY:", "TRANSP:"] {
        assert!(!ics.contains(absent), "unexpected {absent}");
    }
}

#[test]
fn build_event_with_timezone_includes_vtimezone_before_vevent() {
    let ics = build(NewEvent {
        timezone: Some("America/New_York".into()),
        ..timed("vtz@test", "NYC Meeting", "2025-01-15T10:00:00", "2025-01-15T11:00:00")
    });
    let vtz = ics.find("BEGIN:VTIMEZONE").expect("VTIMEZONE present");
    assert!(ics.contains("TZID:America/New_York"));
    assert!(vtz < ics.find("BEGIN:VEVENT").unwrap());
}

#[test]
fn utc_times_and_no_timezone_param_have_no_vtimezone() {
    let with_param = build(NewEvent { timezone: Some("America/New_York".into()), ..basic("u@t", "UTC Meeting") });
    assert!(!with_param.contains("BEGIN:VTIMEZONE"));
    let without = build(basic("n@t", "No TZ Meeting"));
    assert!(!without.contains("BEGIN:VTIMEZONE"));
}

#[test]
fn blank_description_and_location_are_omitted() {
    let ics =
        build(NewEvent { description: Some(String::new()), location: Some("   ".into()), ..basic("b@t", "Test") });
    assert!(!ics.contains("DESCRIPTION:"));
    assert!(!ics.contains("LOCATION:"));
}

#[test]
fn round_trip_parsing_produces_same_values() {
    let ics = build(NewEvent {
        description: Some("Testing the full cycle".into()),
        location: Some("Test Room".into()),
        ..timed("roundtrip-test@example.com", "Round Trip Test", "2025-01-15T14:00:00Z", "2025-01-15T15:30:00Z")
    });
    let events = parse_events(&ics);
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.uid, "roundtrip-test@example.com");
    assert_eq!(e.summary, "Round Trip Test");
    assert_eq!(e.description.as_deref(), Some("Testing the full cycle"));
    assert_eq!(e.location.as_deref(), Some("Test Room"));
    assert_eq!(e.start_time.as_deref(), Some("2025-01-15T14:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-01-15T15:30:00Z"));
    assert!(!e.is_all_day);
}

#[test]
fn round_trip_all_day_event() {
    let events = parse_events(&build(all_day("allday-roundtrip@example.com", "All Day", "2025-01-15", "2025-01-17")));
    assert!(events[0].is_all_day);
    assert_eq!(events[0].start_date.as_deref(), Some("2025-01-15"));
    assert_eq!(events[0].end_date.as_deref(), Some("2025-01-17"));
}

#[test]
fn created_and_last_modified_emitted_only_when_provided() {
    let ics = build(NewEvent {
        created_at: Some(instant("2026-01-10T08:30:00Z")),
        last_modified: Some(instant("2026-01-12T14:45:00Z")),
        ..basic("with-created@example.com", "Has CREATED")
    });
    assert!(ics.contains("CREATED:20260110T083000Z"), "{ics}");
    assert!(ics.contains("LAST-MODIFIED:20260112T144500Z"), "{ics}");
    let plain = build(basic("no-created@example.com", "No CREATED"));
    assert!(lines_starting(&plain, "CREATED:").is_empty());
    assert!(lines_starting(&plain, "LAST-MODIFIED:").is_empty());
}

#[test]
fn timed_recurring_event_emits_duration_not_dtend() {
    let ics = build(NewEvent {
        rrule: Some("FREQ=WEEKLY;BYDAY=TH".into()),
        ..timed("weekly-1h@example.com", "Weekly", "2026-01-15T10:00:00Z", "2026-01-15T11:00:00Z")
    });
    assert!(ics.contains("DURATION:PT1H"), "{ics}");
    assert!(lines_starting(&ics, "DTEND").is_empty(), "{ics}");
}

#[test]
fn timed_non_recurring_event_still_emits_dtend() {
    let ics = build(timed("one-shot@example.com", "One-shot", "2026-01-15T10:00:00Z", "2026-01-15T11:30:00Z"));
    assert!(ics.contains("DTEND:20260115T113000Z"));
    assert!(lines_starting(&ics, "DURATION").is_empty());
}

#[test]
fn all_day_recurring_event_emits_duration_in_days() {
    let one =
        build(NewEvent { rrule: Some("FREQ=DAILY".into()), ..all_day("d@t", "Daily", "2026-01-15", "2026-01-15") });
    assert!(one.contains("DURATION:P1D"), "{one}");
    assert!(lines_starting(&one, "DTEND").is_empty());
    let three =
        build(NewEvent { rrule: Some("FREQ=YEARLY".into()), ..all_day("w@t", "Retreat", "2026-01-16", "2026-01-18") });
    assert!(three.contains("DURATION:P3D"), "{three}");
}

#[test]
fn distinct_end_timezone_emits_separate_tzids_and_two_vtimezones() {
    let ics = build(NewEvent {
        timezone: Some("America/New_York".into()),
        end_timezone: Some("America/Los_Angeles".into()),
        ..timed("flight@example.com", "JFK to LAX", "2026-03-15T08:00:00", "2026-03-15T11:00:00")
    });
    assert!(ics.contains("DTSTART;TZID=America/New_York:20260315T080000"), "{ics}");
    assert!(ics.contains("DTEND;TZID=America/Los_Angeles:20260315T110000"), "{ics}");
    assert_eq!(ics.matches("BEGIN:VTIMEZONE").count(), 2);
}

#[test]
fn missing_or_equal_end_timezone_reuses_start_zone_once() {
    for end_timezone in [None, Some("America/New_York".to_string())] {
        let ics = build(NewEvent {
            timezone: Some("America/New_York".into()),
            end_timezone,
            ..timed("same-tz@example.com", "NY meeting", "2026-03-15T10:00:00", "2026-03-15T11:00:00")
        });
        assert!(ics.contains("DTSTART;TZID=America/New_York:20260315T100000"));
        assert!(ics.contains("DTEND;TZID=America/New_York:20260315T110000"));
        assert_eq!(ics.matches("BEGIN:VTIMEZONE").count(), 1);
    }
}

#[test]
fn one_rdate_and_exdate_line_per_value() {
    let ics = build(NewEvent {
        rrule: Some("FREQ=WEEKLY".into()),
        rdates: Some(vec!["2026-02-14T10:00:00Z".into(), "2026-03-14T10:00:00Z".into()]),
        exdates: Some(vec!["2026-02-12T10:00:00Z".into(), "2026-02-19T10:00:00Z".into()]),
        ..timed("rdate-multi@example.com", "Recurring", "2026-01-15T10:00:00Z", "2026-01-15T11:00:00Z")
    });
    let rdates = lines_starting(&ics, "RDATE");
    assert_eq!(rdates.len(), 2, "{ics}");
    assert!(rdates.iter().any(|l| l.contains("20260214T100000Z")));
    assert!(rdates.iter().any(|l| l.contains("20260314T100000Z")));
    assert_eq!(lines_starting(&ics, "EXDATE").len(), 2);
}

#[test]
fn empty_rdates_and_exdates_emit_nothing() {
    let ics = build(NewEvent {
        rrule: Some("FREQ=WEEKLY".into()),
        rdates: Some(vec![]),
        exdates: Some(vec![]),
        ..basic("no-extras@example.com", "Plain recurring")
    });
    assert!(lines_starting(&ics, "RDATE").is_empty());
    assert!(lines_starting(&ics, "EXDATE").is_empty());
}

fn with_alarms(alarms: Option<Vec<AlarmSpec>>) -> String {
    build(NewEvent { alarms, ..timed("alarm@example.com", "Alarm", "2026-01-15T10:00:00Z", "2026-01-15T11:00:00Z") })
}

fn alarm(trigger: &str) -> AlarmSpec {
    AlarmSpec { trigger: trigger.into(), ..AlarmSpec::default() }
}

#[test]
fn duration_trigger_display_alarm_emits_valarm_block() {
    let ics = with_alarms(Some(vec![alarm("-PT15M")]));
    for needle in ["BEGIN:VALARM", "ACTION:DISPLAY", "TRIGGER:-PT15M", "DESCRIPTION:Reminder", "END:VALARM"] {
        assert!(ics.contains(needle), "missing {needle}:\n{ics}");
    }
}

#[test]
fn absolute_trigger_uses_value_date_time() {
    let ics = with_alarms(Some(vec![alarm("20260115T093000Z")]));
    assert!(ics.contains("TRIGGER;VALUE=DATE-TIME:20260115T093000Z"), "{ics}");
}

#[test]
fn audio_action_omits_description_and_summary() {
    let ics = with_alarms(Some(vec![AlarmSpec { action: Some("AUDIO".into()), ..alarm("-PT5M") }]));
    assert!(ics.contains("ACTION:AUDIO"));
    assert!(ics.contains("TRIGGER:-PT5M"));
    let block = valarm_block(&ics);
    assert!(!block.contains("DESCRIPTION:"), "{ics}");
    assert!(!block.contains("SUMMARY:"), "{ics}");
}

#[test]
fn email_action_emits_summary_and_description() {
    let ics = with_alarms(Some(vec![AlarmSpec {
        action: Some("EMAIL".into()),
        summary: Some("Meeting reminder".into()),
        description: Some("Your meeting starts soon".into()),
        ..alarm("-PT1H")
    }]));
    assert!(ics.contains("ACTION:EMAIL"));
    assert!(ics.contains("SUMMARY:Meeting reminder"));
    assert!(ics.contains("DESCRIPTION:Your meeting starts soon"));
}

#[test]
fn repeat_and_duration_emitted_only_as_a_pair() {
    let paired = with_alarms(Some(vec![AlarmSpec {
        repeat_count: Some(3),
        repeat_duration: Some("PT5M".into()),
        ..alarm("-PT15M")
    }]));
    let block = valarm_block(&paired);
    assert!(block.contains("REPEAT:3"));
    assert!(block.contains("DURATION:PT5M"));
    let unpaired = with_alarms(Some(vec![AlarmSpec { repeat_count: Some(3), ..alarm("-PT15M") }]));
    let block = valarm_block(&unpaired);
    assert!(!block.contains("REPEAT:"));
    assert!(!block.contains("DURATION:"));
}

#[test]
fn two_alarms_emit_two_valarm_blocks() {
    let ics = with_alarms(Some(vec![alarm("-PT15M"), alarm("-P1D")]));
    assert_eq!(ics.matches("BEGIN:VALARM").count(), 2);
    assert!(ics.contains("TRIGGER:-PT15M"));
    assert!(ics.contains("TRIGGER:-P1D"));
}

#[test]
fn null_or_empty_alarms_emit_no_valarm() {
    assert!(!with_alarms(None).contains("BEGIN:VALARM"));
    assert!(!with_alarms(Some(vec![])).contains("BEGIN:VALARM"));
}

#[test]
fn naive_times_without_seconds_are_accepted() {
    // Deliberate deviation: the Kotlin builder threw for a zoned value without
    // seconds (`2025-01-15T09:00`); the validator admits that shape, so the
    // Rust builder accepts it on every path.
    let ics = build(NewEvent {
        timezone: Some("Europe/Paris".into()),
        ..timed("noseconds@t", "No seconds", "2025-01-15T09:00", "2025-01-15T10:00")
    });
    assert!(ics.contains("DTSTART;TZID=Europe/Paris:20250115T090000"), "{ics}");
}
