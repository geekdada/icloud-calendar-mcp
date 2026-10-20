//! RFC 5545 §3.3.10 RRULE values.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use chrono::Weekday;
use regex::Regex;

use super::ICalError;
use super::datetime::ICalDateTime;

/// Recurrence frequency, ordered from finest to coarsest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Frequency {
    Secondly,
    Minutely,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Frequency {
    pub fn as_str(self) -> &'static str {
        match self {
            Frequency::Secondly => "SECONDLY",
            Frequency::Minutely => "MINUTELY",
            Frequency::Hourly => "HOURLY",
            Frequency::Daily => "DAILY",
            Frequency::Weekly => "WEEKLY",
            Frequency::Monthly => "MONTHLY",
            Frequency::Yearly => "YEARLY",
        }
    }
}

impl FromStr for Frequency {
    type Err = ICalError;

    /// Exact upper-case names only, as the Kotlin model required.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "SECONDLY" => Frequency::Secondly,
            "MINUTELY" => Frequency::Minutely,
            "HOURLY" => Frequency::Hourly,
            "DAILY" => Frequency::Daily,
            "WEEKLY" => Frequency::Weekly,
            "MONTHLY" => Frequency::Monthly,
            "YEARLY" => Frequency::Yearly,
            other => return Err(ICalError::new(format!("Unknown FREQ: {other}"))),
        })
    }
}

/// A BYDAY entry: a weekday with an optional ordinal (`MO`, `2TU`, `-1FR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeekdayNum {
    pub weekday: Weekday,
    pub ordinal: Option<i32>,
}

static WEEKDAY_NUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([+-]?\d{1,2})?([A-Z]{2})$").expect("valid regex"));

impl WeekdayNum {
    pub fn parse(value: &str) -> Result<Self, ICalError> {
        let upper = value.to_ascii_uppercase();
        let caps = WEEKDAY_NUM.captures(&upper).ok_or_else(|| ICalError::new(format!("Invalid weekday: {value}")))?;
        let weekday = weekday_from_ical(&caps[2]).ok_or_else(|| ICalError::new(format!("Invalid weekday: {value}")))?;
        let ordinal = caps.get(1).and_then(|m| m.as_str().parse().ok());
        Ok(Self { weekday, ordinal })
    }
}

impl fmt::Display for WeekdayNum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ordinal {
            Some(n) => write!(f, "{n}{}", weekday_to_ical(self.weekday)),
            None => f.write_str(weekday_to_ical(self.weekday)),
        }
    }
}

pub fn weekday_to_ical(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

pub fn weekday_from_ical(value: &str) -> Option<Weekday> {
    Some(match value.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// A parsed recurrence rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RRule {
    pub freq: Frequency,
    pub interval: i32,
    pub count: Option<i32>,
    pub until: Option<ICalDateTime>,
    pub by_day: Option<Vec<WeekdayNum>>,
    pub by_month_day: Option<Vec<i32>>,
    pub by_month: Option<Vec<i32>>,
    pub by_week_no: Option<Vec<i32>>,
    pub by_year_day: Option<Vec<i32>>,
    pub by_hour: Option<Vec<i32>>,
    pub by_minute: Option<Vec<i32>>,
    pub by_second: Option<Vec<i32>>,
    pub by_set_pos: Option<Vec<i32>>,
    pub wkst: Weekday,
}

static RRULE_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Z]+)=([^;]+)").expect("valid regex"));

impl RRule {
    /// A rule with only `freq` set.
    pub fn new(freq: Frequency) -> Self {
        Self {
            freq,
            interval: 1,
            count: None,
            until: None,
            by_day: None,
            by_month_day: None,
            by_month: None,
            by_week_no: None,
            by_year_day: None,
            by_hour: None,
            by_minute: None,
            by_second: None,
            by_set_pos: None,
            wkst: Weekday::Mon,
        }
    }

    /// Parse an RRULE value (without the `RRULE:` prefix).
    ///
    /// Unparseable numbers inside a BY-list are skipped; a missing or unknown
    /// FREQ, a malformed UNTIL or a malformed BYDAY entry is an error.
    pub fn parse(value: &str) -> Result<Self, ICalError> {
        let mut parts: Vec<(String, String)> = Vec::new();
        for caps in RRULE_PART.captures_iter(value) {
            let key = caps[1].to_string();
            let val = caps[2].to_string();
            match parts.iter_mut().find(|(k, _)| *k == key) {
                Some(existing) => existing.1 = val,
                None => parts.push((key, val)),
            }
        }
        let get = |key: &str| parts.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let freq: Frequency =
            get("FREQ").ok_or_else(|| ICalError::new(format!("RRULE missing FREQ: {value}")))?.parse()?;
        let ints = |key: &str| -> Option<Vec<i32>> {
            get(key).map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        };
        let mut r = RRule::new(freq);
        r.interval = get("INTERVAL").and_then(|v| v.parse().ok()).unwrap_or(1);
        r.count = get("COUNT").and_then(|v| v.parse().ok());
        r.until = get("UNTIL").map(|v| ICalDateTime::parse(v, None)).transpose()?;
        r.by_day =
            get("BYDAY").map(|v| v.split(',').map(WeekdayNum::parse).collect::<Result<Vec<_>, _>>()).transpose()?;
        r.by_month_day = ints("BYMONTHDAY");
        r.by_month = ints("BYMONTH");
        r.by_week_no = ints("BYWEEKNO");
        r.by_year_day = ints("BYYEARDAY");
        r.by_hour = ints("BYHOUR");
        r.by_minute = ints("BYMINUTE");
        r.by_second = ints("BYSECOND");
        r.by_set_pos = ints("BYSETPOS");
        r.wkst = get("WKST").and_then(weekday_from_ical).unwrap_or(Weekday::Mon);
        Ok(r)
    }

    /// The RRULE value in canonical order; defaults (INTERVAL=1, WKST=MO) are omitted.
    pub fn to_ical_string(&self) -> String {
        let mut parts = vec![format!("FREQ={}", self.freq.as_str())];
        if self.interval != 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if let Some(count) = self.count {
            parts.push(format!("COUNT={count}"));
        }
        if let Some(until) = &self.until {
            parts.push(format!("UNTIL={}", until.to_ical_string()));
        }
        if let Some(days) = &self.by_day {
            let joined: Vec<String> = days.iter().map(ToString::to_string).collect();
            parts.push(format!("BYDAY={}", joined.join(",")));
        }
        let mut push_list = |name: &str, list: &Option<Vec<i32>>| {
            if let Some(values) = list {
                let joined: Vec<String> = values.iter().map(ToString::to_string).collect();
                parts.push(format!("{name}={}", joined.join(",")));
            }
        };
        push_list("BYMONTHDAY", &self.by_month_day);
        push_list("BYMONTH", &self.by_month);
        push_list("BYWEEKNO", &self.by_week_no);
        push_list("BYYEARDAY", &self.by_year_day);
        push_list("BYSECOND", &self.by_second);
        push_list("BYMINUTE", &self.by_minute);
        push_list("BYHOUR", &self.by_hour);
        push_list("BYSETPOS", &self.by_set_pos);
        if self.wkst != Weekday::Mon {
            parts.push(format!("WKST={}", weekday_to_ical(self.wkst)));
        }
        parts.join(";")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_round_trip() {
        for rule in [
            "FREQ=DAILY",
            "FREQ=WEEKLY;BYDAY=MO,WE,FR",
            "FREQ=MONTHLY;BYDAY=2TU",
            "FREQ=MONTHLY;BYDAY=-1FR",
            "FREQ=YEARLY;BYMONTHDAY=25;BYMONTH=12",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=SU,MO;WKST=SU",
            "FREQ=DAILY;COUNT=10",
            "FREQ=DAILY;UNTIL=20261231T235959Z",
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            "FREQ=HOURLY;BYMINUTE=0,30;BYHOUR=9,10",
        ] {
            assert_eq!(RRule::parse(rule).unwrap().to_ical_string(), rule);
        }
    }

    #[test]
    fn parses_fields() {
        let r = RRule::parse("FREQ=WEEKLY;INTERVAL=2;COUNT=5;BYDAY=+1MO,53SU;WKST=SU").unwrap();
        assert_eq!(r.freq, Frequency::Weekly);
        assert_eq!(r.interval, 2);
        assert_eq!(r.count, Some(5));
        assert_eq!(r.wkst, Weekday::Sun);
        let days = r.by_day.unwrap();
        assert_eq!(days[0], WeekdayNum { weekday: Weekday::Mon, ordinal: Some(1) });
        assert_eq!(days[1], WeekdayNum { weekday: Weekday::Sun, ordinal: Some(53) });
    }

    #[test]
    fn rejects_missing_or_unknown_freq() {
        assert!(RRule::parse("INTERVAL=2").is_err());
        assert!(RRule::parse("FREQ=FORTNIGHTLY").is_err());
        assert!(RRule::parse("FREQ=WEEKLY;BYDAY=XX").is_err());
        assert!(RRule::parse("").is_err());
    }

    #[test]
    fn skips_bad_numbers_in_lists() {
        let r = RRule::parse("FREQ=MONTHLY;BYMONTHDAY=1,x,15").unwrap();
        assert_eq!(r.by_month_day, Some(vec![1, 15]));
    }
}
