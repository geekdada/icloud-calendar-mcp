//! Parser/builder cases of the Kotlin `ReDoSSecurityTest` and `UnicodeSecurityTest`:
//! hostile sizes stay fast, and Unicode survives the round trip.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use icloud_calendar_mcp::ics::{NewEvent, build_event, parse_events, parse_occurrences};

fn assert_within(started: Instant, limit: Duration) {
    assert!(started.elapsed() < limit, "took {:?}", started.elapsed());
}

fn all_day(summary: &str) -> NewEvent {
    NewEvent { summary: summary.into(), start_date: Some("2025-01-15".into()), is_all_day: true, ..NewEvent::default() }
}

fn single_event(lines: &str) -> String {
    format!("BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\n{lines}END:VEVENT\nEND:VCALENDAR\n")
}

// ───────────────────────────── parser resource exhaustion ─────────────────────────────

#[test]
fn huge_description_parses_quickly() {
    let ics = single_event(&format!(
        "UID:bomb@test.com\nSUMMARY:Bomb Test\nDESCRIPTION:{}\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\n",
        "LOL".repeat(100_000)
    ));
    let started = Instant::now();
    let events = parse_events(&ics);
    assert_eq!(events[0].description.as_deref().map(str::len), Some(300_000));
    assert_within(started, Duration::from_secs(5));
}

#[test]
fn heavy_line_folding_is_linear() {
    let mut ics = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:fold@test.com\r\nSUMMARY:Folded Test\r\nDESCRIPTION:",
    );
    for i in 0..10_000 {
        ics.push_str(&format!("Word{i}"));
        if i % 10 == 0 {
            ics.push_str("\r\n ");
        }
    }
    ics.push_str("\r\nDTSTART:20250115T100000Z\r\nDTEND:20250115T110000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
    let started = Instant::now();
    let events = parse_events(&ics);
    assert!(events[0].description.as_deref().unwrap().contains("Word9999"));
    assert_within(started, Duration::from_secs(5));
}

#[test]
fn many_small_events_parse() {
    let mut ics = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
    for i in 0..10_000 {
        ics.push_str(&format!(
            "BEGIN:VEVENT\nUID:small-{i}@test.com\nSUMMARY:Event {i}\nDTSTART:20250115T100000Z\nDTEND:20250115T100100Z\nEND:VEVENT\n"
        ));
    }
    ics.push_str("END:VCALENDAR\n");
    let started = Instant::now();
    assert_eq!(parse_events(&ics).len(), 10_000);
    assert_within(started, Duration::from_secs(5));
}

#[test]
fn deeply_nested_components_do_not_crash() {
    let mut ics = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
    ics.push_str(&"BEGIN:VEVENT\n".repeat(100));
    ics.push_str("UID:deep@test.com\nSUMMARY:Deep Nesting\nDTSTART:20250115T100000Z\n");
    ics.push_str(&"END:VEVENT\n".repeat(100));
    ics.push_str("END:VCALENDAR\n");
    let started = Instant::now();
    // Only top-level VEVENTs are events; the outermost has no start, so none map.
    assert!(parse_events(&ics).is_empty());
    assert_within(started, Duration::from_secs(5));
}

#[test]
fn complex_rrule_parses_and_expands_within_bounds() {
    let by_day: Vec<String> = (1..=52).map(|n| format!("{n}MO,{n}TU,{n}WE,{n}TH,{n}FR")).collect();
    let ics = single_event(&format!(
        "UID:rrule@test.com\nSUMMARY:Complex Recurrence\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nRRULE:FREQ=YEARLY;BYDAY={}\n",
        by_day.join(",")
    ));
    let started = Instant::now();
    assert_eq!(parse_events(&ics).len(), 1);
    let from = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339("2025-12-31T23:59:59Z").unwrap().with_timezone(&Utc);
    let occurrences = parse_occurrences(&ics, from, to).expect("well under the expansion bound");
    assert!(!occurrences.is_empty());
    assert_within(started, Duration::from_secs(5));
}

// ───────────────────────────── builder resource exhaustion ─────────────────────────────

#[test]
fn builder_handles_massive_summary_and_description() {
    let started = Instant::now();
    let summary: String = format!("Meeting {}", "A".repeat(100_000)).chars().take(500).collect();
    assert!(build_event(&all_day(&summary)).unwrap().contains("SUMMARY:"));
    let description: String = format!("Notes: {}", "B".repeat(100_000)).chars().take(5000).collect();
    let ics = build_event(&NewEvent { description: Some(description), ..all_day("Test") }).unwrap();
    assert!(ics.contains("DESCRIPTION:Notes: "));
    assert_within(started, Duration::from_secs(5));
}

#[test]
fn builder_escaping_stress() {
    let started = Instant::now();
    let heavy = ";,\\;,\\;,\\".repeat(10_000);
    let ics = build_event(&NewEvent { description: Some(heavy.clone()), ..all_day("Test") }).unwrap();
    assert!(ics.contains("\\;"));
    assert_eq!(parse_events(&ics)[0].description.as_deref(), Some(heavy.as_str()), "escaping round-trips exactly");
    assert_within(started, Duration::from_secs(5));
}

#[test]
fn parser_and_builder_do_not_accumulate_state() {
    let started = Instant::now();
    for i in 0..10 {
        let ics = single_event(&format!(
            "UID:large-{i}@test.com\nSUMMARY:Large Event {i}\nDESCRIPTION:{}\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\n",
            "X".repeat(100_000)
        ));
        assert_eq!(parse_events(&ics).len(), 1);
    }
    for i in 0..1000 {
        assert!(build_event(&all_day(&format!("Event {i} with {}", "description ".repeat(100)))).is_ok());
    }
    assert_within(started, Duration::from_secs(5));
}

// ───────────────────────────── unicode ─────────────────────────────

#[test]
fn unicode_title_builds_and_round_trips() {
    let title = "会議 - Meeting 📅";
    let ics = build_event(&all_day(title)).unwrap();
    assert!(ics.contains("SUMMARY:会議 - Meeting 📅"));
    assert_eq!(parse_events(&ics)[0].summary, title);
}

#[test]
fn parser_reads_unicode_content() {
    let events = parse_events(&single_event(
        "UID:unicode@test.com\nSUMMARY:日本語のイベント\nDESCRIPTION:这是中文描述\nLOCATION:東京\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\n",
    ));
    assert_eq!(events[0].summary, "日本語のイベント");
    assert_eq!(events[0].description.as_deref(), Some("这是中文描述"));
    assert_eq!(events[0].location.as_deref(), Some("東京"));
}

#[test]
fn parser_reads_emoji_in_various_fields() {
    let events = parse_events(&single_event(
        "UID:emoji@test.com\nSUMMARY:🎉 Party Time! 🎊\nDESCRIPTION:Let's celebrate! 🥳🎈🎁\nLOCATION:🏠 My Place\nDTSTART:20250115T180000Z\nDTEND:20250115T230000Z\n",
    ));
    assert_eq!(events[0].summary, "🎉 Party Time! 🎊");
    assert_eq!(events[0].location.as_deref(), Some("🏠 My Place"));
}

#[test]
fn combining_diacritical_marks_are_preserved_byte_for_byte() {
    let combining = "Cafe\u{301} Meeting";
    let events = parse_events(&single_event(&format!(
        "UID:combining@test.com\nSUMMARY:{combining}\nLOCATION:Café René\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\n"
    )));
    assert_eq!(events[0].summary, combining, "no normalization is applied");
    let rebuilt = build_event(&all_day(combining)).unwrap();
    assert_eq!(parse_events(&rebuilt)[0].summary, combining);
}

#[test]
fn bidi_and_zero_width_characters_round_trip_as_data() {
    for text in ["\u{202E}gnp.exe\u{202C}", "pass\u{200B}word", "a\u{2066}b\u{2069}c", "soft\u{AD}hyphen"] {
        let ics = build_event(&all_day(text)).unwrap();
        assert_eq!(parse_events(&ics)[0].summary, text);
    }
}
