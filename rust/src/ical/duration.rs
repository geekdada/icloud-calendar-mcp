//! RFC 5545 §3.3.6 DURATION values.

use std::sync::LazyLock;

use chrono::TimeDelta;
use regex::Regex;

/// ISO 8601 `PnDTnHnMn.nS` with optional per-field signs, as `java.time.Duration.parse` reads it.
static ISO_DURATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^([-+]?)P(?:([-+]?[0-9]+)D)?(T(?:([-+]?[0-9]+)H)?(?:([-+]?[0-9]+)M)?(?:([-+]?[0-9]+)(?:[.,]([0-9]{0,9}))?S)?)?$",
    )
    .expect("valid regex")
});
static WEEKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)W").expect("valid regex"));
static DAYS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)D").expect("valid regex"));
static HOURS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)H").expect("valid regex"));
static MINUTES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)M").expect("valid regex"));
static SECONDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)S").expect("valid regex"));

/// Parse an RFC 5545 or ISO 8601 duration (`-PT15M`, `P1W`, `P1DT2H`).
///
/// Strict ISO parsing is tried first; the fallback mirrors the lenient
/// iCalendar reader of the Kotlin build (weeks, and fields found anywhere in
/// their part). Returns `None` for blank input or input not starting with `P`.
pub fn parse_duration(value: &str) -> Option<TimeDelta> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    parse_iso(trimmed).or_else(|| parse_lenient(trimmed))
}

fn parse_iso(value: &str) -> Option<TimeDelta> {
    let caps = ISO_DURATION.captures(value)?;
    let has_time_part = caps.get(3).is_some();
    let any_field = [2, 4, 5, 6].iter().any(|&i| caps.get(i).is_some());
    let time_fields = [4, 5, 6].iter().any(|&i| caps.get(i).is_some());
    if !any_field || (has_time_part && !time_fields) {
        return None;
    }
    let field = |i: usize| -> Option<i64> { caps.get(i).map_or(Some(0), |m| m.as_str().parse().ok()) };
    let seconds_value = field(6)?;
    let mut total = TimeDelta::try_days(field(2)?)?
        .checked_add(&TimeDelta::try_hours(field(4)?)?)?
        .checked_add(&TimeDelta::try_minutes(field(5)?)?)?
        .checked_add(&TimeDelta::try_seconds(seconds_value)?)?;
    if let Some(fraction) = caps.get(7).filter(|m| !m.as_str().is_empty()) {
        let nanos: i64 = format!("{:0<9}", fraction.as_str()).parse().ok()?;
        let nanos = if caps.get(6)?.as_str().starts_with('-') { -nanos } else { nanos };
        total = total.checked_add(&TimeDelta::nanoseconds(nanos))?;
    }
    Some(if &caps[1] == "-" { -total } else { total })
}

fn parse_lenient(value: &str) -> Option<TimeDelta> {
    let upper = value.to_ascii_uppercase();
    let (negative, rest) = match upper.as_bytes().first() {
        Some(b'-') => (true, &upper[1..]),
        Some(b'+') => (false, &upper[1..]),
        _ => (false, upper.as_str()),
    };
    let body = rest.strip_prefix('P')?;
    let magnitude = if let Some(weeks) = body.strip_suffix('W') {
        TimeDelta::try_days(weeks.parse::<i64>().ok()?.checked_mul(7)?)?
    } else {
        let (date_part, time_part) = body.split_once('T').unwrap_or((body, ""));
        let grab = |re: &Regex, part: &str| -> i64 {
            re.captures(part).and_then(|c| c[1].parse().ok()).unwrap_or(0)
        };
        // Weeks combined with days (`P1W2D`) is ISO 8601, not RFC 5545, but
        // peers emit it and dropping the weeks would shorten the event.
        TimeDelta::try_days(grab(&WEEKS, date_part).checked_mul(7)?.checked_add(grab(&DAYS, date_part))?)?
            .checked_add(&TimeDelta::try_hours(grab(&HOURS, time_part))?)?
            .checked_add(&TimeDelta::try_minutes(grab(&MINUTES, time_part))?)?
            .checked_add(&TimeDelta::try_seconds(grab(&SECONDS, time_part))?)?
    };
    Some(if negative { -magnitude } else { magnitude })
}

/// Format a duration in compact RFC 5545 form: `-PT15M`, `PT1H30M`, `P1D`, `PT0S`.
///
/// Sub-second precision is dropped; days are never folded into weeks.
pub fn format_duration(duration: TimeDelta) -> String {
    let negative = duration < TimeDelta::zero();
    let total_seconds = duration.num_seconds().unsigned_abs();
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push('P');
    if days > 0 {
        out.push_str(&format!("{days}D"));
    }
    if hours > 0 || minutes > 0 || seconds > 0 {
        out.push('T');
        if hours > 0 {
            out.push_str(&format!("{hours}H"));
        }
        if minutes > 0 {
            out.push_str(&format!("{minutes}M"));
        }
        if seconds > 0 {
            out.push_str(&format!("{seconds}S"));
        }
    }
    if out.len() <= 2 {
        out.push_str("T0S");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_duration("-PT15M"), Some(TimeDelta::minutes(-15)));
        assert_eq!(parse_duration("PT1H30M"), Some(TimeDelta::minutes(90)));
        assert_eq!(parse_duration("P1D"), Some(TimeDelta::days(1)));
        assert_eq!(parse_duration("-P1DT2H"), Some(-(TimeDelta::days(1) + TimeDelta::hours(2))));
        assert_eq!(parse_duration("P1W"), Some(TimeDelta::days(7)));
        assert_eq!(parse_duration("-P2W"), Some(TimeDelta::days(-14)));
        assert_eq!(parse_duration("+PT5M"), Some(TimeDelta::minutes(5)));
        assert_eq!(parse_duration("PT0S"), Some(TimeDelta::zero()));
        assert_eq!(parse_duration("PT1.5S"), Some(TimeDelta::milliseconds(1500)));
        assert_eq!(parse_duration("P1W2D"), Some(TimeDelta::days(9)));
        assert_eq!(parse_duration("-P1W2DT3H"), Some(-(TimeDelta::days(9) + TimeDelta::hours(3))));
    }

    #[test]
    fn rejects_non_durations() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("   "), None);
        assert_eq!(parse_duration("15M"), None);
        assert_eq!(parse_duration("20260115T093000Z"), None);
    }

    #[test]
    fn formats_compactly() {
        assert_eq!(format_duration(TimeDelta::minutes(-15)), "-PT15M");
        assert_eq!(format_duration(TimeDelta::minutes(90)), "PT1H30M");
        assert_eq!(format_duration(TimeDelta::days(1)), "P1D");
        assert_eq!(format_duration(TimeDelta::days(7)), "P7D");
        assert_eq!(format_duration(TimeDelta::zero()), "PT0S");
        assert_eq!(format_duration(TimeDelta::days(1) + TimeDelta::seconds(5)), "P1DT5S");
        assert_eq!(format_duration(TimeDelta::days(-2)), "-P2D");
    }

    #[test]
    fn round_trips() {
        for s in ["-PT15M", "PT1H30M", "P1D", "-P1DT2H", "PT45S", "P3DT4H5M6S"] {
            assert_eq!(format_duration(parse_duration(s).unwrap()), s);
        }
    }
}
