//! Patterns shared by the input validator and the ICS writers, so the boundary
//! check and the emitter agree on what each input shape means.
//!
//! All patterns are anchored and free of nested quantifiers; the `regex` crate
//! also guarantees linear-time matching, so none of them can backtrack.

use std::sync::LazyLock;

use regex::Regex;

/// A VALARM TRIGGER given as an absolute basic-format UTC instant
/// (`20260115T093000Z`).
pub static ICAL_ABSOLUTE_TRIGGER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{8}T\d{6}Z$").expect("valid regex"));

/// A trailing numeric offset (`+HH:MM` / `-HH:MM`) on an ISO 8601 datetime.
pub static OFFSET_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[+-]\d{2}:\d{2}$").expect("valid regex"));

/// True when an ISO 8601 datetime carries an explicit zone: a UTC `Z` or a
/// numeric offset.
pub fn has_explicit_zone(datetime: &str) -> bool {
    datetime.ends_with('Z') || OFFSET_SUFFIX.is_match(datetime)
}
