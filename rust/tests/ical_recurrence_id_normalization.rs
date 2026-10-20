//! Port of icaldav-core `RecurrenceIdNormalizationTest`: a RECURRENCE-ID names
//! the instant of the occurrence it replaces (RFC 5545 §3.8.4.4), so overrides
//! match by normalized instant rather than by calendar day.
//!
//! The Kotlin suite switched the JVM default zone to prove matching does not
//! depend on it. The Rust system zone is fixed per process, so the same
//! property is covered by a Z-form RECURRENCE-ID whose UTC date differs from
//! the master's local date.

mod recurrence_support;

use chrono::{DateTime, Utc};
use icloud_calendar_mcp::ical::expander::{expand, normalize_to_master_value_type};
use icloud_calendar_mcp::ical::{ICalDateTime, ICalEvent};
use recurrence_support::*;

fn parse_dt(value: &str, tzid: Option<&str>) -> ICalDateTime {
    ICalDateTime::parse(value, tzid).unwrap()
}

fn expand_parsed(ics: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<ICalEvent> {
    let events = parse(ics);
    let (overrides, masters): (Vec<ICalEvent>, Vec<ICalEvent>) =
        events.into_iter().partition(|e| e.recurrence_id.is_some());
    expand(&masters[0], from, to, &overrides).unwrap()
}

fn ny_window() -> (DateTime<Utc>, DateTime<Utc>) {
    (instant(zdt(NY, 2023, 12, 1, 0, 0)), instant(zdt(NY, 2023, 12, 10, 0, 0)))
}

/// A daily NY standup at `master_time` plus one override with `recurrence_id_line`.
fn standup_with_override(master_time: &str, recurrence_id_line: &str, override_start: &str) -> String {
    vcal(&format!(
        "BEGIN:VEVENT\nUID:standup\nDTSTART;TZID=America/New_York:20231201T{master_time}\nRRULE:FREQ=DAILY;COUNT=5\n\
         SUMMARY:Standup\nEND:VEVENT\nBEGIN:VEVENT\nUID:standup\n{recurrence_id_line}\n\
         DTSTART;TZID=America/New_York:{override_start}\nSUMMARY:Standup (moved)\nEND:VEVENT\n"
    ))
}

fn assert_dec3_overridden(occ: &[ICalEvent], expected_ny_time: &str) {
    assert_eq!(occ.len(), 5, "override must replace, not add, an occurrence");
    let dec3 = occ.iter().find(|o| local_day(o) == 3).expect("Dec 3 occurrence");
    assert_eq!(dec3.summary.as_deref(), Some("Standup (moved)"));
    assert_eq!(times_in(std::slice::from_ref(dec3), NY), [expected_ny_time]);
    let mut days: Vec<u32> = occ.iter().map(local_day).collect();
    days.sort_unstable();
    assert_eq!(days, [1, 2, 3, 4, 5]);
}

#[test]
fn date_recurrence_id_is_promoted_to_master_time_in_master_zone() {
    let master = parse_dt("20231201T100000", Some("America/New_York"));
    let normalized = normalize_to_master_value_type(&parse_dt("20231203", None), &master);
    assert!(!normalized.is_date);
    assert_eq!(normalized.timestamp, instant(zdt(NY, 2023, 12, 3, 10, 0)).timestamp_millis());
}

#[test]
fn date_time_recurrence_id_is_demoted_to_date_for_all_day_master() {
    let master = parse_dt("20231203", None);
    let normalized = normalize_to_master_value_type(&parse_dt("20231203T120000Z", None), &master);
    assert!(normalized.is_date);
    assert_eq!(normalized.timestamp, parse_dt("20231203", None).timestamp);
}

#[test]
fn matching_value_types_are_returned_unchanged() {
    let master = parse_dt("20231201T100000Z", None);
    let recurrence_id = parse_dt("20231203T100000Z", None);
    assert_eq!(normalize_to_master_value_type(&recurrence_id, &master), recurrence_id);
}

#[test]
fn demotion_uses_the_utc_date_of_the_recurrence_id() {
    // 23:30Z on Dec 3 is already Dec 4 east of UTC; the DATE stays Dec 3.
    let master = parse_dt("20231201", None);
    let normalized = normalize_to_master_value_type(&parse_dt("20231203T233000Z", None), &master);
    assert_eq!(normalized.timestamp, parse_dt("20231203", None).timestamp);
}

#[test]
fn z_form_recurrence_id_overrides_tzid_master_occurrence() {
    let ics = standup_with_override("100000", "RECURRENCE-ID:20231203T150000Z", "20231203T140000");
    let (from, to) = ny_window();
    assert_dec3_overridden(&expand_parsed(&ics, from, to), "14:00");
}

#[test]
fn z_form_recurrence_id_on_next_utc_day_overrides_local_day() {
    // 21:00 New York on Dec 3 is 02:00Z on Dec 4: a day-code match would miss it.
    let ics = standup_with_override("210000", "RECURRENCE-ID:20231204T020000Z", "20231203T220000");
    let (from, to) = ny_window();
    assert_dec3_overridden(&expand_parsed(&ics, from, to), "22:00");
}

#[test]
fn recurrence_id_in_another_zone_matches_by_instant() {
    // 16:00 Paris is 10:00 New York on Dec 3.
    let ics = standup_with_override("100000", "RECURRENCE-ID;TZID=Europe/Paris:20231203T160000", "20231203T140000");
    let (from, to) = ny_window();
    assert_dec3_overridden(&expand_parsed(&ics, from, to), "14:00");
}

#[test]
fn date_recurrence_id_overrides_timed_master_occurrence() {
    let ics = standup_with_override("100000", "RECURRENCE-ID;VALUE=DATE:20231203", "20231203T140000");
    let (from, to) = ny_window();
    assert_dec3_overridden(&expand_parsed(&ics, from, to), "14:00");
}

#[test]
fn date_time_recurrence_id_overrides_all_day_master_occurrence() {
    let ics = vcal(
        "BEGIN:VEVENT\nUID:vacation\nDTSTART;VALUE=DATE:20231201\nDTEND;VALUE=DATE:20231202\nRRULE:FREQ=DAILY;COUNT=5\n\
         SUMMARY:Vacation\nEND:VEVENT\nBEGIN:VEVENT\nUID:vacation\nRECURRENCE-ID:20231203T000000Z\n\
         DTSTART;VALUE=DATE:20231203\nDTEND;VALUE=DATE:20231204\nSUMMARY:Vacation (special)\nEND:VEVENT\n",
    );
    let occ = expand_parsed(&ics, utc("2023-12-01T00:00:00Z"), utc("2023-12-10T00:00:00Z"));
    assert_eq!(occ.len(), 5);
    let dec3 = occ.iter().find(|o| local_day(o) == 3).unwrap();
    assert_eq!(dec3.summary.as_deref(), Some("Vacation (special)"));
}

#[test]
fn unmatched_override_is_not_injected() {
    let ics = standup_with_override("100000", "RECURRENCE-ID:20231220T150000Z", "20231220T140000");
    let (from, to) = ny_window();
    let occ = expand_parsed(&ics, from, to);
    assert_eq!(occ.len(), 5);
    assert!(occ.iter().all(|o| o.summary.as_deref() == Some("Standup")));
}
