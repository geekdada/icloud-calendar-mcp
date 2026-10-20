//! Port of the Kotlin `Issue2ReproTest`: a description-only update must never
//! corrupt SUMMARY, even when the old DESCRIPTION was line-folded.

use icloud_calendar_mcp::ics::parse_events;
use icloud_calendar_mcp::ics::patcher::{EventPatch, patch};

const ORIGINAL_SUMMARY: &str = "MCP TEST long-description event";
const ORIGINAL_DESCRIPTION: &str = "Send a short follow-up email with an attachment for a warm introduction. Frame around the pitch, not generic availability.";
const NEW_DESCRIPTION: &str = "MCP TEST touch simple.";

/// What iCloud might return: a short SUMMARY and a DESCRIPTION folded at 75 octets.
fn original_with_folded_description() -> String {
    let raw = format!("DESCRIPTION:{ORIGINAL_DESCRIPTION}");
    let folded = format!("{}\r\n {}", &raw[..75], &raw[75..]);
    let summary = format!("SUMMARY:{ORIGINAL_SUMMARY}");
    let lines = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Test//EN",
        "BEGIN:VEVENT",
        "UID:repro-issue2@test",
        "DTSTAMP:20260115T100000Z",
        "DTSTART:20260120T140000Z",
        "DTEND:20260120T150000Z",
        &summary,
        &folded,
        "END:VEVENT",
        "END:VCALENDAR",
    ];
    lines.join("\r\n") + "\r\n"
}

fn description_only(existing: &str, uid: &str) -> String {
    patch(Some(existing), uid, &EventPatch { description: Some(NEW_DESCRIPTION.into()), ..EventPatch::default() })
        .expect("patch succeeds")
}

#[test]
fn parser_handles_folded_description() {
    let events = parse_events(&original_with_folded_description());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, ORIGINAL_SUMMARY);
    assert_eq!(events[0].description.as_deref(), Some(ORIGINAL_DESCRIPTION));
}

#[test]
fn description_only_patch_leaves_summary_exactly_as_it_was() {
    let out = description_only(&original_with_folded_description(), "repro-issue2@test");
    let events = parse_events(&out);
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.summary, ORIGINAL_SUMMARY, "SUMMARY corruption: {out}");
    assert_eq!(e.description.as_deref(), Some(NEW_DESCRIPTION));
    let lower = e.summary.to_lowercase();
    assert!(!lower.contains("warm") && !lower.contains("frame"));
}

#[test]
fn description_only_patch_survives_second_parse() {
    let out = description_only(&original_with_folded_description(), "repro-issue2@test");
    assert!(!parse_events(&out).is_empty());
    let again = parse_events(&out);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].summary, ORIGINAL_SUMMARY);
}

#[test]
fn description_only_patch_on_lf_only_original_preserves_summary() {
    let lf_only = original_with_folded_description().replace("\r\n", "\n");
    let events = parse_events(&description_only(&lf_only, "repro-issue2@test"));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, ORIGINAL_SUMMARY);
    assert_eq!(events[0].description.as_deref(), Some(NEW_DESCRIPTION));
}

#[test]
fn description_with_escaped_commas_and_semicolons_round_trips_clean() {
    let tricky = "Send a follow-up email; warm introduction, framed around the pitch, not generic availability.";
    let summary = format!("SUMMARY:{ORIGINAL_SUMMARY}");
    let ics = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Test//EN",
        "BEGIN:VEVENT",
        "UID:repro-issue2-escape@test",
        "DTSTAMP:20260115T100000Z",
        "DTSTART:20260120T140000Z",
        "DTEND:20260120T150000Z",
        &summary,
        "DESCRIPTION:Send a follow-up email\\; warm introduction\\, framed around the pitch\\, not generic availability.",
        "END:VEVENT",
        "END:VCALENDAR",
    ]
    .join("\r\n")
        + "\r\n";
    assert_eq!(parse_events(&ics)[0].description.as_deref(), Some(tricky));
    let events = parse_events(&description_only(&ics, "repro-issue2-escape@test"));
    assert_eq!(events[0].summary, ORIGINAL_SUMMARY);
    assert_eq!(events[0].description.as_deref(), Some(NEW_DESCRIPTION));
}
