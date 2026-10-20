//! Edits an existing ICS body while preserving everything not being changed.
//!
//! The body is parsed into the model, the patch fields are applied, and the
//! result is regenerated. Alarms, ORGANIZER/ATTENDEE, X-* and other unmodelled
//! properties ride through untouched (unless the patch replaces alarms);
//! CREATED is kept, SEQUENCE is bumped and LAST-MODIFIED refreshed.

use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};

use super::builder::{AlarmSpec, NewEvent, build_event, recurrence_dates};
use super::{parse_date, resolve_timed, sanitize_text};
use crate::ical::expander::{ExpansionLimit, expand, normalize_to_master_value_type};
use crate::ical::generator::generate_calendar;
use crate::ical::parser::parse_all_events;
use crate::ical::{EventStatus, ICalDateTime, ICalError, ICalEvent, RRule};

/// Why a patch could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// The existing body is blank, unparseable, or has no (master) VEVENT.
    Unparseable(String),
    /// An occurrence-level edit on an event with no RRULE/RDATE.
    NotARecurringSeries,
    /// A this-and-future cut at the series' first occurrence: act on the whole series instead.
    FirstOccurrence,
    /// Cutting the series needed more expansion than the work bound allows.
    ExpansionLimit(ExpansionLimit),
    /// A patch value (date or time) could not be read.
    InvalidInput(String),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Unparseable(detail) => write!(f, "Could not patch event: existing ICS is unparseable ({detail})"),
            PatchError::NotARecurringSeries => {
                f.write_str("Single-occurrence scope requires a recurring series, but this event has no RRULE/RDATE")
            }
            PatchError::FirstOccurrence => {
                f.write_str("this_and_future targets the first occurrence; act on the whole series instead")
            }
            PatchError::ExpansionLimit(limit) => write!(f, "{limit}"),
            PatchError::InvalidInput(detail) => write!(f, "Invalid patch value: {detail}"),
        }
    }
}

impl std::error::Error for PatchError {}

impl From<ICalError> for PatchError {
    fn from(error: ICalError) -> Self {
        PatchError::InvalidInput(error.message)
    }
}

impl From<ExpansionLimit> for PatchError {
    fn from(limit: ExpansionLimit) -> Self {
        PatchError::ExpansionLimit(limit)
    }
}

/// The fields of an edit. `None` leaves a field untouched; an empty string
/// clears description, location, url or rrule; an empty list clears
/// rdates/exdates/alarms.
#[derive(Debug, Clone, Default)]
pub struct EventPatch {
    pub summary: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub is_all_day: Option<bool>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub timezone: Option<String>,
    pub end_timezone: Option<String>,
    pub rrule: Option<String>,
    pub status: Option<String>,
    pub url: Option<String>,
    pub categories: Option<Vec<String>>,
    pub priority: Option<i32>,
    pub rdates: Option<Vec<String>>,
    pub exdates: Option<Vec<String>>,
    pub alarms: Option<Vec<AlarmSpec>>,
}

impl EventPatch {
    /// CR/LF stripped from every text field, so no value can inject a property.
    fn sanitized(&self) -> EventPatch {
        let text = |v: &Option<String>| v.as_deref().map(sanitize_text);
        let list = |v: &Option<Vec<String>>| v.as_ref().map(|l| l.iter().map(|s| sanitize_text(s)).collect());
        EventPatch {
            summary: text(&self.summary),
            description: text(&self.description),
            location: text(&self.location),
            rrule: text(&self.rrule),
            status: text(&self.status),
            url: text(&self.url),
            categories: list(&self.categories),
            rdates: list(&self.rdates),
            exdates: list(&self.exdates),
            ..self.clone()
        }
    }

    /// This patch without the series-level fields (rrule, rdates, exdates),
    /// which never apply to a single occurrence.
    fn occurrence_only(&self) -> EventPatch {
        EventPatch { rrule: None, rdates: None, exdates: None, ..self.clone() }
    }
}

/// The two bodies a this-and-future edit produces. CalDAV stores one UID per
/// resource, so the continuing series (fresh UID) cannot share the master's
/// `.ics`: `truncated_master` is PUT back, `new_series` is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitResult {
    pub truncated_master: String,
    pub new_series: String,
}

/// Apply `patch` to the first VEVENT of `existing_ics`.
///
/// A blank `existing_ics` is the create path: a fresh event is built instead.
pub fn patch(existing_ics: Option<&str>, uid: &str, patch: &EventPatch) -> Result<String, PatchError> {
    let patch = patch.sanitized();
    let Some(existing) = existing_ics.filter(|ics| !ics.trim().is_empty()) else {
        return Ok(build_fresh(uid, &patch)?);
    };
    let events = parse_all_events(existing).map_err(|e| unparseable(uid, existing, &e.message))?;
    let original = events.into_iter().next().ok_or_else(|| unparseable(uid, existing, "No VEVENT found"))?;
    let patched = apply_patch(original, &patch)?;
    Ok(generate_calendar(&[patched]))
}

/// Edit one occurrence of a recurring series (scope = this occurrence).
///
/// The exception VEVENT for `recurrence_id` is updated in place, or created
/// starting at the occurrence's own instant; the master's DTSTART and RRULE
/// are never touched.
pub fn patch_occurrence(existing_ics: Option<&str>, recurrence_id: &str, patch: &EventPatch) -> Result<String, PatchError> {
    let (master, overrides) = parse_series(existing_ics)?;
    let recid = normalize_recurrence_id(recurrence_id, &master)?;
    let existing_index = overrides.iter().position(|o| normalized_recid_timestamp(o, &master) == Some(recid.timestamp));
    let base = match existing_index {
        Some(i) => overrides[i].clone(),
        None => new_exception_base(&master, recid),
    };
    let patched = apply_patch(base, &patch.sanitized().occurrence_only())?;
    let mut events = vec![master];
    events.extend(overrides.into_iter().enumerate().filter(|(i, _)| Some(*i) != existing_index).map(|(_, o)| o));
    events.push(patched);
    Ok(generate_calendar(&events))
}

/// Cancel one occurrence: add it to the master's EXDATEs and drop any
/// exception VEVENT for that instant.
pub fn exdate_occurrence(existing_ics: Option<&str>, recurrence_id: &str) -> Result<String, PatchError> {
    let (mut master, overrides) = parse_series(existing_ics)?;
    let recid = normalize_recurrence_id(recurrence_id, &master)?;
    let remaining: Vec<ICalEvent> =
        overrides.into_iter().filter(|o| normalized_recid_timestamp(o, &master) != Some(recid.timestamp)).collect();
    if !master.exdates.iter().any(|e| e.timestamp == recid.timestamp) {
        master.exdates.push(recid);
    }
    master.sequence = master.sequence.saturating_add(1);
    master.last_modified = Some(ICalDateTime::now());
    let mut events = vec![master];
    events.extend(remaining);
    Ok(generate_calendar(&events))
}

/// Truncate a series so it ends just before `recurrence_id` (scope =
/// this-and-future delete): RRULE gets UNTIL at the last kept occurrence, and
/// EXDATE/RDATE/exceptions at or after the cut are removed.
pub fn truncate_series(existing_ics: Option<&str>, recurrence_id: &str) -> Result<String, PatchError> {
    let (master, overrides) = parse_series(existing_ics)?;
    let recid = normalize_recurrence_id(recurrence_id, &master)?;
    let (new_master, kept) = truncate_master(&master, &overrides, recid.timestamp)?;
    let mut events = vec![new_master];
    events.extend(kept);
    Ok(generate_calendar(&events))
}

/// Split a series at `recurrence_id` (scope = this-and-future edit): truncate
/// the master as [`truncate_series`] does, and start a new series (fresh UID)
/// at the occurrence carrying the remaining recurrence with the patch applied.
/// Exceptions after the cut move to the new series.
pub fn split_series(existing_ics: Option<&str>, recurrence_id: &str, patch: &EventPatch) -> Result<SplitResult, PatchError> {
    let (master, overrides) = parse_series(existing_ics)?;
    let recid = normalize_recurrence_id(recurrence_id, &master)?;
    let (new_master, kept) = truncate_master(&master, &overrides, recid.timestamp)?;
    let mut truncated = vec![new_master];
    truncated.extend(kept);

    let fresh_uid = format!("{}@onekash-mcp", uuid::Uuid::new_v4());
    let base = new_series_from_occurrence(&master, recid, &fresh_uid)?;
    let content_only = EventPatch {
        status: None,
        url: None,
        categories: None,
        priority: None,
        ..patch.sanitized().occurrence_only()
    };
    let mut new_series = apply_patch(base, &content_only)?;
    new_series.sequence = 0;

    // The override AT the cut is dropped: that instance is the new series'
    // first occurrence, which the patch itself defines.
    let carried = overrides
        .into_iter()
        .filter(|o| normalized_recid_timestamp(o, &master).unwrap_or(i64::MIN) > recid.timestamp)
        .map(|mut o| {
            o.uid = fresh_uid.clone();
            o
        });
    let mut new_events = vec![new_series];
    new_events.extend(carried);
    Ok(SplitResult { truncated_master: generate_calendar(&truncated), new_series: generate_calendar(&new_events) })
}

/// The capped master for a cut at `target` plus the exceptions before it.
fn truncate_master(master: &ICalEvent, overrides: &[ICalEvent], target: i64) -> Result<(ICalEvent, Vec<ICalEvent>), PatchError> {
    let kept = occurrence_instants_before(master, target, true)?;
    let Some(&last_kept) = kept.iter().max() else {
        return Err(PatchError::FirstOccurrence);
    };
    // UNTIL is inclusive, so landing it on the last kept occurrence keeps that
    // one and drops the target. A timed series needs a UTC UNTIL, an all-day
    // series a DATE UNTIL.
    let until = ICalDateTime::from_timestamp(last_kept, None, master.dt_start.is_date);
    let mut new_master = master.clone();
    if let Some(rule) = new_master.rrule.as_mut() {
        rule.until = Some(until);
        rule.count = None;
    }
    new_master.exdates.retain(|e| e.timestamp < target);
    new_master.rdates.retain(|r| r.timestamp < target);
    new_master.sequence = new_master.sequence.saturating_add(1);
    new_master.last_modified = Some(ICalDateTime::now());
    let kept_overrides =
        overrides.iter().filter(|o| normalized_recid_timestamp(o, master).unwrap_or(i64::MAX) < target).cloned().collect();
    Ok((new_master, kept_overrides))
}

/// The continuing series: the master at a fresh UID, starting at the
/// occurrence (DTEND shifted by the master's length), its COUNT reduced by the
/// occurrences the truncated master keeps, and future EXDATE/RDATE carried.
fn new_series_from_occurrence(master: &ICalEvent, recid: ICalDateTime, fresh_uid: &str) -> Result<ICalEvent, PatchError> {
    let target = recid.timestamp;
    let mut series = master.clone();
    series.uid = fresh_uid.to_string();
    series.recurrence_id = None;
    series.recurrence_id_range = None;
    series.dt_start = recid;
    series.dt_end = master.dt_end.map(|end| shifted(end, target + (end.timestamp - master.dt_start.timestamp)));
    series.duration = if master.dt_end.is_none() { master.duration } else { None };
    if let Some(rule) = series.rrule.as_mut()
        && let Some(count) = rule.count
    {
        let consumed = occurrence_instants_before(master, target, false)?.len() as i32;
        rule.count = Some((count - consumed).max(1));
    }
    series.exdates.retain(|e| e.timestamp >= target);
    series.rdates.retain(|r| r.timestamp >= target);
    series.sequence = 0;
    series.created = Some(ICalDateTime::now());
    series.last_modified = Some(ICalDateTime::now());
    Ok(series)
}

/// The base exception for a fresh single-occurrence edit: the master moved to
/// the occurrence's instant, stripped of its recurrence, with RECURRENCE-ID set.
fn new_exception_base(master: &ICalEvent, recid: ICalDateTime) -> ICalEvent {
    let mut exception = master.clone();
    exception.recurrence_id = Some(recid);
    exception.recurrence_id_range = None;
    exception.dt_start = recid;
    exception.dt_end = master.dt_end.map(|end| shifted(end, recid.timestamp + (end.timestamp - master.dt_start.timestamp)));
    exception.duration = if master.dt_end.is_none() { master.duration } else { None };
    exception.rrule = None;
    exception.rdates = Vec::new();
    exception.exdates = Vec::new();
    exception
}

/// `value` moved to `timestamp`, keeping its zone and value type.
fn shifted(value: ICalDateTime, timestamp: i64) -> ICalDateTime {
    ICalDateTime { timestamp, ..value }
}

/// Start instants of the master's occurrences strictly before `target`.
/// Without `apply_exdates`, EXDATE and RDATE are ignored: COUNT bounds RRULE
/// generation independently of them (RFC 5545).
fn occurrence_instants_before(master: &ICalEvent, target: i64, apply_exdates: bool) -> Result<Vec<i64>, PatchError> {
    let mut source = master.clone();
    if !apply_exdates {
        source.exdates.clear();
        source.rdates.clear();
    }
    let start = DateTime::<Utc>::from_timestamp_millis(master.dt_start.timestamp.min(target)).unwrap_or_default();
    let end = DateTime::<Utc>::from_timestamp_millis(target).unwrap_or_default();
    let expanded = expand(&source, start, end, &[])?;
    Ok(expanded.iter().map(|o| o.dt_start.timestamp).filter(|t| *t < target).collect())
}

/// The master VEVENT and its RECURRENCE-ID exceptions; the master must recur.
fn parse_series(existing_ics: Option<&str>) -> Result<(ICalEvent, Vec<ICalEvent>), PatchError> {
    let existing = existing_ics
        .filter(|ics| !ics.trim().is_empty())
        .ok_or_else(|| PatchError::Unparseable("existing ICS is required".into()))?;
    let events = parse_all_events(existing).map_err(|e| unparseable("<occurrence>", existing, &e.message))?;
    let (masters, overrides): (Vec<ICalEvent>, Vec<ICalEvent>) =
        events.into_iter().partition(|e| e.recurrence_id.is_none());
    let master = masters.into_iter().next().ok_or_else(|| PatchError::Unparseable("no master VEVENT found".into()))?;
    if !master.is_recurring() {
        return Err(PatchError::NotARecurringSeries);
    }
    Ok((master, overrides))
}

/// Parse a RECURRENCE-ID wire string, anchoring a local form to the master's
/// zone so the instant is the same on every host, then reconcile its value
/// type with the master's DTSTART.
fn normalize_recurrence_id(recurrence_id: &str, master: &ICalEvent) -> Result<ICalDateTime, PatchError> {
    let raw = recurrence_id.trim();
    let tzid = if raw.len() == 8 || raw.ends_with('Z') { None } else { master.dt_start.timezone.map(|tz| tz.name()) };
    let parsed = ICalDateTime::parse(raw, tzid).map_err(|e| PatchError::InvalidInput(e.message))?;
    Ok(normalize_to_master_value_type(&parsed, &master.dt_start))
}

fn normalized_recid_timestamp(event: &ICalEvent, master: &ICalEvent) -> Option<i64> {
    event.recurrence_id.map(|rid| normalize_to_master_value_type(&rid, &master.dt_start).timestamp)
}

fn unparseable(uid: &str, existing: &str, detail: &str) -> PatchError {
    let fingerprint: String = existing.chars().take(200).collect::<String>().replace(['\r', '\n'], " ⏎ ");
    eprintln!(
        "IcsPatcher: failed to parse existing ICS for uid={uid} ({} chars): {detail} | first 200 chars: {fingerprint}",
        existing.len()
    );
    PatchError::Unparseable(detail.to_string())
}

fn build_fresh(uid: &str, patch: &EventPatch) -> Result<String, ICalError> {
    build_event(&NewEvent {
        uid: Some(uid.to_string()),
        summary: patch.summary.clone().unwrap_or_else(|| "Untitled".into()),
        start_time: patch.start_time.clone(),
        end_time: patch.end_time.clone(),
        start_date: patch.start_date.clone(),
        end_date: patch.end_date.clone(),
        is_all_day: patch.is_all_day.unwrap_or(false),
        description: patch.description.clone(),
        location: patch.location.clone(),
        timezone: patch.timezone.clone(),
        end_timezone: patch.end_timezone.clone(),
        rrule: patch.rrule.clone(),
        status: patch.status.clone(),
        url: patch.url.clone(),
        categories: patch.categories.clone(),
        priority: patch.priority,
        rdates: patch.rdates.clone(),
        exdates: patch.exdates.clone(),
        alarms: patch.alarms.clone(),
        ..NewEvent::default()
    })
}

/// Apply `patch` to `original`. Blank description/location/url/rrule clear
/// the property; every other `None` keeps the existing value.
fn apply_patch(original: ICalEvent, patch: &EventPatch) -> Result<ICalEvent, PatchError> {
    let mut event = original;
    apply_date_times(&mut event, patch)?;
    let cleared = |value: &str| (!value.trim().is_empty()).then(|| value.to_string());
    if let Some(summary) = &patch.summary {
        event.summary = Some(summary.clone());
    }
    if let Some(description) = &patch.description {
        event.description = cleared(description);
    }
    if let Some(location) = &patch.location {
        event.location = cleared(location);
    }
    if let Some(status) = patch.status.as_deref().filter(|s| !s.trim().is_empty()) {
        event.status = Some(EventStatus::from_value(status));
    }
    if let Some(rrule) = &patch.rrule {
        event.rrule = cleared(rrule).and_then(|r| RRule::parse(&r).ok());
    }
    if let Some(exdates) = &patch.exdates {
        event.exdates = recurrence_dates(Some(exdates), event.is_all_day)?;
    }
    if let Some(rdates) = &patch.rdates {
        event.rdates = recurrence_dates(Some(rdates), event.is_all_day)?;
    }
    if let Some(categories) = &patch.categories {
        event.categories = categories.clone();
    }
    if let Some(url) = &patch.url {
        event.url = cleared(url);
    }
    if let Some(priority) = patch.priority {
        event.priority = priority;
    }
    if let Some(alarms) = &patch.alarms {
        event.alarms = alarms.iter().map(AlarmSpec::to_ical_alarm).collect();
    }
    // Saturate: a hostile SEQUENCE:2147483647 must not overflow.
    event.sequence = event.sequence.saturating_add(1);
    event.last_modified = Some(ICalDateTime::now());
    Ok(event)
}

/// DTSTART / DTEND / DURATION / all-day flag after the date and time fields.
fn apply_date_times(event: &mut ICalEvent, patch: &EventPatch) -> Result<(), PatchError> {
    let time_update = patch.start_time.is_some() || patch.end_time.is_some();
    let date_update = patch.start_date.is_some() || patch.end_date.is_some();
    if !time_update && !date_update && patch.is_all_day.is_none() {
        return Ok(());
    }
    let all_day = patch.is_all_day.unwrap_or(date_update && !time_update);
    if all_day && date_update {
        if let Some(start) = &patch.start_date {
            event.dt_start = ICalDateTime::from_local_date(parse_date(start)?);
        }
        let end_input = patch.end_date.as_deref().or(patch.start_date.as_deref()).unwrap_or_default();
        // DTEND is exclusive for all-day events.
        event.dt_end = Some(ICalDateTime::from_local_date(parse_date(end_input)? + TimeDelta::days(1)));
        event.duration = None;
    } else if !all_day && time_update {
        if let Some(start) = &patch.start_time {
            event.dt_start = resolve_timed(start, patch.timezone.as_deref())?;
        }
        if let Some(end) = &patch.end_time {
            // DTEND may carry its own zone (a flight); fall back to the start's.
            let zone = patch.end_timezone.as_deref().or(patch.timezone.as_deref());
            event.dt_end = Some(resolve_timed(end, zone)?);
            event.duration = None;
        }
    }
    event.is_all_day = all_day;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXISTING: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Apple Inc.//iCloud//EN\r\nBEGIN:VEVENT\r\nUID:orig@icloud\r\nDTSTART:20250115T100000Z\r\nDTEND:20250115T110000Z\r\nSUMMARY:Original\r\nDESCRIPTION:Keep me\r\nLOCATION:Room 1\r\nSEQUENCE:2\r\nCREATED:20240101T000000Z\r\nORGANIZER;CN=Boss:mailto:boss@example.com\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\nBEGIN:VALARM\r\nUID:ALARM-1\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nDESCRIPTION:Soon\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const WEEKLY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:series@icloud\r\nDTSTART:20250106T100000Z\r\nDTEND:20250106T110000Z\r\nRRULE:FREQ=WEEKLY;COUNT=6\r\nSUMMARY:Weekly\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn only(ics: &str) -> ICalEvent {
        parse_all_events(ics).unwrap().remove(0)
    }

    #[test]
    fn summary_only_patch_preserves_everything_else() {
        let out = patch(Some(EXISTING), "orig@icloud", &EventPatch { summary: Some("Renamed".into()), ..Default::default() }).unwrap();
        let e = only(&out);
        assert_eq!(e.summary.as_deref(), Some("Renamed"));
        assert_eq!(e.description.as_deref(), Some("Keep me"));
        assert_eq!(e.location.as_deref(), Some("Room 1"));
        assert_eq!(e.sequence, 3);
        assert_eq!(e.created.unwrap().to_ical_string(), "20240101T000000Z");
        assert!(e.last_modified.is_some());
        assert_eq!(e.alarms.len(), 1);
        assert_eq!(e.alarms[0].uid.as_deref(), Some("ALARM-1"));
        assert!(out.contains("ORGANIZER;CN=Boss:mailto:boss@example.com\r\n"));
        assert!(out.contains("ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\n"));
        assert!(out.contains("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\n"));
    }

    #[test]
    fn blank_values_clear_and_alarms_replace() {
        let out = patch(
            Some(EXISTING),
            "orig@icloud",
            &EventPatch { description: Some(String::new()), alarms: Some(vec![]), ..Default::default() },
        )
        .unwrap();
        let e = only(&out);
        assert_eq!(e.description, None);
        assert!(e.alarms.is_empty());
    }

    #[test]
    fn crlf_injection_is_neutralized() {
        let out = patch(
            Some(EXISTING),
            "orig@icloud",
            &EventPatch { summary: Some("Evil\r\nX-INJECTED:yes".into()), ..Default::default() },
        )
        .unwrap();
        assert!(!out.contains("\r\nX-INJECTED"));
        assert_eq!(only(&out).summary.as_deref(), Some("Evil X-INJECTED:yes"));
    }

    #[test]
    fn time_and_date_updates() {
        let timed = patch(
            Some(EXISTING),
            "orig@icloud",
            &EventPatch {
                start_time: Some("2025-02-01T09:00:00".into()),
                end_time: Some("2025-02-01T10:00:00".into()),
                timezone: Some("Europe/Paris".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(timed.contains("DTSTART;TZID=Europe/Paris:20250201T090000\r\n"));
        let all_day =
            patch(Some(EXISTING), "orig@icloud", &EventPatch { start_date: Some("2025-03-01".into()), ..Default::default() })
                .unwrap();
        let e = only(&all_day);
        assert!(e.is_all_day);
        assert_eq!(e.dt_start.to_ical_string(), "20250301");
        assert_eq!(e.dt_end.unwrap().to_ical_string(), "20250302");
    }

    #[test]
    fn unparseable_existing_is_an_error_and_blank_builds_fresh() {
        let err = patch(Some("not valid ical data"), "x", &EventPatch::default()).unwrap_err();
        assert!(matches!(err, PatchError::Unparseable(_)));
        assert!(!err.to_string().is_empty());
        let fresh = patch(None, "new@x", &EventPatch { summary: Some("Fresh".into()), ..Default::default() }).unwrap();
        assert_eq!(only(&fresh).uid, "new@x");
        let untitled = patch(Some("  "), "new@x", &EventPatch::default()).unwrap();
        assert_eq!(only(&untitled).summary.as_deref(), Some("Untitled"));
    }

    #[test]
    fn patch_occurrence_adds_then_updates_one_exception() {
        let edited = patch_occurrence(
            Some(WEEKLY),
            "20250113T100000Z",
            &EventPatch { summary: Some("Moved".into()), rrule: Some("FREQ=DAILY".into()), ..Default::default() },
        )
        .unwrap();
        let events = parse_all_events(&edited).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].rrule.as_ref().unwrap().to_ical_string(), "FREQ=WEEKLY;COUNT=6");
        assert_eq!(events[1].recurrence_id.unwrap().to_ical_string(), "20250113T100000Z");
        assert_eq!(events[1].dt_start.to_ical_string(), "20250113T100000Z");
        assert_eq!(events[1].summary.as_deref(), Some("Moved"));

        let again = patch_occurrence(Some(&edited), "20250113T100000Z", &EventPatch { location: Some("Here".into()), ..Default::default() }).unwrap();
        let events = parse_all_events(&again).unwrap();
        assert_eq!(events.len(), 2, "existing exception is updated, not duplicated");
        assert_eq!(events[1].summary.as_deref(), Some("Moved"));
        assert_eq!(events[1].location.as_deref(), Some("Here"));
    }

    #[test]
    fn exdate_occurrence_excludes_and_drops_exception() {
        let edited = patch_occurrence(Some(WEEKLY), "20250113T100000Z", &EventPatch { summary: Some("x".into()), ..Default::default() }).unwrap();
        let out = exdate_occurrence(Some(&edited), "20250113T100000Z").unwrap();
        let events = parse_all_events(&out).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].exdates[0].to_ical_string(), "20250113T100000Z");
        let twice = exdate_occurrence(Some(&out), "20250113T100000Z").unwrap();
        assert_eq!(parse_all_events(&twice).unwrap()[0].exdates.len(), 1);
    }

    #[test]
    fn occurrence_ops_require_recurring_series() {
        assert_eq!(exdate_occurrence(Some(EXISTING), "20250115T100000Z"), Err(PatchError::NotARecurringSeries));
        assert!(matches!(exdate_occurrence(None, "20250115T100000Z"), Err(PatchError::Unparseable(_))));
    }

    #[test]
    fn truncate_caps_with_until_at_last_kept() {
        let out = truncate_series(Some(WEEKLY), "20250127T100000Z").unwrap();
        let master = only(&out);
        let rule = master.rrule.unwrap();
        assert_eq!(rule.count, None);
        assert_eq!(rule.until.unwrap().to_ical_string(), "20250120T100000Z");
        assert_eq!(truncate_series(Some(WEEKLY), "20250106T100000Z"), Err(PatchError::FirstOccurrence));
    }

    #[test]
    fn split_creates_continuing_series_with_reduced_count() {
        let result = split_series(Some(WEEKLY), "20250120T100000Z", &EventPatch { summary: Some("New era".into()), ..Default::default() }).unwrap();
        let master = only(&result.truncated_master);
        assert_eq!(master.rrule.unwrap().until.unwrap().to_ical_string(), "20250113T100000Z");
        let series = only(&result.new_series);
        assert_ne!(series.uid, "series@icloud");
        assert!(series.uid.ends_with("@onekash-mcp"));
        assert_eq!(series.dt_start.to_ical_string(), "20250120T100000Z");
        assert_eq!(series.dt_end.unwrap().to_ical_string(), "20250120T110000Z");
        assert_eq!(series.rrule.unwrap().count, Some(4));
        assert_eq!(series.summary.as_deref(), Some("New era"));
        assert_eq!(series.sequence, 0);
    }
}
