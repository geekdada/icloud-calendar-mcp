//! Flattens iCalendar data into the [`ParsedEvent`] shape the MCP tools return:
//! timed events as UTC ISO 8601 instants, all-day events as inclusive dates.
//!
//! Cancelled events and events without a SUMMARY are skipped.

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use crate::ical::duration::format_duration;
use crate::ical::expander::{ExpansionLimit, expand};
use crate::ical::parser::parse_all_events;
use crate::ical::{EventStatus, ICalAlarm, ICalDateTime, ICalEvent};

/// Upper bound on how far expansion is widened backward to catch an occurrence
/// that started before the window but is still running. Mirrors the 366-day
/// get_events span cap: an occurrence longer than that which also started more
/// than a year earlier is effectively permanent, and capping the pad keeps a
/// pathological long-duration series from widening expansion without bound.
const MAX_EXPANSION_PAD: TimeDelta = TimeDelta::days(366);

/// End assumed for a timed event with neither DTEND nor DURATION. Shared by the
/// reported end and the expansion pad so the two cannot drift apart.
const DEFAULT_TIMED_DURATION: TimeDelta = TimeDelta::hours(1);

/// One event (or one occurrence of a series) as reported to MCP clients.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedEvent {
    pub uid: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub is_all_day: bool,
    /// ISO 8601 UTC instant, timed events only.
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    /// `YYYY-MM-DD`, all-day events only; the end date is inclusive.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// TZID of DTSTART, when zoned.
    pub timezone: Option<String>,
    /// TZID of DTEND, when zoned differently from DTSTART.
    pub end_timezone: Option<String>,
    pub rrule: Option<String>,
    /// RECURRENCE-ID in wire form (`20260818T140000Z` / `20260818`) when this
    /// is one occurrence of a series.
    pub recurrence_id: Option<String>,
    pub rdates: Vec<String>,
    pub exdates: Vec<String>,
    /// `TENTATIVE` or `CONFIRMED` when the source carried a STATUS.
    pub status: Option<String>,
    pub url: Option<String>,
    pub categories: Vec<String>,
    /// 1 (highest) to 9; `None` when undefined.
    pub priority: Option<i32>,
    /// `Name <email>` or `email`.
    pub organizer: Option<String>,
    pub attendee_count: usize,
    pub alarms: Vec<ParsedAlarm>,
}

/// A VALARM as reported to MCP clients; mirrors [`super::AlarmSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAlarm {
    /// `-PT15M` duration form or absolute `20260115T093000Z`.
    pub trigger: String,
    pub action: String,
    pub description: Option<String>,
    pub summary: Option<String>,
    pub repeat_count: Option<i32>,
    pub repeat_duration: Option<String>,
}

/// Every VEVENT mapped verbatim (a series reports its master's DTSTART).
/// Unparseable data yields an empty list: the read path treats it as "no events".
pub fn parse_events(ics: &str) -> Vec<ParsedEvent> {
    if ics.trim().is_empty() {
        return Vec::new();
    }
    let Ok(events) = parse_all_events(ics) else {
        return Vec::new();
    };
    events.iter().filter_map(|e| map_event(e, None)).collect()
}

/// The occurrences in `range_start..=range_end`, expanding RRULE/RDATE and
/// honoring EXDATE and RECURRENCE-ID overrides. Non-recurring events pass
/// through unchanged, so mixed responses work.
pub fn parse_occurrences(
    ics: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<ParsedEvent>, ExpansionLimit> {
    if ics.trim().is_empty() {
        return Ok(Vec::new());
    }
    let Ok(events) = parse_all_events(ics) else {
        return Ok(Vec::new());
    };
    // A VEVENT carrying RECURRENCE-ID modifies another VEVENT's occurrence; it
    // is folded into its master. With no master at all, map what is there.
    let (masters, overrides): (Vec<&ICalEvent>, Vec<&ICalEvent>) =
        events.iter().partition(|e| e.recurrence_id.is_none());
    if masters.is_empty() {
        return Ok(events.iter().filter_map(|e| map_event(e, None)).collect());
    }

    let mut out = Vec::new();
    for master in masters {
        if !master.is_recurring() {
            out.extend(map_event(master, None));
            continue;
        }
        // The expander matches by start, so an occurrence that began before the
        // window but is still running would be missed; widen backward by the
        // master's own duration and drop those that do not reach the window.
        let pad = effective_duration(master).clamp(TimeDelta::zero(), MAX_EXPANSION_PAD);
        let own_overrides: Vec<ICalEvent> =
            overrides.iter().filter(|o| o.uid == master.uid).map(|o| (*o).clone()).collect();
        let expanded = expand(master, range_start - pad, range_end, &own_overrides)?;
        out.extend(
            expanded
                .iter()
                .filter(|occurrence| reaches_window(occurrence, range_start))
                .filter_map(|occurrence| map_event(occurrence, Some(master))),
        );
    }
    Ok(out)
}

/// Per-occurrence duration, matching the end time reported for the event.
fn effective_duration(event: &ICalEvent) -> TimeDelta {
    if let Some(duration) = event.duration {
        return duration;
    }
    if let Some(end) = event.dt_end {
        return TimeDelta::milliseconds(end.timestamp - event.dt_start.timestamp);
    }
    if event.is_all_day { TimeDelta::days(1) } else { DEFAULT_TIMED_DURATION }
}

/// An occurrence starting inside the window passes; one starting before it
/// passes only while it is still in progress at `range_start`.
fn reaches_window(occurrence: &ICalEvent, range_start: DateTime<Utc>) -> bool {
    let start = occurrence.dt_start.to_utc();
    if start >= range_start {
        return true;
    }
    let end = occurrence.dt_end.map_or_else(|| start + effective_duration(occurrence), |e| e.to_utc());
    end > range_start
}

/// Map an event. With `series_master`, `event` is an expanded occurrence: its
/// identity is its own RECURRENCE-ID (an edited override keeps the original
/// instant) or its start, and it reports the series RRULE.
fn map_event(event: &ICalEvent, series_master: Option<&ICalEvent>) -> Option<ParsedEvent> {
    if event.status == Some(EventStatus::Cancelled) {
        return None;
    }
    let summary = event.summary.as_deref().filter(|s| !s.trim().is_empty())?;
    if event.uid.trim().is_empty() {
        return None;
    }
    let mut parsed = if event.is_all_day { map_all_day(event) } else { map_timed(event) };
    parsed.uid = event.uid.clone();
    parsed.summary = summary.to_string();
    parsed.description = event.description.clone();
    parsed.location = event.location.clone();
    parsed.status = match event.status {
        Some(EventStatus::Tentative) => Some("TENTATIVE".into()),
        Some(EventStatus::Confirmed) => Some("CONFIRMED".into()),
        _ => None,
    };
    parsed.url = event.url.clone();
    parsed.categories = event.categories.clone();
    parsed.priority = (event.priority > 0).then_some(event.priority);
    parsed.organizer = event.organizer.as_ref().map(|o| match &o.name {
        Some(name) => format!("{name} <{}>", o.email),
        None => o.email.clone(),
    });
    parsed.attendee_count = event.attendees.len();
    parsed.recurrence_id = match series_master {
        Some(_) => Some(event.recurrence_id.unwrap_or(event.dt_start).to_ical_string()),
        None => event.recurrence_id.map(|r| r.to_ical_string()),
    };
    parsed.rrule = series_master.and_then(|m| m.rrule.as_ref()).or(event.rrule.as_ref()).map(|r| r.to_ical_string());
    parsed.rdates = event.rdates.iter().map(|d| format_recurrence_date(d, event.is_all_day)).collect();
    parsed.exdates = event.exdates.iter().map(|d| format_recurrence_date(d, event.is_all_day)).collect();
    parsed.alarms = event.alarms.iter().filter_map(map_alarm).collect();
    Some(parsed)
}

fn map_all_day(event: &ICalEvent) -> ParsedEvent {
    let start = event.dt_start.to_local_date();
    // DTEND is exclusive for all-day events; report the inclusive last day.
    let end = event.dt_end.map(|e| e.to_local_date() - TimeDelta::days(1)).unwrap_or(start);
    ParsedEvent {
        is_all_day: true,
        start_date: Some(start.format("%Y-%m-%d").to_string()),
        end_date: Some(end.format("%Y-%m-%d").to_string()),
        ..ParsedEvent::default()
    }
}

fn map_timed(event: &ICalEvent) -> ParsedEvent {
    let start = event.dt_start.to_utc();
    let end = match (event.dt_end, event.duration) {
        (Some(end), _) => end.to_utc(),
        (None, Some(duration)) => start + duration,
        (None, None) => start + DEFAULT_TIMED_DURATION,
    };
    let start_tzid = event.dt_start.timezone.map(|tz| tz.name().to_string());
    let end_tzid = event.dt_end.and_then(|e| e.timezone).map(|tz| tz.name().to_string());
    ParsedEvent {
        is_all_day: false,
        start_time: Some(format_instant(start)),
        end_time: Some(format_instant(end)),
        end_timezone: end_tzid.filter(|end| Some(end) != start_tzid.as_ref()),
        timezone: start_tzid,
        ..ParsedEvent::default()
    }
}

/// The trigger keeps its wire form: a duration for relative alarms, a
/// basic-format UTC instant for absolute ones.
fn map_alarm(alarm: &ICalAlarm) -> Option<ParsedAlarm> {
    let trigger = alarm
        .trigger
        .map(format_duration)
        .or_else(|| alarm.trigger_absolute.map(|t| t.to_ical_string()))?;
    Some(ParsedAlarm {
        trigger,
        action: alarm.action.as_str().to_string(),
        description: alarm.description.clone(),
        summary: alarm.summary.clone(),
        repeat_count: (alarm.repeat_count > 0).then_some(alarm.repeat_count),
        repeat_duration: alarm.repeat_duration.map(format_duration),
    })
}

fn format_recurrence_date(dt: &ICalDateTime, is_all_day: bool) -> String {
    if is_all_day { dt.to_local_date().format("%Y-%m-%d").to_string() } else { format_instant(dt.to_utc()) }
}

/// ISO 8601 UTC with fractional seconds only when present (`2025-01-15T10:00:00Z`).
pub fn format_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn maps_timed_event_to_utc_instants() {
        let events = parse_events(&wrap(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART;TZID=America/New_York:20250115T090000\r\nDTEND;TZID=America/Los_Angeles:20250115T090000\r\nSUMMARY:Flight\r\nPRIORITY:1\r\nEND:VEVENT\r\n",
        ));
        let e = &events[0];
        assert_eq!(e.start_time.as_deref(), Some("2025-01-15T14:00:00Z"));
        assert_eq!(e.end_time.as_deref(), Some("2025-01-15T17:00:00Z"));
        assert_eq!(e.timezone.as_deref(), Some("America/New_York"));
        assert_eq!(e.end_timezone.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(e.priority, Some(1));
        assert!(e.start_date.is_none());
    }

    #[test]
    fn maps_all_day_with_inclusive_end() {
        let events = parse_events(&wrap(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART;VALUE=DATE:20250115\r\nDTEND;VALUE=DATE:20250117\r\nSUMMARY:Trip\r\nEND:VEVENT\r\n",
        ));
        assert_eq!(events[0].start_date.as_deref(), Some("2025-01-15"));
        assert_eq!(events[0].end_date.as_deref(), Some("2025-01-16"));
        assert!(events[0].start_time.is_none());
    }

    #[test]
    fn default_end_is_one_hour_or_duration() {
        let events = parse_events(&wrap(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20250115T100000Z\r\nSUMMARY:x\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nDTSTART:20250115T100000Z\r\nDURATION:PT30M\r\nSUMMARY:y\r\nEND:VEVENT\r\n",
        ));
        assert_eq!(events[0].end_time.as_deref(), Some("2025-01-15T11:00:00Z"));
        assert_eq!(events[1].end_time.as_deref(), Some("2025-01-15T10:30:00Z"));
    }

    #[test]
    fn skips_cancelled_and_untitled() {
        let events = parse_events(&wrap(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20250115T100000Z\r\nSUMMARY:x\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nDTSTART:20250115T100000Z\r\nSUMMARY:  \r\nEND:VEVENT\r\n",
        ));
        assert!(events.is_empty());
        assert!(parse_events("garbage").is_empty());
        assert!(parse_events("").is_empty());
    }

    #[test]
    fn organizer_and_alarms() {
        let events = parse_events(&wrap(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20250115T100000Z\r\nSUMMARY:x\r\nORGANIZER;CN=Jane Doe:mailto:jane@example.com\r\nATTENDEE:mailto:a@example.com\r\nATTENDEE:mailto:b@example.com\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-PT15M\r\nSUMMARY:Subj\r\nREPEAT:2\r\nDURATION:PT5M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        ));
        let e = &events[0];
        assert_eq!(e.organizer.as_deref(), Some("Jane Doe <jane@example.com>"));
        assert_eq!(e.attendee_count, 2);
        assert_eq!(e.alarms[0].trigger, "-PT15M");
        assert_eq!(e.alarms[0].action, "EMAIL");
        assert_eq!(e.alarms[0].repeat_count, Some(2));
        assert_eq!(e.alarms[0].repeat_duration.as_deref(), Some("PT5M"));
    }

    #[test]
    fn occurrences_carry_identity_and_series_rule() {
        let ics = wrap(
            "BEGIN:VEVENT\r\nUID:y\r\nDTSTART:20230610T090000Z\r\nDTEND:20230610T100000Z\r\nRRULE:FREQ=YEARLY\r\nSUMMARY:Anniversary\r\nEND:VEVENT\r\n",
        );
        let occ = parse_occurrences(&ics, utc("2026-01-01T00:00:00Z"), utc("2026-12-31T23:59:59Z")).unwrap();
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].start_time.as_deref(), Some("2026-06-10T09:00:00Z"));
        assert_eq!(occ[0].recurrence_id.as_deref(), Some("20260610T090000Z"));
        assert_eq!(occ[0].rrule.as_deref(), Some("FREQ=YEARLY"));
    }

    #[test]
    fn boundary_spanning_occurrence_is_kept() {
        let ics = wrap(
            "BEGIN:VEVENT\r\nUID:n\r\nDTSTART:20250101T220000Z\r\nDTEND:20250102T020000Z\r\nRRULE:FREQ=DAILY\r\nSUMMARY:Night shift\r\nEND:VEVENT\r\n",
        );
        let occ = parse_occurrences(&ics, utc("2025-03-10T00:00:00Z"), utc("2025-03-10T23:59:59Z")).unwrap();
        let starts: Vec<_> = occ.iter().map(|o| o.start_time.clone().unwrap()).collect();
        assert_eq!(starts, ["2025-03-09T22:00:00Z", "2025-03-10T22:00:00Z"]);
    }

    #[test]
    fn override_keeps_original_instant_as_identity() {
        let ics = wrap(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20250101T100000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSUMMARY:Daily\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d\r\nRECURRENCE-ID:20250102T100000Z\r\nDTSTART:20250102T150000Z\r\nSUMMARY:Moved\r\nEND:VEVENT\r\n",
        );
        let occ = parse_occurrences(&ics, utc("2025-01-01T00:00:00Z"), utc("2025-01-31T00:00:00Z")).unwrap();
        assert_eq!(occ[1].summary, "Moved");
        assert_eq!(occ[1].recurrence_id.as_deref(), Some("20250102T100000Z"));
        assert_eq!(occ[1].start_time.as_deref(), Some("2025-01-02T15:00:00Z"));
    }

    #[test]
    fn overrides_without_master_are_mapped() {
        let ics = wrap("BEGIN:VEVENT\r\nUID:o\r\nRECURRENCE-ID:20250102T100000Z\r\nDTSTART:20250102T150000Z\r\nSUMMARY:Only override\r\nEND:VEVENT\r\n");
        let occ = parse_occurrences(&ics, utc("2025-01-01T00:00:00Z"), utc("2025-01-31T00:00:00Z")).unwrap();
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].recurrence_id.as_deref(), Some("20250102T100000Z"));
    }

    #[test]
    fn expansion_limit_propagates() {
        let ics = wrap("BEGIN:VEVENT\r\nUID:m\r\nDTSTART:20250101T000000Z\r\nRRULE:FREQ=MINUTELY\r\nSUMMARY:x\r\nEND:VEVENT\r\n");
        assert!(parse_occurrences(&ics, utc("2025-01-01T00:00:00Z"), utc("2025-12-31T23:59:59Z")).is_err());
    }
}
