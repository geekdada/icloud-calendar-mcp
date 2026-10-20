//! The MCP-facing ICS layer on top of [`crate::ical`]:
//!
//! - [`parser`]: flattens events (and expanded occurrences) into [`ParsedEvent`]s.
//! - [`builder`]: builds a new event body from tool inputs.
//! - [`patcher`]: edits an existing body (whole event, one occurrence, or a
//!   this-and-future split) while preserving everything it does not touch.

pub mod builder;
pub mod parser;
pub mod patcher;

use chrono::{DateTime, NaiveDate, NaiveDateTime};

pub use builder::{AlarmSpec, NewEvent, build_event};
pub use parser::{ParsedAlarm, ParsedEvent, parse_events, parse_occurrences};
pub use patcher::{EventPatch, PatchError, SplitResult};

use crate::ical::timezone::{local_to_timestamp, resolve_tzid};
use crate::ical::{ICalDateTime, ICalError};
use crate::patterns::has_explicit_zone;

/// Strip CR/LF so a value like `text\r\nX-EVIL:injected` cannot smuggle a
/// separate property into the emitted calendar.
pub fn sanitize_text(value: &str) -> String {
    value.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// An ISO 8601 date-time from a tool input, before zone resolution.
enum IsoDateTime {
    /// Carried a `Z` or numeric offset: an absolute instant (epoch millis).
    Absolute(i64),
    /// No zone: a wall-clock time to be anchored.
    Naive(NaiveDateTime),
}

fn parse_iso(value: &str) -> Option<IsoDateTime> {
    let value = value.trim();
    if has_explicit_zone(value) {
        let normalized = with_seconds(value);
        return DateTime::parse_from_rfc3339(&normalized).ok().map(|dt| IsoDateTime::Absolute(dt.timestamp_millis()));
    }
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
        .map(IsoDateTime::Naive)
}

/// Insert `:00` seconds into `YYYY-MM-DDTHH:MM` followed by a zone designator.
fn with_seconds(value: &str) -> String {
    let zone_at = value.rfind(['Z', '+', '-']).filter(|&i| i > 10).unwrap_or(value.len());
    let (time, zone) = value.split_at(zone_at);
    if time.len() == 16 { format!("{time}:00{zone}") } else { value.to_string() }
}

/// Resolve a timed tool input the way the write path stores it: an explicit
/// `Z`/offset is that instant (and wins over `timezone`); a naive value is
/// wall-clock time in `timezone`, or UTC when no zone is given.
pub fn resolve_timed(value: &str, timezone: Option<&str>) -> Result<ICalDateTime, ICalError> {
    match parse_iso(value).ok_or_else(|| ICalError::new(format!("Invalid datetime: {value}")))? {
        IsoDateTime::Absolute(timestamp) => Ok(ICalDateTime::utc(timestamp)),
        IsoDateTime::Naive(local) => match timezone {
            None => Ok(ICalDateTime::utc(local.and_utc().timestamp_millis())),
            Some(tzid) => {
                let zone = resolve_tzid(tzid);
                Ok(ICalDateTime {
                    timestamp: local_to_timestamp(local, zone),
                    timezone: Some(zone),
                    is_utc: false,
                    is_date: false,
                })
            }
        },
    }
}

/// Parse a `YYYY-MM-DD` tool input.
pub fn parse_date(value: &str) -> Result<NaiveDate, ICalError> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| ICalError::new(format!("Invalid date: {value}")))
}

/// An RDATE/EXDATE tool input: a DATE for all-day events, else a UTC instant
/// (a naive value is read as UTC). A value of the other shape is coerced
/// (date part, or UTC midnight) rather than rejected.
pub fn recurrence_date(value: &str, is_all_day: bool) -> Result<ICalDateTime, ICalError> {
    let value = value.trim();
    if is_all_day {
        let date_part = value.split('T').next().unwrap_or(value);
        return parse_date(date_part).map(ICalDateTime::from_local_date);
    }
    match parse_date(value) {
        Ok(date) => Ok(ICalDateTime::utc(date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp_millis())),
        Err(_) => resolve_timed(value, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Tz;

    #[test]
    fn resolves_timed_inputs() {
        assert_eq!(
            resolve_timed("2025-01-15T10:00:00Z", Some("Asia/Tokyo")).unwrap().to_ical_string(),
            "20250115T100000Z"
        );
        assert_eq!(resolve_timed("2025-01-15T19:00:00+09:00", None).unwrap().to_ical_string(), "20250115T100000Z");
        assert_eq!(resolve_timed("2025-01-15T10:00:00", None).unwrap().to_ical_string(), "20250115T100000Z");
        let zoned = resolve_timed("2025-01-15T10:00", Some("America/New_York")).unwrap();
        assert_eq!(zoned.timezone, Some(Tz::America__New_York));
        assert_eq!(zoned.to_ical_string(), "20250115T100000");
        assert_eq!(resolve_timed("2025-01-15T10:00Z", None).unwrap().to_ical_string(), "20250115T100000Z");
        assert_eq!(resolve_timed("2025-01-15T10:00-05:00", None).unwrap().to_ical_string(), "20250115T150000Z");
        assert!(resolve_timed("not a time", None).is_err());
    }

    #[test]
    fn recurrence_dates_follow_event_kind() {
        assert_eq!(recurrence_date("2025-01-15", true).unwrap().to_ical_string(), "20250115");
        assert_eq!(recurrence_date("2025-01-15T10:00:00Z", false).unwrap().to_ical_string(), "20250115T100000Z");
        assert_eq!(recurrence_date("2025-01-15T10:00:00", false).unwrap().to_ical_string(), "20250115T100000Z");
        assert_eq!(recurrence_date("2025-01-15", false).unwrap().to_ical_string(), "20250115T000000Z");
        assert_eq!(recurrence_date("2025-01-15T10:00:00Z", true).unwrap().to_ical_string(), "20250115");
    }

    #[test]
    fn sanitize_strips_line_breaks() {
        assert_eq!(sanitize_text("a\r\nX-EVIL:1\nb\rc"), "a X-EVIL:1 b c");
    }
}
