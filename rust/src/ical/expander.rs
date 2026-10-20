//! Expands a recurring event into its occurrences within a time range:
//! `(DTSTART ∪ RRULE ∪ RDATE) − EXDATE`, with RECURRENCE-ID overrides
//! substituted for the occurrences they modify.

use std::fmt;

use chrono::{DateTime, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;

use super::datetime::ICalDateTime;
use super::model::ICalEvent;
use super::recur::{PeriodLimitExceeded, rule_dates};
use super::timezone::{local_to_timestamp, system_zone};

/// Most instances one series may generate inside a window. A pathological rule
/// (FREQ=SECONDLY, or a very wide window) aborts with [`ExpansionLimit`]
/// instead of returning a truncated, silently wrong list.
pub const MAX_ITERATIONS: usize = 10_000;

/// How far apart an occurrence and a RECURRENCE-ID may be and still match:
/// absorbs sub-second rounding and peers that are off by up to a minute.
const OVERRIDE_MATCH_TOLERANCE_MS: i64 = 60_000;

/// A series expanded to more than [`MAX_ITERATIONS`] instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionLimit {
    pub uid: String,
    pub limit: usize,
}

impl fmt::Display for ExpansionLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Recurrence expansion for series '{}' exceeded the limit of {} occurrences", self.uid, self.limit)
    }
}

impl std::error::Error for ExpansionLimit {}

/// The zone occurrences are generated in: UTC for all-day and UTC-form series,
/// else the DTSTART's zone.
#[derive(Clone, Copy)]
enum EventZone {
    Utc,
    Region(Tz),
}

impl EventZone {
    fn of(master: &ICalEvent) -> Self {
        if master.is_all_day || master.dt_start.is_utc {
            EventZone::Utc
        } else {
            EventZone::Region(master.dt_start.timezone.unwrap_or_else(system_zone))
        }
    }

    fn tz(self) -> Tz {
        match self {
            EventZone::Utc => Tz::UTC,
            EventZone::Region(tz) => tz,
        }
    }

    fn local(self, timestamp: i64) -> NaiveDateTime {
        let utc = DateTime::<Utc>::from_timestamp_millis(timestamp).unwrap_or_default();
        utc.with_timezone(&self.tz()).naive_local()
    }

    /// An occurrence start at local `local`, typed like the master's DTSTART.
    fn occurrence(self, local: NaiveDateTime, is_date: bool) -> ICalDateTime {
        if is_date {
            return ICalDateTime::from_local_date(local.date());
        }
        let timestamp = local_to_timestamp(local, self.tz());
        match self {
            EventZone::Utc => ICalDateTime::utc(timestamp),
            EventZone::Region(tz) => ICalDateTime { timestamp, timezone: Some(tz), is_utc: false, is_date: false },
        }
    }
}

/// Expand `master` into the occurrences starting in `range_start..=range_end`
/// (RDATEs: `[range_start, range_end)`), sorted by start. A non-recurring event comes back unchanged.
///
/// `overrides` are the RECURRENCE-ID instances of the same UID; each replaces
/// at most one generated occurrence.
pub fn expand(
    master: &ICalEvent,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    overrides: &[ICalEvent],
) -> Result<Vec<ICalEvent>, ExpansionLimit> {
    if !master.is_recurring() {
        return Ok(vec![master.clone()]);
    }
    let zone = EventZone::of(master);
    let duration = event_duration(master);
    let excluded_days: Vec<String> = master.exdates.iter().map(ICalDateTime::to_day_code).collect();
    let mut overrides_by_instant: Vec<(i64, &ICalEvent, bool)> = overrides
        .iter()
        .filter_map(|o| o.recurrence_id.map(|rid| (normalize_to_master_value_type(&rid, &master.dt_start).timestamp, o, false)))
        .collect();
    let mut take_override = |instant: i64| -> Option<ICalEvent> {
        let slot = overrides_by_instant
            .iter_mut()
            .find(|(at, _, used)| !*used && (*at - instant).abs() <= OVERRIDE_MATCH_TOLERANCE_MS)?;
        slot.2 = true;
        Some(slot.1.clone())
    };

    let mut occurrences = Vec::new();
    let mut generated_days: Vec<String> = Vec::new();

    if let Some(rule) = &master.rrule {
        let seed = local_seed(master, zone);
        let to_local = |instant: DateTime<Utc>| -> NaiveDateTime {
            let local = zone.local(instant.timestamp_millis());
            if master.is_all_day { local.date().and_time(NaiveTime::MIN) } else { local }
        };
        let until = rule.until.map(|u| until_in_zone(&u, zone));
        // RRULE instants include the range end (as ical4j's getDates did): for an
        // all-day series the end is truncated to that day's midnight, and an
        // occurrence on that day must still be generated.
        let end_inclusive = to_local(range_end) + TimeDelta::nanoseconds(1);
        let dates = rule_dates(rule, seed, until, to_local(range_start), end_inclusive, MAX_ITERATIONS + 1)
            .map_err(|PeriodLimitExceeded| ExpansionLimit { uid: master.uid.clone(), limit: MAX_ITERATIONS })?;
        if dates.len() > MAX_ITERATIONS {
            return Err(ExpansionLimit { uid: master.uid.clone(), limit: MAX_ITERATIONS });
        }
        for local in dates {
            let start = zone.occurrence(local, master.is_all_day);
            let day_code = local.date().format("%Y%m%d").to_string();
            if excluded_days.contains(&day_code) {
                continue;
            }
            generated_days.push(day_code);
            if let Some(replacement) = take_override(local_to_timestamp(local, zone.tz())) {
                occurrences.push(replacement);
                continue;
            }
            occurrences.push(occurrence_of(master, start, duration));
        }
    }

    for rdate in &master.rdates {
        if rdate.timestamp < range_start.timestamp_millis() || rdate.timestamp >= range_end.timestamp_millis() {
            continue;
        }
        let day_code = rdate.to_day_code();
        if excluded_days.contains(&day_code) || generated_days.contains(&day_code) {
            continue;
        }
        generated_days.push(day_code);
        if let Some(replacement) = take_override(rdate.timestamp) {
            occurrences.push(replacement);
            continue;
        }
        occurrences.push(occurrence_of(master, *rdate, duration));
    }

    occurrences.sort_by_key(|o| o.dt_start.timestamp);
    Ok(occurrences)
}

/// The master's DTSTART as the wall-clock seed (midnight for all-day series).
fn local_seed(master: &ICalEvent, zone: EventZone) -> NaiveDateTime {
    let local = zone.local(master.dt_start.timestamp);
    if master.is_all_day { local.date().and_time(NaiveTime::MIN) } else { local }
}

/// UNTIL on the series' wall clock: a UTC or zoned UNTIL is converted at its
/// instant; a DATE UNTIL covers its whole day; a floating UNTIL is taken as-is.
fn until_in_zone(until: &ICalDateTime, zone: EventZone) -> NaiveDateTime {
    if until.is_date {
        let day = until.to_local_date();
        return day.and_hms_opt(23, 59, 59).unwrap_or_else(|| day.and_time(NaiveTime::MIN));
    }
    if until.is_utc || until.timezone.is_some() {
        return zone.local(until.timestamp);
    }
    until.to_local_date_time()
}

/// DURATION, else DTEND − DTSTART, else unknown.
fn event_duration(event: &ICalEvent) -> Option<TimeDelta> {
    event
        .duration
        .or_else(|| event.dt_end.map(|end| TimeDelta::milliseconds(end.timestamp - event.dt_start.timestamp)))
}

fn occurrence_of(master: &ICalEvent, start: ICalDateTime, duration: Option<TimeDelta>) -> ICalEvent {
    let mut occurrence = master.clone();
    occurrence.dt_end = duration.map(|d| ICalDateTime {
        timestamp: start.timestamp + d.num_milliseconds(),
        timezone: start.timezone,
        is_utc: start.is_utc,
        is_date: master.is_all_day,
    });
    occurrence.dt_start = start;
    occurrence.rrule = None;
    occurrence.exdates = Vec::new();
    occurrence.rdates = Vec::new();
    occurrence.recurrence_id = None;
    occurrence
}

/// Reconcile a RECURRENCE-ID with the master DTSTART's value type.
///
/// Peers sometimes write a DATE RECURRENCE-ID against a timed master (or the
/// reverse), and servers keep it verbatim. A DATE is promoted to the master's
/// time of day in the master's zone; a DATE-TIME against an all-day master is
/// demoted to its calendar date (in its own zone, UTC when floating or `Z`).
pub fn normalize_to_master_value_type(value: &ICalDateTime, master_start: &ICalDateTime) -> ICalDateTime {
    if value.is_date == master_start.is_date {
        return *value;
    }
    if master_start.is_date {
        let zone = value.timezone.unwrap_or(Tz::UTC);
        let date = DateTime::<Utc>::from_timestamp_millis(value.timestamp).unwrap_or_default().with_timezone(&zone).date_naive();
        return ICalDateTime::from_local_date(date);
    }
    let master_zone = master_start.timezone.unwrap_or(Tz::UTC);
    let master_time = DateTime::<Utc>::from_timestamp_millis(master_start.timestamp)
        .unwrap_or_default()
        .with_timezone(&master_zone)
        .time();
    let value_date = DateTime::<Utc>::from_timestamp_millis(value.timestamp).unwrap_or_default().date_naive();
    let timestamp = local_to_timestamp(value_date.and_time(master_time), master_zone);
    ICalDateTime { timestamp, timezone: Some(master_zone), is_utc: false, is_date: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::parser::parse_all_events;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn events(body: &str) -> Vec<ICalEvent> {
        parse_all_events(&format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")).unwrap()
    }

    fn starts(occ: &[ICalEvent]) -> Vec<String> {
        occ.iter().map(|o| o.dt_start.to_ical_string()).collect()
    }

    #[test]
    fn non_recurring_passes_through() {
        let e = events("BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20240101T100000Z\r\nSUMMARY:x\r\nEND:VEVENT\r\n");
        let out = expand(&e[0], utc("2030-01-01T00:00:00Z"), utc("2031-01-01T00:00:00Z"), &[]).unwrap();
        assert_eq!(out, e);
    }

    #[test]
    fn expands_zoned_weekly_series_across_dst() {
        let e = events(
            "BEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=America/New_York:20240304T090000\r\nDTEND;TZID=America/New_York:20240304T100000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        let out = expand(&e[0], utc("2024-03-01T00:00:00Z"), utc("2024-04-01T00:00:00Z"), &[]).unwrap();
        assert_eq!(starts(&out), ["20240304T090000", "20240311T090000", "20240318T090000"]);
        // 09:00 EST is 14:00Z; after the DST change 09:00 EDT is 13:00Z.
        assert_eq!(out[0].dt_start.to_utc(), utc("2024-03-04T14:00:00Z"));
        assert_eq!(out[1].dt_start.to_utc(), utc("2024-03-11T13:00:00Z"));
        assert_eq!(out[1].dt_end.unwrap().to_utc(), utc("2024-03-11T14:00:00Z"));
        assert!(out.iter().all(|o| o.rrule.is_none() && o.recurrence_id.is_none()));
    }

    #[test]
    fn exdate_and_override_applied() {
        let e = events(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20240101T100000Z\r\nDTEND:20240101T110000Z\r\nRRULE:FREQ=DAILY;COUNT=4\r\nEXDATE:20240102T100000Z\r\nSUMMARY:Daily\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d\r\nRECURRENCE-ID:20240103T100000Z\r\nDTSTART:20240103T150000Z\r\nDTEND:20240103T160000Z\r\nSUMMARY:Moved\r\nEND:VEVENT\r\n",
        );
        let out = expand(&e[0], utc("2024-01-01T00:00:00Z"), utc("2024-02-01T00:00:00Z"), &e[1..]).unwrap();
        assert_eq!(starts(&out), ["20240101T100000Z", "20240103T150000Z", "20240104T100000Z"]);
        assert_eq!(out[1].summary.as_deref(), Some("Moved"));
    }

    #[test]
    fn rdates_added_without_duplicates() {
        let e = events(
            "BEGIN:VEVENT\r\nUID:r\r\nDTSTART:20240101T100000Z\r\nRRULE:FREQ=WEEKLY;COUNT=2\r\nRDATE:20240103T100000Z,20240108T100000Z\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        let out = expand(&e[0], utc("2024-01-01T00:00:00Z"), utc("2024-02-01T00:00:00Z"), &[]).unwrap();
        assert_eq!(starts(&out), ["20240101T100000Z", "20240103T100000Z", "20240108T100000Z"]);
    }

    #[test]
    fn all_day_series_stays_on_dates() {
        let e = events(
            "BEGIN:VEVENT\r\nUID:y\r\nDTSTART;VALUE=DATE:20200315\r\nDTEND;VALUE=DATE:20200316\r\nRRULE:FREQ=YEARLY\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        let out = expand(&e[0], utc("2026-01-01T00:00:00Z"), utc("2026-12-31T23:59:59Z"), &[]).unwrap();
        assert_eq!(starts(&out), ["20260315"]);
        assert_eq!(out[0].dt_end.unwrap().to_ical_string(), "20260316");
    }

    #[test]
    fn utc_until_compared_as_instant() {
        // Seoul is UTC+9: the 10:00 KST occurrence on Jan 3 is 01:00Z, exactly UNTIL.
        let e = events(
            "BEGIN:VEVENT\r\nUID:s\r\nDTSTART;TZID=Asia/Seoul:20240101T100000\r\nRRULE:FREQ=DAILY;UNTIL=20240103T010000Z\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
        );
        let out = expand(&e[0], utc("2023-12-31T00:00:00Z"), utc("2024-02-01T00:00:00Z"), &[]).unwrap();
        assert_eq!(starts(&out), ["20240101T100000", "20240102T100000", "20240103T100000"]);
    }

    #[test]
    fn too_many_instances_is_an_error() {
        let e = events("BEGIN:VEVENT\r\nUID:m\r\nDTSTART:20240101T000000Z\r\nRRULE:FREQ=MINUTELY\r\nSUMMARY:x\r\nEND:VEVENT\r\n");
        let err = expand(&e[0], utc("2024-01-01T00:00:00Z"), utc("2024-03-01T00:00:00Z"), &[]).unwrap_err();
        assert_eq!(err, ExpansionLimit { uid: "m".into(), limit: MAX_ITERATIONS });
    }

    #[test]
    fn date_recurrence_id_matches_timed_master() {
        let master_start = ICalDateTime::parse("20240101T090000", Some("America/New_York")).unwrap();
        let recid = ICalDateTime::parse("20240105", None).unwrap();
        let normalized = normalize_to_master_value_type(&recid, &master_start);
        assert_eq!(normalized.to_ical_string(), "20240105T090000");
        assert_eq!(normalized.timezone, Some(Tz::America__New_York));
    }

    #[test]
    fn timed_recurrence_id_matches_all_day_master() {
        let master_start = ICalDateTime::parse("20240101", None).unwrap();
        let recid = ICalDateTime::parse("20240105T000000Z", None).unwrap();
        assert_eq!(normalize_to_master_value_type(&recid, &master_start).to_ical_string(), "20240105");
    }
}
