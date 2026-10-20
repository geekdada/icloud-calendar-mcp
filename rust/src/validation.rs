//! Input validation for MCP tool parameters.
//!
//! Every tool argument passes through here before any CalDAV call, so malformed
//! or hostile input is rejected up front with a message the caller can act on.
//! Messages are kept identical to the Kotlin build so clients see the same text.

use std::sync::LazyLock;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use chrono_tz::Tz;
use regex::Regex;

use crate::ical::timezone::local_to_timestamp;
use crate::patterns::{ICAL_ABSOLUTE_TRIGGER, has_explicit_zone};
use crate::scope::EventScope;

/// `Ok(())` when valid, otherwise the caller-facing message.
pub type ValidationResult = Result<(), String>;

const MAX_TITLE_LENGTH: usize = 500;
const MAX_CALENDAR_ID_LENGTH: usize = 500;
const MAX_TIMEZONE_LENGTH: usize = 100;
const MAX_ALARMS: usize = 8;
const MAX_ALARM_TEXT_LENGTH: usize = 500;
const MAX_RECURRENCE_DATES: usize = 366;

/// Default cap for free-text fields such as `description`.
pub const MAX_DESCRIPTION_LENGTH: usize = 5000;

/// Default cap for `location`.
pub const MAX_LOCATION_LENGTH: usize = 500;

/// Largest `get_events` window, in days between start_date and end_date, so a
/// full leap year (2024-01-01..2025-01-01, 366 days) is accepted. Caps the work
/// a single read can request before any CalDAV fetch.
pub const MAX_RANGE_DAYS: i64 = 366;

// `[0-9]` rather than `\d`: the Rust `\d` class is Unicode-aware, and the
// Kotlin patterns only ever accepted ASCII digits.
static DATE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$").expect("valid regex"));

// Naive, UTC (`Z`) or numeric-offset datetimes, with optional seconds.
static DATETIME_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2})(?::([0-9]{2}))?(Z|([+-])([0-9]{2}):([0-9]{2}))?$",
    )
    .expect("valid regex")
});

static CALENDAR_ID_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9\-_:/\.@]+$").expect("valid regex"));

// RFC 5545 §3.3.6 duration, approximately; the writer has the final say.
static ALARM_DURATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[+-]?P(?:T(?:[0-9]+H)?(?:[0-9]+M)?(?:[0-9]+S)?|(?:[0-9]+W|[0-9]+D)(?:T(?:[0-9]+H)?(?:[0-9]+M)?(?:[0-9]+S)?)?)$",
    )
    .expect("valid regex")
});

const ALARM_ACTIONS: [&str; 3] = ["DISPLAY", "AUDIO", "EMAIL"];

const DANGEROUS_SCHEMES: [&str; 5] = ["file://", "javascript:", "data:", "vbscript:", "ftp://"];

const SSRF_PATTERNS: [&str; 23] = [
    "localhost",
    "127.0.0.1",
    "::1",
    "0.0.0.0",
    "169.254.",
    "10.",
    "192.168.",
    "172.16.",
    "172.17.",
    "172.18.",
    "172.19.",
    "172.20.",
    "172.21.",
    "172.22.",
    "172.23.",
    "172.24.",
    "172.25.",
    "172.26.",
    "172.27.",
    "172.28.",
    "172.29.",
    "172.30.",
    "172.31.",
];

const DANGEROUS_PATTERNS: [&str; 19] =
    ["..", ";", "|", "`", "$(", "${", "<", ">", "\n", "\r", "%00", "\\x00", "*", "?", "[", "{", "^", "$", "+"];

/// One entry of the `alarms` argument, decoded from JSON at the MCP boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlarmInput {
    pub trigger: Option<String>,
    pub action: Option<String>,
    pub description: Option<String>,
    pub summary: Option<String>,
    pub repeat_count: Option<i64>,
    pub repeat_duration: Option<String>,
}

/// Length as Kotlin's `String.length` counts it (UTF-16 code units), so the
/// size caps reject exactly what the Kotlin build rejected.
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn is_blank(text: Option<&str>) -> bool {
    text.is_none_or(|t| t.trim().is_empty())
}

/// Validate a `YYYY-MM-DD` date.
pub fn validate_date(date: Option<&str>, field_name: &str) -> ValidationResult {
    let Some(date) = date.filter(|d| !d.trim().is_empty()) else {
        return Err(format!("{field_name} is required"));
    };
    if !DATE_PATTERN.is_match(date) {
        return Err(format!("{field_name} must be in YYYY-MM-DD format"));
    }
    parse_local_date(date).map(|_| ()).ok_or_else(|| format!("{field_name} is not a valid date"))
}

fn parse_local_date(date: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

/// Validate the span between a `get_events` start_date and end_date: rejects an
/// inverted range and one wider than [`MAX_RANGE_DAYS`].
///
/// A missing or malformed date passes, leaving the format error to
/// [`validate_date`], so the two compose without double-reporting.
pub fn validate_date_span(start_date: Option<&str>, end_date: Option<&str>) -> ValidationResult {
    let (Some(start), Some(end)) = (strict_date(start_date), strict_date(end_date)) else {
        return Ok(());
    };
    let span_days = (end - start).num_days();
    if span_days < 0 {
        return Err("end_date must not precede start_date".to_string());
    }
    if span_days > MAX_RANGE_DAYS {
        return Err(format!(
            "Date range is {span_days} days, which exceeds the maximum of {MAX_RANGE_DAYS} days; narrow the date range"
        ));
    }
    Ok(())
}

fn strict_date(date: Option<&str>) -> Option<NaiveDate> {
    let date = date.filter(|d| !d.trim().is_empty() && DATE_PATTERN.is_match(d))?;
    parse_local_date(date)
}

/// A datetime matched by [`DATETIME_PATTERN`] and range-checked.
struct IsoDateTime {
    local: NaiveDateTime,
    /// Seconds east of UTC; `None` for a naive (zone-less) value.
    offset_seconds: Option<i32>,
}

fn parse_iso_datetime(datetime: &str) -> Option<IsoDateTime> {
    let caps = DATETIME_PATTERN.captures(datetime)?;
    let number = |i: usize| caps.get(i).map_or(Some(0), |m| m.as_str().parse::<u32>().ok());
    let date = NaiveDate::from_ymd_opt(caps[1].parse().ok()?, number(2)?, number(3)?)?;
    let time = NaiveTime::from_hms_opt(number(4)?, number(5)?, number(6)?)?;
    let offset_seconds = match caps.get(7).map(|m| m.as_str()) {
        None => None,
        Some("Z") => Some(0),
        Some(_) => {
            let (hours, minutes) = (number(9)?, number(10)?);
            // java.time.ZoneOffset accepts at most ±18:00.
            if minutes > 59 || hours * 3600 + minutes * 60 > 18 * 3600 {
                return None;
            }
            let magnitude = i32::try_from(hours * 3600 + minutes * 60).ok()?;
            Some(if &caps[8] == "-" { -magnitude } else { magnitude })
        }
    };
    Some(IsoDateTime { local: date.and_time(time), offset_seconds })
}

/// Validate an ISO 8601 datetime: naive, UTC (`Z`), or with a `±HH:MM` offset.
pub fn validate_date_time(datetime: Option<&str>, field_name: &str) -> ValidationResult {
    let Some(datetime) = datetime.filter(|d| !d.trim().is_empty()) else {
        return Err(format!("{field_name} is required"));
    };
    if !DATETIME_PATTERN.is_match(datetime) {
        return Err(format!(
            "{field_name} must be in ISO 8601 format: naive local (YYYY-MM-DDTHH:MM:SS), UTC (…Z), or with an offset (…+HH:MM)"
        ));
    }
    parse_iso_datetime(datetime).map(|_| ()).ok_or_else(|| format!("{field_name} is not a valid datetime"))
}

/// Validate an optional IANA timezone identifier (e.g. `America/New_York`).
pub fn validate_timezone(timezone: Option<&str>, field_name: &str) -> ValidationResult {
    let Some(timezone) = timezone.filter(|t| !t.trim().is_empty()) else {
        return Ok(());
    };
    if utf16_len(timezone) > MAX_TIMEZONE_LENGTH {
        return Err(format!("{field_name} is too long"));
    }
    timezone.parse::<Tz>().map(|_| ()).map_err(|_| format!("{field_name} is not a recognized IANA timezone"))
}

/// Validate the `alarms` list: at most 8 entries, each with an RFC 5545 duration
/// or absolute UTC trigger, a known action, and bounded text.
pub fn validate_alarm_list(alarms: Option<&[AlarmInput]>, field_name: &str) -> ValidationResult {
    let Some(alarms) = alarms.filter(|a| !a.is_empty()) else {
        return Ok(());
    };
    if alarms.len() > MAX_ALARMS {
        return Err(format!("{field_name} has too many entries (max 8)"));
    }
    alarms.iter().enumerate().try_for_each(|(i, alarm)| validate_alarm(alarm, &format!("{field_name}[{i}]")))
}

fn validate_alarm(alarm: &AlarmInput, prefix: &str) -> ValidationResult {
    let Some(trigger) = alarm.trigger.as_deref() else {
        return Err(format!("{prefix}.trigger is required"));
    };
    if !ALARM_DURATION_PATTERN.is_match(trigger) && !ICAL_ABSOLUTE_TRIGGER.is_match(trigger) {
        return Err(format!(
            "{prefix}.trigger must be an RFC 5545 duration (e.g. -PT15M) or absolute UTC instant (yyyyMMddTHHmmssZ)"
        ));
    }
    if alarm.action.as_deref().is_some_and(|a| !ALARM_ACTIONS.contains(&a)) {
        return Err(format!("{prefix}.action must be one of DISPLAY/AUDIO/EMAIL"));
    }
    if alarm.description.as_deref().is_some_and(|d| utf16_len(d) > MAX_ALARM_TEXT_LENGTH) {
        return Err(format!("{prefix}.description exceeds 500 characters"));
    }
    if alarm.summary.as_deref().is_some_and(|s| utf16_len(s) > MAX_ALARM_TEXT_LENGTH) {
        return Err(format!("{prefix}.summary exceeds 500 characters"));
    }
    if alarm.repeat_count.is_some_and(|c| c < 0) {
        return Err(format!("{prefix}.repeat_count must be >= 0"));
    }
    if alarm.repeat_duration.as_deref().is_some_and(|d| !ALARM_DURATION_PATTERN.is_match(d)) {
        return Err(format!("{prefix}.repeat_duration must be an RFC 5545 duration"));
    }
    Ok(())
}

/// Validate RDATE/EXDATE values: each a `YYYY-MM-DD` date or a naive/UTC
/// datetime, at most 366 entries.
pub fn validate_recurrence_date_list(values: Option<&[String]>, field_name: &str) -> ValidationResult {
    let Some(values) = values.filter(|v| !v.is_empty()) else {
        return Ok(());
    };
    if values.len() > MAX_RECURRENCE_DATES {
        return Err(format!("{field_name} has too many entries (max 366)"));
    }
    for (i, value) in values.iter().enumerate() {
        let item = format!("{field_name}[{i}]");
        if validate_date(Some(value), &item).is_ok() {
            continue;
        }
        // The writer appends 'Z' to a naive value and parses it as an instant, so
        // a numeric offset would make it throw; reject offsets here instead.
        let without_z = value.strip_suffix('Z').unwrap_or(value);
        if has_explicit_zone(without_z) {
            return Err(format!("{item} must be a date (YYYY-MM-DD) or a UTC datetime (…Z), not an offset datetime"));
        }
        validate_date_time(Some(without_z), &item)?;
    }
    Ok(())
}

/// Validate a calendar or event identifier, rejecting injection, traversal and
/// SSRF patterns.
pub fn validate_calendar_id(calendar_id: Option<&str>, field_name: &str) -> ValidationResult {
    let Some(calendar_id) = calendar_id.filter(|c| !c.trim().is_empty()) else {
        return Err(format!("{field_name} is required"));
    };
    if utf16_len(calendar_id) > MAX_CALENDAR_ID_LENGTH {
        return Err(format!("{field_name} is too long (max {MAX_CALENDAR_ID_LENGTH} characters)"));
    }
    if !CALENDAR_ID_PATTERN.is_match(calendar_id) || contains_injection_pattern(calendar_id) {
        return Err(format!("{field_name} contains invalid characters"));
    }
    Ok(())
}

fn contains_injection_pattern(input: &str) -> bool {
    let lower = input.to_lowercase();
    DANGEROUS_SCHEMES.iter().any(|scheme| lower.starts_with(scheme))
        || SSRF_PATTERNS.iter().any(|pattern| lower.contains(pattern))
        || DANGEROUS_PATTERNS.iter().any(|pattern| input.contains(pattern))
}

/// Validate an event title (required, at most 500 characters).
pub fn validate_title(title: Option<&str>, field_name: &str) -> ValidationResult {
    let Some(title) = title.filter(|t| !t.trim().is_empty()) else {
        return Err(format!("{field_name} is required"));
    };
    if utf16_len(title) > MAX_TITLE_LENGTH {
        return Err(format!("{field_name} is too long (max {MAX_TITLE_LENGTH} characters)"));
    }
    Ok(())
}

/// Validate an optional free-text field such as `description` or `location`.
pub fn validate_optional_text(text: Option<&str>, field_name: &str, max_length: usize) -> ValidationResult {
    match text {
        Some(text) if !is_blank(Some(text)) && utf16_len(text) > max_length => {
            Err(format!("{field_name} is too long (max {max_length} characters)"))
        }
        _ => Ok(()),
    }
}

/// Validate that `end_time` is after `start_time`.
///
/// Endpoints are compared as the instants the ICS writer will store: an explicit
/// `Z`/offset is that instant; a naive value is anchored to `timezone` (the end
/// to `end_timezone`, falling back to `timezone`), or read as UTC when no zone
/// is given. Resolving identically keeps a mixed naive/zoned range honest.
pub fn validate_time_range(
    start_time: &str,
    end_time: &str,
    timezone: Option<&str>,
    end_timezone: Option<&str>,
) -> ValidationResult {
    let start = resolve_instant(start_time, timezone);
    let end = resolve_instant(end_time, end_timezone.or(timezone));
    match (start, end) {
        (Some(start), Some(end)) if end < start => Err("End time must be after start time".to_string()),
        (Some(start), Some(end)) if end == start => Err("End time must be different from start time".to_string()),
        (Some(_), Some(_)) => Ok(()),
        _ => Err("Invalid time format".to_string()),
    }
}

/// Epoch millis a datetime denotes, resolved the way the ICS writer resolves it.
fn resolve_instant(datetime: &str, timezone: Option<&str>) -> Option<i64> {
    let parsed = parse_iso_datetime(datetime)?;
    if let Some(offset) = parsed.offset_seconds {
        return Some((parsed.local.and_utc().timestamp() - i64::from(offset)) * 1000);
    }
    match timezone {
        Some(zone) => Some(local_to_timestamp(parsed.local, zone.parse::<Tz>().ok()?)),
        None => Some(parsed.local.and_utc().timestamp_millis()),
    }
}

/// Neutralize CRLF injection before text reaches the ICS writer.
///
/// Only line breaks are collapsed (to spaces), so a value cannot smuggle a
/// separate property onto the wire. RFC 5545 escaping of `\ , ;` is left to the
/// generator; escaping here too would double-escape.
pub fn sanitize_for_ics(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// Validate the optional `scope` argument: omitted is fine, otherwise it must be
/// one of [`EventScope::TOKENS`].
pub fn validate_scope(scope: Option<&str>, field_name: &str) -> ValidationResult {
    let Some(scope) = scope.filter(|s| !s.trim().is_empty()) else {
        return Ok(());
    };
    if EventScope::from_token(Some(scope)).is_none() {
        return Err(format!("{field_name} must be one of {}", EventScope::TOKENS.join(", ")));
    }
    Ok(())
}

/// Reject series-level fields (rrule / rdates / exdates) under an occurrence
/// scope: they describe the whole recurrence set, so pairing them with a scope
/// that targets occurrences is contradictory.
pub fn validate_occurrence_scope_fields(
    scope: Option<&str>,
    rrule: Option<&str>,
    rdates: Option<&[String]>,
    exdates: Option<&[String]>,
    field_name: &str,
) -> ValidationResult {
    let Some(parsed) = EventScope::from_token(scope) else {
        return Ok(());
    };
    if !parsed.is_occurrence_level() {
        return Ok(());
    }
    let mut offenders = Vec::new();
    if !is_blank(rrule) {
        offenders.push("rrule");
    }
    if rdates.is_some_and(|d| !d.is_empty()) {
        offenders.push("rdates");
    }
    if exdates.is_some_and(|d| !d.is_empty()) {
        offenders.push("exdates");
    }
    if offenders.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} cannot be set with {field_name}={}; series-level fields belong to the whole series (use scope=all_events)",
        offenders.join(", "),
        parsed.token()
    ))
}

/// The messages of every failed result, in order.
pub fn collect_errors(results: &[ValidationResult]) -> Vec<String> {
    results.iter().filter_map(|r| r.as_ref().err().cloned()).collect()
}
