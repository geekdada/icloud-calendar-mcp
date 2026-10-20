//! Port of the Kotlin `IcsPatcherTest`: patching existing bodies (whole event,
//! one occurrence, this-and-future) while preserving what is not changed.

use icloud_calendar_mcp::ical::parser::parse_all_events;
use icloud_calendar_mcp::ics::patcher::{
    EventPatch, PatchError, exdate_occurrence, patch, patch_occurrence, split_series, truncate_series,
};
use icloud_calendar_mcp::ics::{AlarmSpec, ParsedEvent, parse_events};

fn ics(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// A calendar with one VEVENT made of `uid` plus the given property lines.
fn event(uid: &str, props: &str) -> String {
    ics(&format!(
        "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\nBEGIN:VEVENT\nUID:{uid}\nDTSTAMP:20251220T100000Z\n{props}\nEND:VEVENT\nEND:VCALENDAR"
    ))
}

const TIMED: &str = "DTSTART:20251225T100000Z\nDTEND:20251225T110000Z";

fn summary(s: &str) -> EventPatch {
    EventPatch { summary: Some(s.into()), ..EventPatch::default() }
}

fn patched(existing: &str, uid: &str, fields: EventPatch) -> String {
    patch(Some(existing), uid, &fields).expect("patch succeeds")
}

fn parsed_one(ics: &str) -> ParsedEvent {
    let mut events = parse_events(ics);
    assert_eq!(events.len(), 1, "expected one event in:\n{ics}");
    events.remove(0)
}

// ───────────────────────────── preservation ─────────────────────────────

#[test]
fn patch_preserves_valarm_blocks() {
    let original = event(
        "alarm-test@test",
        &format!(
            "{TIMED}\nSUMMARY:Original Title\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:15 minutes before\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT1H\nDESCRIPTION:1 hour before\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nDESCRIPTION:1 day before\nEND:VALARM"
        ),
    );
    let out = patched(&original, "alarm-test@test", summary("Updated Title"));
    assert_eq!(out.matches("BEGIN:VALARM").count(), 3);
    for trigger in ["TRIGGER:-PT15M", "TRIGGER:-PT1H", "TRIGGER:-P1D"] {
        assert!(out.contains(trigger), "{trigger} preserved");
    }
    assert!(out.contains("SUMMARY:Updated Title"));
}

#[test]
fn patch_preserves_attendees_and_organizer() {
    let original = event(
        "attendee-test@test",
        &format!(
            "{TIMED}\nSUMMARY:Meeting with Attendees\nORGANIZER;CN=John Doe:mailto:john@example.com\nATTENDEE;CN=Jane Smith;PARTSTAT=ACCEPTED:mailto:jane@example.com\nATTENDEE;CN=Bob Wilson;PARTSTAT=TENTATIVE:mailto:bob@example.com"
        ),
    );
    let out = patched(&original, "attendee-test@test", summary("Updated Meeting"));
    for email in ["john@example.com", "jane@example.com", "bob@example.com"] {
        assert!(out.contains(email), "{email} preserved");
    }
    let e = parsed_one(&out);
    assert_eq!(e.summary, "Updated Meeting");
    assert_eq!(e.attendee_count, 2);
    assert!(e.organizer.is_some());
}

#[test]
fn patch_preserves_x_apple_and_custom_properties() {
    let original = event(
        "xprop-test@test",
        &format!(
            "{TIMED}\nSUMMARY:Event with Apple Props\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\nX-APPLE-STRUCTURED-LOCATION;VALUE=URI:geo:37.33,-122.03\nX-CUSTOM-PROP:custom value"
        ),
    );
    let out = patched(&original, "xprop-test@test", summary("Updated Title"));
    assert!(out.contains("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC"));
    assert!(out.contains("X-APPLE-STRUCTURED-LOCATION;VALUE=URI:geo:37.33,-122.03"));
    assert!(out.contains("X-CUSTOM-PROP:custom value"));
}

#[test]
fn patch_increments_sequence() {
    let original = event("seq-test@test", &format!("{TIMED}\nSUMMARY:Original\nSEQUENCE:5"));
    assert!(patched(&original, "seq-test@test", summary("Updated")).contains("SEQUENCE:6"));
    let unsequenced = event("no-seq@test", &format!("{TIMED}\nSUMMARY:No Sequence"));
    assert!(patched(&unsequenced, "no-seq@test", summary("Updated")).contains("SEQUENCE:1"));
}

// ───────────────────────────── times ─────────────────────────────

fn times(start: &str, end: &str) -> EventPatch {
    EventPatch { start_time: Some(start.into()), end_time: Some(end.into()), ..EventPatch::default() }
}

#[test]
fn patch_updates_dtstart_and_dtend_for_timed_events() {
    let original = event("time-update@test", &format!("{TIMED}\nSUMMARY:Original Time"));
    let e = parsed_one(&patched(&original, "time-update@test", times("2025-12-26T14:00:00Z", "2025-12-26T15:00:00Z")));
    assert_eq!(e.start_time.as_deref(), Some("2025-12-26T14:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-12-26T15:00:00Z"));
    assert!(!e.is_all_day);
}

#[test]
fn patch_updates_times_with_z_and_offset_values() {
    let original = event("z-update@test", &format!("{TIMED}\nSUMMARY:Z Update"));
    let z = patched(&original, "z-update@test", times("2026-08-06T09:30:00Z", "2026-08-06T10:30:00Z"));
    assert!(z.contains("DTSTART:20260806T093000Z") && z.contains("DTEND:20260806T103000Z"), "{z}");
    assert!(!z.contains("TZID="));
    let offset = patched(&original, "z-update@test", times("2026-08-06T18:30:00+09:00", "2026-08-06T19:30:00+09:00"));
    assert!(offset.contains("DTSTART:20260806T093000Z") && offset.contains("DTEND:20260806T103000Z"), "{offset}");
    assert!(!offset.contains("TZID="));
}

#[test]
fn patch_offset_ignores_timezone_parameter() {
    let original = event("offset-wins-update@test", &format!("{TIMED}\nSUMMARY:Offset Wins Update"));
    let out = patched(
        &original,
        "offset-wins-update@test",
        EventPatch {
            timezone: Some("America/New_York".into()),
            ..times("2026-08-06T18:30:00+09:00", "2026-08-06T19:30:00+09:00")
        },
    );
    assert!(out.contains("DTSTART:20260806T093000Z") && out.contains("DTEND:20260806T103000Z"), "{out}");
    assert!(!out.contains("TZID="));
}

const WEEKLY: &str = "DTSTART:20251225T100000Z\nDTEND:20251225T110000Z\nSUMMARY:Weekly\nRRULE:FREQ=WEEKLY";

#[test]
fn patch_occurrence_applies_offset_start_as_absolute_instant() {
    let original = event("series-offset@test", WEEKLY);
    let out = patch_occurrence(
        Some(&original),
        "20251225T100000Z",
        &times("2026-08-06T18:30:00+09:00", "2026-08-06T19:30:00+09:00"),
    )
    .unwrap();
    assert!(out.contains("DTSTART:20260806T093000Z") && out.contains("DTEND:20260806T103000Z"), "{out}");
}

#[test]
fn patch_updates_all_day_dates() {
    let original =
        event("allday-update@test", "DTSTART;VALUE=DATE:20251225\nDTEND;VALUE=DATE:20251226\nSUMMARY:Christmas Day");
    let e = parsed_one(&patched(
        &original,
        "allday-update@test",
        EventPatch {
            start_date: Some("2025-12-31".into()),
            end_date: Some("2025-12-31".into()),
            is_all_day: Some(true),
            ..EventPatch::default()
        },
    ));
    assert!(e.is_all_day);
    assert_eq!(e.start_date.as_deref(), Some("2025-12-31"));
    assert_eq!(e.end_date.as_deref(), Some("2025-12-31"));
}

#[test]
fn patch_updates_times_with_timezone() {
    let original = event("tz-update@test", &format!("{TIMED}\nSUMMARY:Timezone Event"));
    let out = patched(
        &original,
        "tz-update@test",
        EventPatch { timezone: Some("America/New_York".into()), ..times("2025-12-26T10:00:00", "2025-12-26T11:00:00") },
    );
    assert!(out.contains("TZID=America/New_York"));
    let e = parsed_one(&out);
    assert!(!e.is_all_day);
    assert_eq!(e.start_time.as_deref(), Some("2025-12-26T15:00:00Z"));
}

#[test]
fn patch_keeps_existing_times_when_no_time_params() {
    let original = event("keep-time@test", &format!("{TIMED}\nSUMMARY:Keep My Time"));
    let e = parsed_one(&patched(&original, "keep-time@test", summary("Title Changed Only")));
    assert_eq!(e.start_time.as_deref(), Some("2025-12-25T10:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-12-25T11:00:00Z"));
}

// ───────────────────────────── extended properties ─────────────────────────────

#[test]
fn patch_updates_status() {
    let original = event("status-test@test", &format!("{TIMED}\nSUMMARY:Event\nSTATUS:CONFIRMED"));
    let out = patched(
        &original,
        "status-test@test",
        EventPatch { status: Some("TENTATIVE".into()), ..EventPatch::default() },
    );
    assert!(out.contains("STATUS:TENTATIVE"));
    assert!(!out.contains("STATUS:CONFIRMED"));
}

#[test]
fn patch_updates_url_categories_and_priority() {
    let original = event("ext@test", &format!("{TIMED}\nSUMMARY:Event\nCATEGORIES:OLD\nPRIORITY:5"));
    let e = parsed_one(&patched(
        &original,
        "ext@test",
        EventPatch {
            url: Some("https://example.com/meeting".into()),
            categories: Some(vec!["MEETING".into(), "WORK".into()]),
            priority: Some(1),
            ..EventPatch::default()
        },
    ));
    assert_eq!(e.url.as_deref(), Some("https://example.com/meeting"));
    assert_eq!(e.categories, vec!["MEETING", "WORK"]);
    assert_eq!(e.priority, Some(1));
}

#[test]
fn patch_keeps_existing_extended_properties_when_not_provided() {
    let original =
        event("keep-ext@test", &format!("{TIMED}\nSUMMARY:Event\nSTATUS:CONFIRMED\nPRIORITY:3\nCATEGORIES:WORK"));
    let out = patched(&original, "keep-ext@test", summary("Updated"));
    assert!(out.contains("STATUS:CONFIRMED"));
    assert!(out.contains("PRIORITY:3"));
    assert!(out.contains("CATEGORIES:WORK"));
}

#[test]
fn patch_updates_and_removes_rrule() {
    let original = event("rrule-test@test", &format!("{TIMED}\nSUMMARY:Weekly\nRRULE:FREQ=WEEKLY;BYDAY=MO"));
    let updated = patched(
        &original,
        "rrule-test@test",
        EventPatch { rrule: Some("FREQ=DAILY;COUNT=10".into()), ..EventPatch::default() },
    );
    assert!(updated.contains("RRULE:FREQ=DAILY"));
    assert!(!updated.contains("BYDAY=MO"));
    let removed =
        patched(&original, "rrule-test@test", EventPatch { rrule: Some(String::new()), ..EventPatch::default() });
    assert!(!removed.contains("RRULE:"));
}

// ───────────────────────────── fallback ─────────────────────────────

#[test]
fn patch_builds_fresh_event_when_existing_is_missing_or_blank() {
    let fields =
        EventPatch { summary: Some("New Event".into()), ..times("2025-12-25T10:00:00Z", "2025-12-25T11:00:00Z") };
    for existing in [None, Some("  ")] {
        let out = patch(existing, "fallback@test", &fields).unwrap();
        assert!(out.contains("BEGIN:VCALENDAR"));
        assert!(out.contains("SUMMARY:New Event"));
        assert!(out.contains("UID:fallback@test"));
    }
}

#[test]
fn patch_errors_on_unparseable_existing_ics() {
    let err = patch(
        Some("not valid ical data"),
        "fallback-invalid@test",
        &EventPatch { summary: Some("Fallback Event".into()), ..times("2025-12-25T10:00:00Z", "2025-12-25T11:00:00Z") },
    )
    .unwrap_err();
    assert!(matches!(err, PatchError::Unparseable(_)));
    assert!(!err.to_string().trim().is_empty());
}

#[test]
fn patch_recovers_original_data_when_existing_uses_lf_only_folding() {
    let desc = "DESCRIPTION:Send a short follow-up email with an attachment for a warm introduction. Frame around the pitch, not generic availability.";
    let folded = format!("{}\n {}", &desc[..75], &desc[75..]);
    let lf_only = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Test//EN",
        "BEGIN:VEVENT",
        "UID:lf-fold@test",
        "DTSTAMP:20260115T100000Z",
        "DTSTART:20260120T140000Z",
        "DTEND:20260120T150000Z",
        "SUMMARY:Original SUMMARY value",
        &folded,
        "END:VEVENT",
        "END:VCALENDAR",
    ]
    .join("\n");
    let out = patched(
        &lf_only,
        "lf-fold@test",
        EventPatch { description: Some("MCP TEST touch simple.".into()), ..EventPatch::default() },
    );
    let e = parsed_one(&out);
    assert_eq!(e.summary, "Original SUMMARY value");
    assert_eq!(e.description.as_deref(), Some("MCP TEST touch simple."));
}

#[test]
fn patch_uses_untitled_when_summary_missing_for_fallback() {
    let out = patch(None, "untitled@test", &times("2025-12-25T10:00:00Z", "2025-12-25T11:00:00Z")).unwrap();
    assert!(out.contains("SUMMARY:Untitled"));
}

#[test]
fn patched_ics_has_valid_structure_and_fresh_dtstamp() {
    let original = ics(
        "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//Test//EN\nBEGIN:VEVENT\nUID:structure@test\nDTSTAMP:20200101T000000Z\nDTSTART:20251225T100000Z\nDTEND:20251225T110000Z\nSUMMARY:Event\nEND:VEVENT\nEND:VCALENDAR",
    );
    let out = patched(&original, "structure@test", summary("Updated"));
    for needle in ["BEGIN:VCALENDAR", "END:VCALENDAR", "VERSION:2.0", "BEGIN:VEVENT", "END:VEVENT"] {
        assert!(out.contains(needle));
    }
    assert!(!out.contains("DTSTAMP:20200101T000000Z"), "DTSTAMP is regenerated");
}

#[test]
fn round_trip_patch_then_parse_preserves_all_fields() {
    let original = event(
        "roundtrip@test",
        &format!(
            "{TIMED}\nSUMMARY:Original\nDESCRIPTION:A description\nLOCATION:Conference Room\nRRULE:FREQ=WEEKLY;BYDAY=TH\nSTATUS:CONFIRMED\nPRIORITY:3\nCATEGORIES:MEETING,WORK\nORGANIZER;CN=Boss:mailto:boss@example.com\nATTENDEE;CN=Worker:mailto:worker@example.com\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder\nEND:VALARM"
        ),
    );
    let e = parsed_one(&patched(&original, "roundtrip@test", summary("Updated Title")));
    assert_eq!(e.uid, "roundtrip@test");
    assert_eq!(e.summary, "Updated Title");
    assert_eq!(e.description.as_deref(), Some("A description"));
    assert_eq!(e.location.as_deref(), Some("Conference Room"));
    assert_eq!(e.start_time.as_deref(), Some("2025-12-25T10:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-12-25T11:00:00Z"));
    assert!(e.rrule.as_deref().unwrap().contains("FREQ=WEEKLY"));
    assert_eq!(e.status.as_deref(), Some("CONFIRMED"));
    assert_eq!(e.priority, Some(3));
    assert_eq!(e.categories, vec!["MEETING", "WORK"]);
    assert!(e.organizer.is_some());
    assert_eq!(e.attendee_count, 1);
    assert_eq!(e.alarms.len(), 1);
}

#[test]
fn round_trip_multiple_patches_preserve_properties() {
    let original = event(
        "multi-patch@test",
        &format!(
            "{TIMED}\nSUMMARY:V1\nSEQUENCE:0\nORGANIZER;CN=John:mailto:john@example.com\nATTENDEE;CN=Jane:mailto:jane@example.com\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT30M\nDESCRIPTION:Alarm\nEND:VALARM"
        ),
    );
    let v2 = patched(&original, "multi-patch@test", summary("V2"));
    assert!(v2.contains("SEQUENCE:1"));
    let v3 = patched(&v2, "multi-patch@test", EventPatch { location: Some("Room 42".into()), ..EventPatch::default() });
    assert!(v3.contains("SEQUENCE:2"));
    let v4 =
        patched(&v3, "multi-patch@test", EventPatch { description: Some("New desc".into()), ..EventPatch::default() });
    assert!(v4.contains("SEQUENCE:3"));
    let e = parsed_one(&v4);
    assert_eq!(e.summary, "V2");
    assert_eq!(e.location.as_deref(), Some("Room 42"));
    assert_eq!(e.description.as_deref(), Some("New desc"));
    assert_eq!(e.attendee_count, 1);
    assert_eq!(v4.matches("BEGIN:VALARM").count(), 1);
}

#[test]
fn patch_removes_description_and_location_when_blank() {
    let original =
        event("rm@test", &format!("{TIMED}\nSUMMARY:Event\nDESCRIPTION:Old description\nLOCATION:Old Location"));
    let no_desc =
        patched(&original, "rm@test", EventPatch { description: Some(String::new()), ..EventPatch::default() });
    assert!(!no_desc.contains("DESCRIPTION:"));
    let no_loc = patched(&original, "rm@test", EventPatch { location: Some(String::new()), ..EventPatch::default() });
    assert!(!no_loc.contains("LOCATION:"));
}

#[test]
fn all_day_dtend_is_exclusive() {
    let original =
        event("allday-exc@test", "DTSTART;VALUE=DATE:20251225\nDTEND;VALUE=DATE:20251226\nSUMMARY:Christmas");
    let out = patched(
        &original,
        "allday-exc@test",
        EventPatch {
            start_date: Some("2026-02-18".into()),
            end_date: Some("2026-02-18".into()),
            is_all_day: Some(true),
            ..EventPatch::default()
        },
    );
    assert!(out.contains("DTSTART;VALUE=DATE:20260218"));
    assert!(out.contains("DTEND;VALUE=DATE:20260219"));
}

#[test]
fn multi_day_all_day_fresh_event_has_exclusive_dtend() {
    let out = patch(
        None,
        "multiday@test",
        &EventPatch {
            summary: Some("3 Day Event".into()),
            start_date: Some("2026-02-18".into()),
            end_date: Some("2026-02-20".into()),
            is_all_day: Some(true),
            ..EventPatch::default()
        },
    )
    .unwrap();
    assert!(out.contains("DTSTART;VALUE=DATE:20260218"));
    assert!(out.contains("DTEND;VALUE=DATE:20260221"));
}

#[test]
fn patch_updates_all_fields_simultaneously() {
    let original = event(
        "full-update@test",
        &format!(
            "{TIMED}\nSUMMARY:Original\nDESCRIPTION:Old desc\nLOCATION:Old place\nSTATUS:CONFIRMED\nPRIORITY:5\nCATEGORIES:OLD\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder\nEND:VALARM"
        ),
    );
    let out = patched(
        &original,
        "full-update@test",
        EventPatch {
            summary: Some("New Title".into()),
            description: Some("New description".into()),
            location: Some("New Place".into()),
            status: Some("TENTATIVE".into()),
            priority: Some(1),
            categories: Some(vec!["MEETING".into(), "IMPORTANT".into()]),
            rrule: Some("FREQ=DAILY;COUNT=5".into()),
            ..times("2025-12-26T14:00:00Z", "2025-12-26T16:00:00Z")
        },
    );
    let e = parsed_one(&out);
    assert_eq!(e.summary, "New Title");
    assert_eq!(e.description.as_deref(), Some("New description"));
    assert_eq!(e.location.as_deref(), Some("New Place"));
    assert_eq!(e.start_time.as_deref(), Some("2025-12-26T14:00:00Z"));
    assert_eq!(e.end_time.as_deref(), Some("2025-12-26T16:00:00Z"));
    assert_eq!(e.status.as_deref(), Some("TENTATIVE"));
    assert_eq!(e.priority, Some(1));
    assert_eq!(e.categories, vec!["MEETING", "IMPORTANT"]);
    assert!(e.rrule.is_some());
    assert!(out.contains("BEGIN:VALARM"));
}

fn with_timestamps() -> String {
    ics("
        BEGIN:VCALENDAR
        VERSION:2.0
        PRODID:-//Test//Test//EN
        BEGIN:VEVENT
        UID:lm-test@example.com
        DTSTAMP:20240101T000000Z
        CREATED:20240101T000000Z
        LAST-MODIFIED:20240101T000000Z
        DTSTART:20260115T100000Z
        DTEND:20260115T110000Z
        SUMMARY:Original
        END:VEVENT
        END:VCALENDAR
    ")
    .replace('\n', "\r\n")
}

#[test]
fn patch_refreshes_last_modified_but_preserves_created() {
    let out = patched(&with_timestamps(), "lm-test@example.com", summary("Edited"));
    assert!(out.contains("CREATED:20240101T000000Z"), "{out}");
    assert!(!out.contains("LAST-MODIFIED:20240101T000000Z"), "{out}");
    assert!(out.lines().any(|l| l.starts_with("LAST-MODIFIED:")));
    let second = patched(&out, "lm-test@example.com", summary("Second edit"));
    assert!(second.contains("CREATED:20240101T000000Z"), "{second}");
}

fn recurring_with(extra: &str) -> String {
    ics(&format!(
        "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\nBEGIN:VEVENT\nUID:rd@test\nDTSTAMP:20260101T000000Z\nDTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nRRULE:FREQ=WEEKLY\n{extra}\nSUMMARY:Recurring\nEND:VEVENT\nEND:VCALENDAR"
    ))
    .replace('\n', "\r\n")
}

#[test]
fn patch_replaces_rdate_and_exdate_values() {
    let out = patched(
        &recurring_with("RDATE:20260214T100000Z"),
        "rd@test",
        EventPatch {
            rdates: Some(vec!["2026-03-14T10:00:00Z".into(), "2026-04-14T10:00:00Z".into()]),
            ..EventPatch::default()
        },
    );
    assert!(!out.contains("20260214T100000Z"), "{out}");
    assert!(out.contains("20260314T100000Z") && out.contains("20260414T100000Z"));
    let out = patched(
        &recurring_with("EXDATE:20260212T100000Z"),
        "rd@test",
        EventPatch { exdates: Some(vec!["2026-02-19T10:00:00Z".into()]), ..EventPatch::default() },
    );
    assert!(!out.contains("20260212T100000Z"), "{out}");
    assert!(out.contains("20260219T100000Z"));
}

#[test]
fn patch_without_rdates_leaves_existing_untouched() {
    let out = patched(&recurring_with("RDATE:20260214T100000Z"), "rd@test", summary("Edited title only"));
    assert!(out.contains("20260214T100000Z"), "{out}");
}

#[test]
fn patch_does_not_synthesize_created_when_missing() {
    let original =
        event("no-created@example.com", "DTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nSUMMARY:No CREATED here");
    let out = patched(&original, "no-created@example.com", summary("Edited"));
    assert!(!out.lines().any(|l| l.starts_with("CREATED:")), "{out}");
}

// ───────────────────────────── alarms ─────────────────────────────

fn two_alarms() -> String {
    event(
        "alarm-patch@test",
        "DTSTART:20260115T100000Z\nDTEND:20260115T110000Z\nSUMMARY:With two alarms\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Primary\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nDESCRIPTION:Day before\nEND:VALARM",
    )
    .replace('\n', "\r\n")
}

#[test]
fn patch_with_alarms_replaces_existing_alarms() {
    let out = patched(
        &two_alarms(),
        "alarm-patch@test",
        EventPatch {
            alarms: Some(vec![AlarmSpec { trigger: "-PT5M".into(), ..AlarmSpec::default() }]),
            ..EventPatch::default()
        },
    );
    assert_eq!(out.matches("BEGIN:VALARM").count(), 1, "{out}");
    assert!(out.contains("TRIGGER:-PT5M"));
    assert!(!out.contains("TRIGGER:-PT15M"));
    assert!(!out.contains("TRIGGER:-P1D"));
}

#[test]
fn patch_without_alarms_preserves_and_empty_alarms_clears() {
    let kept = patched(&two_alarms(), "alarm-patch@test", summary("Edit only"));
    assert_eq!(kept.matches("BEGIN:VALARM").count(), 2);
    let cleared =
        patched(&two_alarms(), "alarm-patch@test", EventPatch { alarms: Some(vec![]), ..EventPatch::default() });
    assert!(!cleared.contains("BEGIN:VALARM"));
}

// ───────────────────────────── single occurrence ─────────────────────────────

fn vevent_count(ics: &str) -> usize {
    ics.matches("BEGIN:VEVENT").count()
}

const WEEKLY_TIMED_SERIES: &str = "
    BEGIN:VCALENDAR
    VERSION:2.0
    PRODID:-//Test//Test//EN
    BEGIN:VEVENT
    UID:series@test
    DTSTAMP:20260101T000000Z
    DTSTART:20260105T090000Z
    DTEND:20260105T100000Z
    RRULE:FREQ=WEEKLY;BYDAY=MO
    SUMMARY:Weekly sync
    END:VEVENT
    END:VCALENDAR";

const WEEKLY_WITH_FEB2_OVERRIDE: &str = "
    BEGIN:VCALENDAR
    VERSION:2.0
    PRODID:-//Test//Test//EN
    BEGIN:VEVENT
    UID:series@test
    DTSTAMP:20260101T000000Z
    DTSTART:20260105T090000Z
    DTEND:20260105T100000Z
    RRULE:FREQ=WEEKLY;BYDAY=MO
    SUMMARY:Weekly sync
    END:VEVENT
    BEGIN:VEVENT
    UID:series@test
    DTSTAMP:20260101T000000Z
    RECURRENCE-ID:20260202T090000Z
    DTSTART:20260202T093000Z
    DTEND:20260202T103000Z
    SUMMARY:Old override
    LOCATION:Room 5
    END:VEVENT
    END:VCALENDAR";

const YEARLY_ALL_DAY: &str = "
    BEGIN:VCALENDAR
    VERSION:2.0
    PRODID:-//Test//Test//EN
    BEGIN:VEVENT
    UID:allday@test
    DTSTAMP:20230101T000000Z
    DTSTART;VALUE=DATE:20230517
    DTEND;VALUE=DATE:20230518
    RRULE:FREQ=YEARLY
    SUMMARY:Anniversary
    END:VEVENT
    END:VCALENDAR";

fn standalone() -> String {
    event("single@test", "DTSTART:20260105T090000Z\nDTEND:20260105T100000Z\nSUMMARY:One-off")
}

#[test]
fn patch_occurrence_adds_exception_with_recurrence_id_and_patched_summary() {
    let out = patch_occurrence(Some(&ics(WEEKLY_TIMED_SERIES)), "20260202T090000Z", &summary("Moved sync")).unwrap();
    assert_eq!(vevent_count(&out), 2, "{out}");
    assert!(out.contains("RECURRENCE-ID:20260202T090000Z"), "{out}");
    assert!(out.contains("SUMMARY:Moved sync"));
    assert!(out.contains("DTSTART:20260105T090000Z"), "master DTSTART unchanged");
    assert!(out.contains("RRULE:FREQ=WEEKLY;BYDAY=MO"));
    assert_eq!(out.matches("RRULE:").count(), 1, "only the master carries an RRULE");
}

#[test]
fn patch_occurrence_with_only_summary_keeps_exception_at_occurrence_instant() {
    let out = patch_occurrence(Some(&ics(WEEKLY_TIMED_SERIES)), "20260202T090000Z", &summary("Renamed")).unwrap();
    assert!(out.contains("DTSTART:20260202T090000Z"), "{out}");
}

#[test]
fn patch_occurrence_moving_time_keeps_identity() {
    let out = patch_occurrence(
        Some(&ics(WEEKLY_TIMED_SERIES)),
        "20260202T090000Z",
        &times("2026-02-02T15:00:00Z", "2026-02-02T16:00:00Z"),
    )
    .unwrap();
    assert!(out.contains("RECURRENCE-ID:20260202T090000Z"));
    assert!(out.contains("DTSTART:20260202T150000Z"), "{out}");
}

#[test]
fn patch_occurrence_updates_existing_exception_in_place() {
    let out =
        patch_occurrence(Some(&ics(WEEKLY_WITH_FEB2_OVERRIDE)), "20260202T090000Z", &summary("New override")).unwrap();
    assert_eq!(vevent_count(&out), 2, "{out}");
    assert!(out.contains("SUMMARY:New override"));
    assert!(!out.contains("SUMMARY:Old override"));
}

#[test]
fn exdate_occurrence_adds_exdate_and_leaves_series_intact() {
    let out = exdate_occurrence(Some(&ics(WEEKLY_TIMED_SERIES)), "20260202T090000Z").unwrap();
    assert_eq!(vevent_count(&out), 1);
    assert!(out.contains("EXDATE:20260202T090000Z"), "{out}");
    assert!(out.contains("DTSTART:20260105T090000Z"));
    assert!(out.contains("RRULE:FREQ=WEEKLY;BYDAY=MO"));
}

#[test]
fn exdate_occurrence_drops_matching_exception() {
    let out = exdate_occurrence(Some(&ics(WEEKLY_WITH_FEB2_OVERRIDE)), "20260202T090000Z").unwrap();
    assert_eq!(vevent_count(&out), 1, "{out}");
    assert!(!out.contains("SUMMARY:Old override"));
    assert!(out.contains("EXDATE:20260202T090000Z"));
}

#[test]
fn occurrence_ops_work_for_all_day_series_with_date_recurrence_id() {
    let series = ics(YEARLY_ALL_DAY);
    let out = patch_occurrence(Some(&series), "20260517", &summary("Special anniversary")).unwrap();
    assert_eq!(vevent_count(&out), 2);
    assert!(out.contains("RECURRENCE-ID;VALUE=DATE:20260517"), "{out}");
    assert!(out.contains("DTSTART;VALUE=DATE:20260517"));
    assert!(out.contains("SUMMARY:Special anniversary"));
    let exdated = exdate_occurrence(Some(&series), "20260517").unwrap();
    assert!(exdated.contains("EXDATE;VALUE=DATE:20260517"), "{exdated}");
}

#[test]
fn occurrence_ops_reject_non_recurring_event() {
    let one_off = standalone();
    assert_eq!(
        patch_occurrence(Some(&one_off), "20260105T090000Z", &summary("nope")),
        Err(PatchError::NotARecurringSeries)
    );
    assert_eq!(exdate_occurrence(Some(&one_off), "20260105T090000Z"), Err(PatchError::NotARecurringSeries));
    assert_eq!(truncate_series(Some(&one_off), "20260105T090000Z"), Err(PatchError::NotARecurringSeries));
    assert_eq!(
        split_series(Some(&one_off), "20260105T090000Z", &summary("nope")).unwrap_err(),
        PatchError::NotARecurringSeries
    );
}

// ───────────────────────────── this and future ─────────────────────────────

#[test]
fn truncate_series_caps_master_with_until_at_last_kept() {
    let out = truncate_series(Some(&ics(WEEKLY_TIMED_SERIES)), "20260119T090000Z").unwrap();
    assert_eq!(vevent_count(&out), 1);
    assert!(out.contains("DTSTART:20260105T090000Z"));
    assert!(out.contains("FREQ=WEEKLY") && out.contains("BYDAY=MO"));
    assert!(out.contains("UNTIL=20260112T090000Z"), "{out}");
}

#[test]
fn truncate_series_drops_exceptions_at_or_after_cut() {
    let with_future = ics(WEEKLY_WITH_FEB2_OVERRIDE);
    let out = truncate_series(Some(&with_future), "20260119T090000Z").unwrap();
    assert_eq!(vevent_count(&out), 1, "{out}");
    assert!(!out.contains("SUMMARY:Old override"));
}

#[test]
fn truncate_series_emits_date_until_for_all_day_series() {
    let out = truncate_series(Some(&ics(YEARLY_ALL_DAY)), "20260517").unwrap();
    assert!(out.contains("UNTIL=20250517"), "{out}");
    assert!(!out.contains("UNTIL=20250517T"));
}

#[test]
fn truncate_and_split_at_first_occurrence_report_first_occurrence() {
    let series = ics(WEEKLY_TIMED_SERIES);
    assert_eq!(truncate_series(Some(&series), "20260105T090000Z"), Err(PatchError::FirstOccurrence));
    assert_eq!(
        split_series(Some(&series), "20260105T090000Z", &summary("whole")).unwrap_err(),
        PatchError::FirstOccurrence
    );
}

#[test]
fn split_series_truncates_master_and_returns_fresh_series() {
    let split =
        split_series(Some(&ics(WEEKLY_TIMED_SERIES)), "20260119T090000Z", &summary("Renamed from here")).unwrap();
    let master = &split.truncated_master;
    assert_eq!(vevent_count(master), 1);
    assert!(master.contains("UID:series@test"));
    assert!(master.contains("SUMMARY:Weekly sync"));
    assert!(master.contains("UNTIL=20260112T090000Z"), "{master}");
    let series = &split.new_series;
    assert_eq!(vevent_count(series), 1);
    assert!(!series.contains("UID:series@test"), "{series}");
    assert!(series.contains("UID:"));
    assert!(series.contains("SUMMARY:Renamed from here"));
    assert!(series.contains("DTSTART:20260119T090000Z"), "{series}");
    assert!(series.contains("FREQ=WEEKLY"));
    assert!(!series.contains("RECURRENCE-ID"));
    assert!(!series.contains("UNTIL="), "{series}");
}

#[test]
fn split_series_reduces_count_on_new_series_and_caps_master() {
    let daily = event(
        "series@test",
        "DTSTART:20260105T090000Z\nDTEND:20260105T100000Z\nRRULE:FREQ=DAILY;COUNT=5\nSUMMARY:Daily standup",
    );
    let split = split_series(Some(&daily), "20260107T090000Z", &summary("Reworked standup")).unwrap();
    assert!(split.truncated_master.contains("UNTIL=20260106T090000Z"), "{}", split.truncated_master);
    assert!(!split.truncated_master.contains("COUNT="));
    assert!(split.new_series.contains("COUNT=3"), "{}", split.new_series);
    assert!(split.new_series.contains("DTSTART:20260107T090000Z"));
}

#[test]
fn split_series_moves_occurrence_time_when_patch_changes_it() {
    let split = split_series(
        Some(&ics(WEEKLY_TIMED_SERIES)),
        "20260119T090000Z",
        &times("2026-01-19T14:00:00Z", "2026-01-19T15:00:00Z"),
    )
    .unwrap();
    assert!(split.new_series.contains("DTSTART:20260119T140000Z"), "{}", split.new_series);
}

#[test]
fn split_series_carries_later_modified_instance_onto_new_series() {
    let split =
        split_series(Some(&ics(WEEKLY_WITH_FEB2_OVERRIDE)), "20260119T090000Z", &summary("Renamed from here")).unwrap();
    assert_eq!(vevent_count(&split.truncated_master), 1, "{}", split.truncated_master);
    let series = &split.new_series;
    assert_eq!(vevent_count(series), 2, "{series}");
    assert!(series.contains("RECURRENCE-ID:20260202T090000Z"), "{series}");
    assert!(series.contains("LOCATION:Room 5"));
    assert!(!series.contains("UID:series@test"));
    let uids: std::collections::BTreeSet<&str> =
        series.lines().filter_map(|l| l.trim_end_matches('\r').strip_prefix("UID:")).collect();
    assert_eq!(uids.len(), 1, "new series and its exception share one UID: {uids:?}");
}

#[test]
fn split_series_drops_exception_exactly_at_cut() {
    let split = split_series(
        Some(&ics(WEEKLY_WITH_FEB2_OVERRIDE)),
        "20260202T090000Z",
        &EventPatch {
            summary: Some("Renamed from the moved one".into()),
            location: Some("Room 9".into()),
            ..EventPatch::default()
        },
    )
    .unwrap();
    let series = &split.new_series;
    assert!(!series.contains("RECURRENCE-ID"), "{series}");
    assert_eq!(vevent_count(series), 1);
    assert!(series.contains("LOCATION:Room 9"));
    assert!(!series.contains("Room 5"));
}

#[test]
fn split_and_patch_results_parse_back_cleanly() {
    let split = split_series(Some(&ics(WEEKLY_TIMED_SERIES)), "20260119T090000Z", &summary("x")).unwrap();
    assert_eq!(parse_all_events(&split.truncated_master).unwrap().len(), 1);
    assert_eq!(parse_all_events(&split.new_series).unwrap().len(), 1);
}
