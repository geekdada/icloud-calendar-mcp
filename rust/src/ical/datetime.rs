//! iCalendar DATE / DATE-TIME values that keep their time zone.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use super::ICalError;
use super::timezone::{local_to_timestamp, resolve_tzid, system_zone};

/// A DATE or DATE-TIME value as it appears in iCalendar data.
///
/// Four forms are distinguished:
/// - UTC (`20231215T140000Z`): `is_utc`, no `timezone`.
/// - Zoned local (`TZID=America/New_York:20231215T140000`): `timezone` set.
/// - Floating (`20231215T140000`): resolved in the system zone, which is then
///   recorded as `timezone`.
/// - DATE (`20231215`): `is_date`, stored as UTC midnight so the calendar date
///   is the same on every host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ICalDateTime {
    /// Epoch milliseconds.
    pub timestamp: i64,
    /// Zone for local forms; `None` for UTC and DATE values.
    pub timezone: Option<Tz>,
    /// True when written in UTC (`Z`) form; DATE values are stored as UTC too.
    pub is_utc: bool,
    /// True for DATE (all-day) values.
    pub is_date: bool,
}

const BASIC_DATE_TIME: &str = "%Y%m%dT%H%M%S";

impl ICalDateTime {
    /// Parse an iCalendar date or date-time, resolving a local form in `tzid`
    /// (or the system zone when no TZID is given).
    pub fn parse(value: &str, tzid: Option<&str>) -> Result<Self, ICalError> {
        let trimmed = value.trim();
        if let Some(utc) = trimmed.strip_suffix('Z') {
            let local =
                parse_basic_date_time(utc).ok_or_else(|| ICalError::new(format!("Invalid UTC datetime: {value}")))?;
            return Ok(Self::utc(local.and_utc().timestamp_millis()));
        }
        if trimmed.len() == 8 && trimmed.bytes().all(|b| b.is_ascii_digit()) {
            let date = NaiveDate::parse_from_str(trimmed, "%Y%m%d")
                .map_err(|_| ICalError::new(format!("Invalid iCalendar date: {value}")))?;
            return Ok(Self::from_local_date(date));
        }
        if let Some(local) = parse_basic_date_time(trimmed) {
            let zone = tzid.map(resolve_tzid).unwrap_or_else(system_zone);
            return Ok(Self {
                timestamp: local_to_timestamp(local, zone),
                timezone: Some(zone),
                is_utc: false,
                is_date: false,
            });
        }
        Err(ICalError::new(format!("Invalid iCalendar datetime format: {value}")))
    }

    /// The current instant, in UTC form.
    pub fn now() -> Self {
        Self::utc(Utc::now().timestamp_millis())
    }

    /// A UTC DATE-TIME at `timestamp`.
    pub fn utc(timestamp: i64) -> Self {
        Self { timestamp, timezone: None, is_utc: true, is_date: false }
    }

    /// A value at `timestamp`; UTC form when `timezone` is `None`.
    pub fn from_timestamp(timestamp: i64, timezone: Option<Tz>, is_date: bool) -> Self {
        Self { timestamp, timezone, is_utc: timezone.is_none(), is_date }
    }

    /// A DATE value for `date`, stored as UTC midnight.
    pub fn from_local_date(date: NaiveDate) -> Self {
        Self {
            timestamp: date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp_millis(),
            timezone: None,
            is_utc: true,
            is_date: true,
        }
    }

    /// A zoned local DATE-TIME at the instant of `dt`, or the DATE of its local
    /// calendar day when `is_date`.
    pub fn from_zoned(dt: DateTime<Tz>, is_date: bool) -> Self {
        if is_date {
            return Self::from_local_date(dt.date_naive());
        }
        Self { timestamp: dt.timestamp_millis(), timezone: Some(dt.timezone()), is_utc: false, is_date: false }
    }

    /// The zone this value's wall-clock fields are read in: UTC for DATE and
    /// UTC-form values, else the recorded zone (the system zone if none).
    pub fn zone(&self) -> Tz {
        if self.is_date || self.is_utc { Tz::UTC } else { self.timezone.unwrap_or_else(system_zone) }
    }

    /// This instant with the wall-clock fields of [`zone`](Self::zone).
    pub fn to_zoned(&self) -> DateTime<Tz> {
        self.zone()
            .timestamp_millis_opt(self.timestamp)
            .single()
            .unwrap_or_else(|| Tz::UTC.timestamp_millis_opt(0).single().expect("epoch is representable"))
    }

    /// The instant in UTC.
    pub fn to_utc(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.timestamp).unwrap_or_default()
    }

    /// The calendar date in [`zone`](Self::zone).
    pub fn to_local_date(&self) -> NaiveDate {
        self.to_zoned().date_naive()
    }

    /// The wall-clock date-time in [`zone`](Self::zone).
    pub fn to_local_date_time(&self) -> NaiveDateTime {
        self.to_zoned().naive_local()
    }

    /// `YYYYMMDD` of the local calendar date; used to match EXDATEs and RDATEs by day.
    pub fn to_day_code(&self) -> String {
        self.to_local_date().format("%Y%m%d").to_string()
    }

    /// The iCalendar wire form: `YYYYMMDD`, `YYYYMMDDTHHMMSSZ`, or local `YYYYMMDDTHHMMSS`.
    pub fn to_ical_string(&self) -> String {
        if self.is_date {
            self.to_local_date().format("%Y%m%d").to_string()
        } else if self.is_utc {
            self.to_utc().format("%Y%m%dT%H%M%SZ").to_string()
        } else {
            self.to_zoned().format(BASIC_DATE_TIME).to_string()
        }
    }

    /// The TZID to emit for this value, if it is a zoned local DATE-TIME.
    pub fn tzid(&self) -> Option<&'static str> {
        if self.is_date || self.is_utc { None } else { self.timezone.map(|tz| tz.name()) }
    }
}

/// `YYYYMMDDTHHMMSS` with exactly that shape and valid field ranges.
fn parse_basic_date_time(value: &str) -> Option<NaiveDateTime> {
    let bytes = value.as_bytes();
    let shaped = bytes.len() == 15
        && bytes[8] == b'T'
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..].iter().all(u8::is_ascii_digit);
    if !shaped {
        return None;
    }
    NaiveDateTime::parse_from_str(value, BASIC_DATE_TIME).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> i64 {
        DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    #[test]
    fn parses_utc_form() {
        let dt = ICalDateTime::parse("20231215T140000Z", None).unwrap();
        assert_eq!(dt.timestamp, ts("2023-12-15T14:00:00Z"));
        assert!(dt.is_utc && !dt.is_date && dt.timezone.is_none());
        assert_eq!(dt.to_ical_string(), "20231215T140000Z");
    }

    #[test]
    fn parses_date_form_as_utc_midnight() {
        let dt = ICalDateTime::parse("20260123", None).unwrap();
        assert_eq!(dt.timestamp, ts("2026-01-23T00:00:00Z"));
        assert!(dt.is_date && dt.is_utc);
        assert_eq!(dt.to_local_date(), NaiveDate::from_ymd_opt(2026, 1, 23).unwrap());
        assert_eq!(dt.to_ical_string(), "20260123");
        assert_eq!(dt.to_day_code(), "20260123");
    }

    #[test]
    fn parses_zoned_local_form() {
        let dt = ICalDateTime::parse("20231215T090000", Some("America/New_York")).unwrap();
        assert_eq!(dt.timestamp, ts("2023-12-15T14:00:00Z"));
        assert_eq!(dt.timezone, Some(Tz::America__New_York));
        assert!(!dt.is_utc);
        assert_eq!(dt.to_ical_string(), "20231215T090000");
        assert_eq!(dt.tzid(), Some("America/New_York"));
    }

    #[test]
    fn floating_form_uses_system_zone() {
        let dt = ICalDateTime::parse("20231215T090000", None).unwrap();
        assert_eq!(dt.timezone, Some(system_zone()));
        assert_eq!(dt.to_ical_string(), "20231215T090000");
    }

    #[test]
    fn rejects_malformed_values() {
        for bad in ["", "2023-12-15", "20231315T000000Z", "20231215T2500", "abcdefgh", "20231215T090000ZZ"] {
            assert!(ICalDateTime::parse(bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn utc_value_keeps_utc_date_regardless_of_zone() {
        let dt = ICalDateTime::parse("20260123T000000Z", None).unwrap();
        assert_eq!(dt.to_local_date(), NaiveDate::from_ymd_opt(2026, 1, 23).unwrap());
    }

    #[test]
    fn from_timestamp_marks_utc_without_zone() {
        let dt = ICalDateTime::from_timestamp(0, None, false);
        assert!(dt.is_utc);
        assert_eq!(dt.to_ical_string(), "19700101T000000Z");
        let zoned = ICalDateTime::from_timestamp(0, Some(Tz::Asia__Tokyo), false);
        assert!(!zoned.is_utc);
        assert_eq!(zoned.to_ical_string(), "19700101T090000");
    }
}
