//! Port of icaldav-core `RRuleExpanderRdateTest`: the recurrence set is
//! `(DTSTART ∪ RRULE ∪ RDATE) − EXDATE` (RFC 5545 §3.8.5).
//!
//! icaldav dropped DTSTART from an RDATE-only series; RFC 5545 makes DTSTART
//! the first instance, so those cases expect one more occurrence here.

mod recurrence_support;

use chrono::{DateTime, Utc};
use icloud_calendar_mcp::ical::ICalEvent;
use recurrence_support::*;

fn range_2026() -> (DateTime<Utc>, DateTime<Utc>) {
    (utc("2026-01-01T00:00:00Z"), utc("2026-12-31T23:59:59Z"))
}

/// A VEVENT starting 2026-01-15 10:00Z with `extra` content lines.
fn event_with(uid: &str, end: &str, extra: &str) -> ICalEvent {
    parse_one(&vcal(&format!(
        "BEGIN:VEVENT\nUID:{uid}\nDTSTAMP:20260101T000000Z\nDTSTART:20260115T100000Z\nDTEND:{end}\n{extra}END:VEVENT\n"
    )))
}

fn one_hour(uid: &str, extra: &str) -> ICalEvent {
    event_with(uid, "20260115T110000Z", extra)
}

fn expand_2026(e: &ICalEvent) -> Vec<ICalEvent> {
    let (start, end) = range_2026();
    expand_all(e, start, end)
}

#[test]
fn rdate_only_series_keeps_dtstart_and_adds_rdates() {
    let e = one_hour("rdate-only@test.com", "RDATE:20260120T100000Z,20260125T100000Z,20260130T100000Z\n");
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260120", "20260125", "20260130"]);
}

#[test]
fn rdate_only_event_is_recurring() {
    let e = one_hour("rdate-recurring@test.com", "RDATE:20260120T100000Z\n");
    assert!(e.is_recurring());
    assert!(e.rrule.is_none());
    assert_eq!(e.rdates.len(), 1);
}

#[test]
fn event_without_rrule_or_rdate_returns_itself() {
    let e = one_hour("no-recurrence@test.com", "SUMMARY:Single event\n");
    let occ = expand_2026(&e);
    assert_eq!(occ.len(), 1);
    assert_eq!(occ[0].uid, e.uid);
}

#[test]
fn rdate_adds_extra_occurrence_to_rrule() {
    let e = one_hour("rrule-rdate@test.com", "RRULE:FREQ=WEEKLY;COUNT=3\nRDATE:20260210T100000Z\n");
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260122", "20260129", "20260210"]);
}

#[test]
fn rdate_equal_to_rrule_instance_is_deduplicated() {
    let e = one_hour("rrule-rdate-dup@test.com", "RRULE:FREQ=WEEKLY;COUNT=3\nRDATE:20260122T100000Z\n");
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260122", "20260129"]);
}

#[test]
fn exdate_excludes_rdate_occurrence() {
    let e = one_hour(
        "rdate-exdate@test.com",
        "RDATE:20260120T100000Z,20260125T100000Z,20260130T100000Z\nEXDATE:20260125T100000Z\n",
    );
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260120", "20260130"]);
}

#[test]
fn exdate_excludes_rrule_instance_while_rdate_survives() {
    let e = one_hour(
        "rrule-rdate-exdate@test.com",
        "RRULE:FREQ=WEEKLY;COUNT=3\nRDATE:20260210T100000Z\nEXDATE:20260122T100000Z\n",
    );
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260129", "20260210"]);
}

#[test]
fn rdate_after_range_is_excluded() {
    let e = one_hour("rdate-range@test.com", "RDATE:20260120T100000Z,20270115T100000Z\n");
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260120"]);
}

#[test]
fn rdate_before_range_is_excluded() {
    let e = one_hour("rdate-before@test.com", "RDATE:20251215T100000Z,20260120T100000Z\n");
    assert_eq!(day_codes(&expand_2026(&e)), ["20260115", "20260120"]);
}

#[test]
fn rdate_occurrence_preserves_master_properties() {
    let e = one_hour(
        "rdate-props@test.com",
        "SUMMARY:Important Meeting\nDESCRIPTION:Test description\nLOCATION:Conference Room\nRDATE:20260120T100000Z\n",
    );
    let occ = expand_2026(&e);
    assert_eq!(occ.len(), 2);
    for o in &occ {
        assert_eq!(o.summary.as_deref(), Some("Important Meeting"));
        assert_eq!(o.description.as_deref(), Some("Test description"));
        assert_eq!(o.location.as_deref(), Some("Conference Room"));
    }
}

#[test]
fn occurrences_carry_no_recurrence_rules() {
    let e = one_hour("rdate-clean@test.com", "RRULE:FREQ=WEEKLY;COUNT=2\nRDATE:20260210T100000Z\n");
    let occ = expand_2026(&e);
    assert_eq!(occ.len(), 3);
    for o in &occ {
        assert!(o.rrule.is_none());
        assert!(o.rdates.is_empty());
        assert!(o.exdates.is_empty());
    }
}

#[test]
fn rdate_occurrence_keeps_master_duration() {
    let e = event_with("rdate-duration@test.com", "20260115T120000Z", "RDATE:20260120T140000Z\n");
    let occ = expand_2026(&e);
    let rdate = occ.iter().find(|o| o.dt_start.to_day_code() == "20260120").expect("RDATE occurrence");
    assert_eq!(rdate.dt_start.timestamp, utc("2026-01-20T14:00:00Z").timestamp_millis());
    assert_eq!(rdate.dt_end.expect("DTEND").timestamp - rdate.dt_start.timestamp, 2 * 3_600_000);
}

#[test]
fn rrule_and_rdate_occurrences_are_sorted_together() {
    let e = one_hour("rdate-sort@test.com", "RRULE:FREQ=MONTHLY;COUNT=2\nRDATE:20260110T100000Z,20260220T100000Z\n");
    assert_eq!(day_codes(&expand_2026(&e)), ["20260110", "20260115", "20260215", "20260220"]);
}
