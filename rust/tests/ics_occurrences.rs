//! Port of the Kotlin `IcsParserOccurrenceTest`: recurrence expansion on the read path.

use chrono::{DateTime, NaiveDate, Utc};
use icloud_calendar_mcp::ics::{ParsedEvent, parse_events, parse_occurrences};

fn ics(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn cal(body: &str) -> String {
    ics(&format!("BEGIN:VCALENDAR\n{body}\nEND:VCALENDAR"))
}

fn day_start(date: &str) -> DateTime<Utc> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc()
}

fn day_end(date: &str) -> DateTime<Utc> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap().and_hms_opt(23, 59, 59).unwrap().and_utc()
}

/// The window CalendarService builds for `from..=to` (whole UTC days).
fn occ(ics: &str, from: &str, to: &str) -> Vec<ParsedEvent> {
    parse_occurrences(ics, day_start(from), day_end(to)).unwrap()
}

fn starts(events: &[ParsedEvent]) -> Vec<String> {
    let mut s: Vec<String> = events.iter().filter_map(|e| e.start_time.clone()).collect();
    s.sort();
    s
}

const YEARLY_ALLDAY: &str = "
    BEGIN:VEVENT
    UID:yearly-allday@example.com
    DTSTART;VALUE=DATE:20230517
    DTEND;VALUE=DATE:20230518
    RRULE:FREQ=YEARLY
    SUMMARY:Recurring yearly event
    END:VEVENT";

const WEEKLY_TIMED: &str = "
    BEGIN:VEVENT
    UID:weekly-timed@example.com
    DTSTART:20260105T090000Z
    DTEND:20260105T100000Z
    RRULE:FREQ=WEEKLY;BYDAY=MO
    SUMMARY:Weekly sync
    END:VEVENT";

const DAILY_ALLDAY: &str = "
    BEGIN:VEVENT
    UID:daily@example.com
    DTSTART;VALUE=DATE:20260101
    DTEND;VALUE=DATE:20260102
    RRULE:FREQ=DAILY
    SUMMARY:Daily standup
    END:VEVENT";

const OVERNIGHT: &str = "
    BEGIN:VEVENT
    UID:overnight@example.com
    DTSTART:20260601T220000Z
    DTEND:20260602T060000Z
    RRULE:FREQ=DAILY
    SUMMARY:Overnight shift
    END:VEVENT";

const STANDALONE: &str = "
    BEGIN:VEVENT
    UID:single@example.com
    DTSTART;VALUE=DATE:20260725
    DTEND;VALUE=DATE:20260727
    SUMMARY:Two-day all-day event
    END:VEVENT";

#[test]
fn yearly_all_day_event_reports_the_queried_year() {
    let events = occ(&cal(YEARLY_ALLDAY), "2026-05-17", "2026-05-17");
    assert_eq!(events.len(), 1);
    assert!(events[0].is_all_day);
    assert_eq!(events[0].start_date.as_deref(), Some("2026-05-17"));
    assert_eq!(events[0].end_date.as_deref(), Some("2026-05-17"));
}

#[test]
fn unexpanded_parse_still_reports_master_date() {
    let events = parse_events(&cal(YEARLY_ALLDAY));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].start_date.as_deref(), Some("2023-05-17"));
}

#[test]
fn weekly_timed_event_reports_occurrence_in_range() {
    let events = occ(&cal(WEEKLY_TIMED), "2026-02-02", "2026-02-02");
    assert_eq!(events.len(), 1);
    assert!(events[0].start_time.as_deref().unwrap().starts_with("2026-02-02"));
}

#[test]
fn exdate_suppresses_the_excluded_occurrence() {
    let ics = cal("
        BEGIN:VEVENT
        UID:daily-excluded@example.com
        DTSTART;VALUE=DATE:20260101
        DTEND;VALUE=DATE:20260102
        RRULE:FREQ=DAILY
        EXDATE;VALUE=DATE:20260103
        SUMMARY:Daily with a gap
        END:VEVENT");
    assert!(occ(&ics, "2026-01-03", "2026-01-03").is_empty());
    assert_eq!(occ(&ics, "2026-01-04", "2026-01-04").len(), 1);
}

#[test]
fn recurrence_id_override_replaces_that_occurrence() {
    let ics = cal("
        BEGIN:VEVENT
        UID:series@example.com
        DTSTART;VALUE=DATE:20260101
        DTEND;VALUE=DATE:20260102
        RRULE:FREQ=DAILY
        SUMMARY:Original title
        END:VEVENT
        BEGIN:VEVENT
        UID:series@example.com
        RECURRENCE-ID;VALUE=DATE:20260103
        DTSTART;VALUE=DATE:20260103
        DTEND;VALUE=DATE:20260104
        SUMMARY:Overridden title
        END:VEVENT");
    let events = occ(&ics, "2026-01-03", "2026-01-03");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Overridden title");
}

#[test]
fn multi_day_range_returns_every_occurrence() {
    let mut dates: Vec<String> =
        occ(&cal(DAILY_ALLDAY), "2026-03-01", "2026-03-05").into_iter().filter_map(|e| e.start_date).collect();
    dates.sort();
    assert_eq!(dates, vec!["2026-03-01", "2026-03-02", "2026-03-03", "2026-03-04", "2026-03-05"]);
}

#[test]
fn count_limited_series_stops_past_its_end() {
    let ics = cal("
        BEGIN:VEVENT
        UID:limited@example.com
        DTSTART;VALUE=DATE:20260101
        DTEND;VALUE=DATE:20260102
        RRULE:FREQ=DAILY;COUNT=3
        SUMMARY:Three days only
        END:VEVENT");
    assert_eq!(occ(&ics, "2026-01-03", "2026-01-03").len(), 1);
    assert!(occ(&ics, "2026-01-04", "2026-01-04").is_empty());
}

#[test]
fn non_recurring_events_pass_through_unchanged() {
    let events = occ(&cal(STANDALONE), "2026-07-25", "2026-07-25");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].start_date.as_deref(), Some("2026-07-25"));
    assert_eq!(events[0].end_date.as_deref(), Some("2026-07-26"));
    assert!(events[0].rrule.is_none());
}

#[test]
fn blank_or_unparseable_content_yields_no_events() {
    assert!(occ("", "2026-01-01", "2026-01-01").is_empty());
    assert!(occ("   ", "2026-01-01", "2026-01-01").is_empty());
    assert!(occ("not an ics at all", "2026-01-01", "2026-01-01").is_empty());
}

#[test]
fn timed_occurrence_carries_recurrence_id_and_series_rrule() {
    let events = occ(&cal(WEEKLY_TIMED), "2026-02-02", "2026-02-02");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recurrence_id.as_deref(), Some("20260202T090000Z"));
    assert!(events[0].rrule.as_deref().unwrap().contains("FREQ=WEEKLY"));
}

#[test]
fn all_day_occurrence_carries_date_form_recurrence_id() {
    let events = occ(&cal(YEARLY_ALLDAY), "2026-05-17", "2026-05-17");
    assert_eq!(events[0].recurrence_id.as_deref(), Some("20260517"));
    assert!(events[0].rrule.is_some());
}

#[test]
fn standalone_event_carries_no_recurrence_id() {
    let events = occ(&cal(STANDALONE), "2026-07-25", "2026-07-25");
    assert!(events[0].recurrence_id.is_none());
    assert!(events[0].rrule.is_none());
}

#[test]
fn occurrences_of_one_series_carry_distinct_recurrence_ids() {
    let mut ids: Vec<String> =
        occ(&cal(DAILY_ALLDAY), "2026-03-01", "2026-03-03").into_iter().map(|e| e.recurrence_id.unwrap()).collect();
    ids.sort();
    assert_eq!(ids, vec!["20260301", "20260302", "20260303"]);
}

#[test]
fn edited_occurrence_keeps_original_recurrence_id() {
    let ics = cal("
        BEGIN:VEVENT
        UID:series@example.com
        DTSTART:20260105T090000Z
        DTEND:20260105T100000Z
        RRULE:FREQ=DAILY
        SUMMARY:Daily
        END:VEVENT
        BEGIN:VEVENT
        UID:series@example.com
        RECURRENCE-ID:20260107T090000Z
        DTSTART:20260107T150000Z
        DTEND:20260107T160000Z
        SUMMARY:Moved to the afternoon
        END:VEVENT");
    let events = occ(&ics, "2026-01-07", "2026-01-07");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Moved to the afternoon");
    assert_eq!(events[0].recurrence_id.as_deref(), Some("20260107T090000Z"));
    assert!(events[0].start_time.as_deref().unwrap().starts_with("2026-01-07T15:00"));
}

#[test]
fn occurrence_moved_to_another_day_leaves_no_phantom() {
    let ics = cal("
        BEGIN:VEVENT
        UID:series@example.com
        DTSTART:20260105T090000Z
        DTEND:20260105T100000Z
        RRULE:FREQ=DAILY
        SUMMARY:Daily
        END:VEVENT
        BEGIN:VEVENT
        UID:series@example.com
        RECURRENCE-ID:20260107T090000Z
        DTSTART:20260108T150000Z
        DTEND:20260108T160000Z
        SUMMARY:Moved to the next day
        END:VEVENT");
    let events = occ(&ics, "2026-01-06", "2026-01-09");
    let s = starts(&events);
    assert_eq!(events.len(), 4, "{s:?}");
    assert!(s.iter().all(|t| !t.starts_with("2026-01-07")), "{s:?}");
    assert_eq!(s.iter().filter(|t| t.starts_with("2026-01-08")).count(), 2, "{s:?}");
    let moved = events.iter().find(|e| e.recurrence_id.as_deref() == Some("20260107T090000Z")).unwrap();
    assert_eq!(moved.summary, "Moved to the next day");
    assert!(moved.start_time.as_deref().unwrap().starts_with("2026-01-08T15:00"));
}

#[test]
fn daily_overnight_occurrence_spanning_into_queried_day_appears() {
    let s = starts(&occ(&cal(OVERNIGHT), "2026-06-10", "2026-06-10"));
    assert_eq!(s, vec!["2026-06-09T22:00:00Z", "2026-06-10T22:00:00Z"]);
}

#[test]
fn monthly_two_day_all_day_occurrence_queried_on_second_day() {
    let ics = cal("
        BEGIN:VEVENT
        UID:workshop@example.com
        DTSTART;VALUE=DATE:20260115
        DTEND;VALUE=DATE:20260117
        RRULE:FREQ=MONTHLY
        SUMMARY:Two-day workshop
        END:VEVENT");
    let events = occ(&ics, "2026-03-16", "2026-03-16");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].start_date.as_deref(), Some("2026-03-15"));
    assert_eq!(events[0].end_date.as_deref(), Some("2026-03-16"));
}

#[test]
fn occurrence_entirely_before_window_is_absent() {
    let s = starts(&occ(&cal(OVERNIGHT), "2026-06-10", "2026-06-10"));
    assert!(s.iter().all(|t| !t.starts_with("2026-06-08")), "{s:?}");
}

#[test]
fn all_day_occurrence_whose_dtend_touches_window_start_is_excluded() {
    let ics = cal("
        BEGIN:VEVENT
        UID:daily-allday@example.com
        DTSTART;VALUE=DATE:20260201
        DTEND;VALUE=DATE:20260202
        RRULE:FREQ=DAILY
        SUMMARY:Daily all-day
        END:VEVENT");
    let dates: Vec<String> = occ(&ics, "2026-02-10", "2026-02-10").into_iter().filter_map(|e| e.start_date).collect();
    assert_eq!(dates, vec!["2026-02-10"]);
}

#[test]
fn exdate_on_leading_boundary_spanning_occurrence_keeps_it_out() {
    let ics = cal("
        BEGIN:VEVENT
        UID:overnight-ex@example.com
        DTSTART:20260601T220000Z
        DTEND:20260602T060000Z
        RRULE:FREQ=DAILY
        EXDATE:20260609T220000Z
        SUMMARY:Overnight shift with a gap
        END:VEVENT");
    let s = starts(&occ(&ics, "2026-06-10", "2026-06-10"));
    assert!(s.iter().all(|t| !t.starts_with("2026-06-09")), "{s:?}");
    assert!(s.iter().any(|t| t.starts_with("2026-06-10T22:00")), "{s:?}");
}

#[test]
fn override_on_boundary_spanning_occurrence_surfaces_as_override() {
    let ics = cal("
        BEGIN:VEVENT
        UID:overnight-ovr@example.com
        DTSTART:20260601T220000Z
        DTEND:20260602T060000Z
        RRULE:FREQ=DAILY
        SUMMARY:Overnight shift
        END:VEVENT
        BEGIN:VEVENT
        UID:overnight-ovr@example.com
        RECURRENCE-ID:20260609T220000Z
        DTSTART:20260609T220000Z
        DTEND:20260610T063000Z
        SUMMARY:Overnight shift (extended)
        END:VEVENT");
    let events = occ(&ics, "2026-06-10", "2026-06-10");
    let leading: Vec<_> =
        events.iter().filter(|e| e.start_time.as_deref().is_some_and(|t| t.starts_with("2026-06-09"))).collect();
    assert_eq!(leading.len(), 1);
    assert_eq!(leading[0].summary, "Overnight shift (extended)");
    assert_eq!(leading[0].recurrence_id.as_deref(), Some("20260609T220000Z"));
}

#[test]
fn durationless_timed_recurring_occurrence_spanning_into_window_appears() {
    let ics = cal("
        BEGIN:VEVENT
        UID:durationless@example.com
        DTSTART:20260601T233000Z
        RRULE:FREQ=DAILY
        SUMMARY:Late check-in
        END:VEVENT");
    let s = starts(&occ(&ics, "2026-06-10", "2026-06-10"));
    assert!(s.iter().any(|t| t.starts_with("2026-06-09T23:30")), "{s:?}");
}

#[test]
fn rdate_only_boundary_spanning_occurrence_appears() {
    let ics = cal("
        BEGIN:VEVENT
        UID:rdate-only@example.com
        DTSTART:20260609T220000Z
        DTEND:20260610T060000Z
        RDATE:20260609T220000Z
        RDATE:20260612T220000Z
        SUMMARY:RDATE overnight
        END:VEVENT");
    let s = starts(&occ(&ics, "2026-06-10", "2026-06-10"));
    assert!(s.iter().any(|t| t.starts_with("2026-06-09T22:00")), "{s:?}");
    assert!(s.iter().all(|t| !t.starts_with("2026-06-12")), "{s:?}");
}

#[test]
fn orphaned_override_without_master_is_still_mapped() {
    let ics = cal("
        BEGIN:VEVENT
        UID:orphan@example.com
        RECURRENCE-ID;VALUE=DATE:20260103
        DTSTART;VALUE=DATE:20260103
        DTEND;VALUE=DATE:20260104
        SUMMARY:Orphaned override
        END:VEVENT");
    let events = occ(&ics, "2026-01-03", "2026-01-03");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Orphaned override");
}
