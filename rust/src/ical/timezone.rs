//! Time zone resolution: TZID lookup and local wall-clock to instant conversion.

use std::sync::OnceLock;

use chrono::{LocalResult, NaiveDateTime, Offset, TimeDelta, TimeZone};
use chrono_tz::Tz;

use super::windows_zones::WINDOWS_ZONE_ALIASES;

/// The host's time zone, used for floating (zone-less) local times.
///
/// Read once from `TZ`, then the OS setting; UTC when neither names a known zone.
pub fn system_zone() -> Tz {
    static ZONE: OnceLock<Tz> = OnceLock::new();
    *ZONE.get_or_init(detect_system_zone)
}

fn detect_system_zone() -> Tz {
    std::env::var("TZ")
        .ok()
        .and_then(|name| parse_iana(name.trim_start_matches(':')))
        .or_else(|| iana_time_zone::get_timezone().ok().and_then(|name| parse_iana(&name)))
        .unwrap_or(Tz::UTC)
}

/// The IANA zone named exactly `id`, if it exists.
pub fn parse_iana(id: &str) -> Option<Tz> {
    id.parse::<Tz>().ok()
}

/// Resolve a TZID parameter: an IANA id, else a Windows zone name
/// (case-insensitive), else the system zone. Never fails, matching how the
/// Kotlin build resolved TZIDs.
pub fn resolve_tzid(tzid: &str) -> Tz {
    let tzid = tzid.trim();
    if let Some(zone) = parse_iana(tzid) {
        return zone;
    }
    WINDOWS_ZONE_ALIASES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(tzid))
        .and_then(|(_, iana)| parse_iana(iana))
        .unwrap_or_else(system_zone)
}

/// The instant (epoch millis) a local wall-clock time denotes in `zone`.
///
/// Follows `java.time` resolution so stored instants match the Kotlin build:
/// an ambiguous time (DST overlap) takes the earlier offset, and a time inside a
/// DST gap is pushed forward by the gap's length.
pub fn local_to_timestamp(local: NaiveDateTime, zone: Tz) -> i64 {
    match zone.from_local_datetime(&local) {
        LocalResult::Single(dt) => dt.timestamp_millis(),
        LocalResult::Ambiguous(earliest, _) => earliest.timestamp_millis(),
        LocalResult::None => {
            // In a gap: interpret the wall time with the offset in force just
            // before the transition, which lands it the gap's length later.
            let before = local - TimeDelta::days(1);
            let offset = zone.offset_from_utc_datetime(&before).fix();
            (local - TimeDelta::seconds(i64::from(offset.local_minus_utc()))).and_utc().timestamp_millis()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn resolves_iana_and_windows_names() {
        assert_eq!(resolve_tzid("America/New_York"), Tz::America__New_York);
        assert_eq!(resolve_tzid("Eastern Standard Time").name(), "US/Eastern");
        assert_eq!(resolve_tzid("eastern standard time").name(), "US/Eastern");
    }

    #[test]
    fn unknown_tzid_falls_back_to_system_zone() {
        assert_eq!(resolve_tzid("Not/AZone"), system_zone());
    }

    #[test]
    fn plain_local_time() {
        let ts = local_to_timestamp(local(2024, 1, 15, 9, 0), Tz::America__New_York);
        assert_eq!(ts, local(2024, 1, 15, 14, 0).and_utc().timestamp_millis());
    }

    #[test]
    fn gap_time_moves_forward() {
        // 02:30 on 2024-03-10 does not exist in New York; java.time yields 03:30 EDT (07:30Z).
        let ts = local_to_timestamp(local(2024, 3, 10, 2, 30), Tz::America__New_York);
        assert_eq!(ts, local(2024, 3, 10, 7, 30).and_utc().timestamp_millis());
    }

    #[test]
    fn overlap_time_takes_earlier_offset() {
        // 01:30 on 2024-11-03 happens twice; the earlier one is EDT (05:30Z).
        let ts = local_to_timestamp(local(2024, 11, 3, 1, 30), Tz::America__New_York);
        assert_eq!(ts, local(2024, 11, 3, 5, 30).and_utc().timestamp_millis());
    }
}
