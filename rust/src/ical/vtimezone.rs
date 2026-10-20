//! VTIMEZONE generation from the IANA database.
//!
//! chrono-tz exposes offsets, not transition rules, so the rules are recovered
//! from observed transitions: each transition in the sample years becomes a
//! STANDARD or DAYLIGHT sub-component with a yearly RRULE (nth or last weekday
//! of the month when that holds every sampled year, else a fixed day).

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone, Utc, Weekday};
use chrono_tz::{OffsetName, Tz};

use super::content::push_line;
use super::rrule::weekday_to_ical;
use super::timezone::parse_iana;

/// TZIDs that denote UTC and never need a VTIMEZONE.
pub const UTC_TZIDS: [&str; 4] = ["UTC", "Z", "Etc/UTC", "GMT"];

/// How many consecutive years are sampled to recognize a transition rule.
const SAMPLE_YEARS: i32 = 6;

/// One observed offset change.
#[derive(Debug, Clone)]
struct Transition {
    /// Wall-clock time just before the change, in the old offset.
    local_before: NaiveDateTime,
    offset_before: i32,
    offset_after: i32,
    name_after: String,
}

impl Transition {
    fn is_daylight(&self) -> bool {
        self.offset_after > self.offset_before
    }
}

/// The VTIMEZONE block for `tzid`, or an empty string for UTC and unknown ids.
pub fn generate_vtimezone(tzid: &str) -> String {
    if UTC_TZIDS.contains(&tzid) {
        return String::new();
    }
    let Some(zone) = parse_iana(tzid) else {
        return String::new();
    };
    let first_year = Utc::now().year();
    let by_year: Vec<Vec<Transition>> =
        (first_year..first_year + SAMPLE_YEARS).map(|y| transitions_in(zone, y)).collect();

    let mut out = String::new();
    push_line(&mut out, "BEGIN:VTIMEZONE");
    push_line(&mut out, &format!("TZID:{tzid}"));
    if by_year[0].is_empty() {
        push_fixed_component(&mut out, zone, tzid);
    } else {
        for (index, transition) in by_year[0].iter().enumerate() {
            let samples: Vec<&Transition> = by_year.iter().filter_map(|year| year.get(index)).collect();
            push_rule_component(&mut out, transition, &samples);
        }
    }
    push_line(&mut out, "END:VTIMEZONE");
    out
}

fn push_fixed_component(out: &mut String, zone: Tz, tzid: &str) {
    let offset = zone.offset_from_utc_datetime(&Utc::now().naive_utc());
    let seconds = offset.fix().local_minus_utc();
    let name = offset
        .abbreviation()
        .map(str::to_string)
        .unwrap_or_else(|| tzid.rsplit('/').next().unwrap_or(tzid).chars().take(4).collect::<String>().to_uppercase());
    push_line(out, "BEGIN:STANDARD");
    push_line(out, "DTSTART:19700101T000000");
    push_line(out, &format!("TZOFFSETFROM:{}", format_offset(seconds)));
    push_line(out, &format!("TZOFFSETTO:{}", format_offset(seconds)));
    push_line(out, &format!("TZNAME:{name}"));
    push_line(out, "END:STANDARD");
}

fn push_rule_component(out: &mut String, transition: &Transition, samples: &[&Transition]) {
    let kind = if transition.is_daylight() { "DAYLIGHT" } else { "STANDARD" };
    let rule = YearlyRule::recognize(samples);
    let month = transition.local_before.month();
    let time = transition.local_before.time();
    let start_day = rule.day_in(1970, month).unwrap_or(1);
    push_line(out, &format!("BEGIN:{kind}"));
    push_line(out, &format!("DTSTART:1970{month:02}{start_day:02}T{}", time.format("%H%M%S")));
    push_line(out, &format!("RRULE:FREQ=YEARLY;BYMONTH={month};{}", rule.by_part()));
    push_line(out, &format!("TZOFFSETFROM:{}", format_offset(transition.offset_before)));
    push_line(out, &format!("TZOFFSETTO:{}", format_offset(transition.offset_after)));
    push_line(out, &format!("TZNAME:{}", transition.name_after));
    push_line(out, &format!("END:{kind}"));
}

/// How a yearly transition picks its day within the month.
enum YearlyRule {
    /// The nth (1-based) weekday of the month, or the last when `n == -1`.
    NthWeekday { n: i32, weekday: Weekday },
    /// A fixed day of the month.
    MonthDay(u32),
}

impl YearlyRule {
    fn recognize(samples: &[&Transition]) -> YearlyRule {
        let dates: Vec<NaiveDate> = samples.iter().map(|t| t.local_before.date()).collect();
        let first = dates[0];
        let same_weekday = dates.iter().all(|d| d.weekday() == first.weekday());
        if same_weekday && dates.iter().all(|d| is_last_week(*d)) {
            return YearlyRule::NthWeekday { n: -1, weekday: first.weekday() };
        }
        let nth = |d: &NaiveDate| (d.day() as i32 - 1) / 7 + 1;
        if same_weekday && dates.iter().all(|d| nth(d) == nth(&first)) {
            return YearlyRule::NthWeekday { n: nth(&first), weekday: first.weekday() };
        }
        if dates.iter().all(|d| d.day() == first.day()) {
            return YearlyRule::MonthDay(first.day());
        }
        YearlyRule::NthWeekday { n: nth(&first), weekday: first.weekday() }
    }

    fn by_part(&self) -> String {
        match self {
            YearlyRule::NthWeekday { n, weekday } => format!("BYDAY={n}{}", weekday_to_ical(*weekday)),
            YearlyRule::MonthDay(day) => format!("BYMONTHDAY={day}"),
        }
    }

    /// The day this rule selects in `year`/`month`.
    fn day_in(&self, year: i32, month: u32) -> Option<u32> {
        match self {
            YearlyRule::MonthDay(day) => Some(*day),
            YearlyRule::NthWeekday { n, weekday } => {
                let first = NaiveDate::from_ymd_opt(year, month, 1)?;
                let last = last_day_of_month(year, month)?;
                if *n == -1 {
                    let back = (7 + last.weekday().num_days_from_monday() - weekday.num_days_from_monday()) % 7;
                    Some(last.day() - back)
                } else {
                    let ahead = (7 + weekday.num_days_from_monday() - first.weekday().num_days_from_monday()) % 7;
                    let day = 1 + ahead + (*n as u32 - 1) * 7;
                    (day <= last.day()).then_some(day)
                }
            }
        }
    }
}

fn last_day_of_month(year: i32, month: u32) -> Option<NaiveDate> {
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()
}

fn is_last_week(date: NaiveDate) -> bool {
    last_day_of_month(date.year(), date.month()).is_some_and(|last| date.day() + 7 > last.day())
}

/// Every offset change in `year`, in order.
fn transitions_in(zone: Tz, year: i32) -> Vec<Transition> {
    let bounds =
        (Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single(), Utc.with_ymd_and_hms(year + 1, 1, 1, 0, 0, 0).single());
    let (Some(start), Some(end)) = bounds else {
        return Vec::new();
    };
    let offset_at = |t: DateTime<Utc>| zone.offset_from_utc_datetime(&t.naive_utc()).fix().local_minus_utc();
    let mut transitions = Vec::new();
    let mut previous = start;
    while previous < end {
        let current = previous + TimeDelta::days(1);
        if offset_at(previous) != offset_at(current) {
            let instant = first_change(previous, current, offset_at);
            let before = offset_at(instant - TimeDelta::seconds(1));
            let after = zone.offset_from_utc_datetime(&instant.naive_utc());
            let after_seconds = after.fix().local_minus_utc();
            let local_before = instant.naive_utc() + TimeDelta::seconds(i64::from(before));
            if local_before.year() == year {
                transitions.push(Transition {
                    local_before,
                    offset_before: before,
                    offset_after: after_seconds,
                    name_after: after.abbreviation().map_or_else(|| format_offset(after_seconds), str::to_string),
                });
            }
        }
        previous = current;
    }
    transitions
}

/// The first whole minute in `(low, high]` whose offset differs from `low`'s.
fn first_change(low: DateTime<Utc>, high: DateTime<Utc>, offset_at: impl Fn(DateTime<Utc>) -> i32) -> DateTime<Utc> {
    let original = offset_at(low);
    let (mut lo, mut hi) = (0i64, (high - low).num_minutes());
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if offset_at(low + TimeDelta::minutes(mid)) == original {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    low + TimeDelta::minutes(hi)
}

/// `±HHMM` (seconds dropped).
pub fn format_offset(total_seconds: i32) -> String {
    let sign = if total_seconds >= 0 { '+' } else { '-' };
    let abs = total_seconds.unsigned_abs();
    format!("{sign}{:02}{:02}", abs / 3600, (abs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_and_unknown_produce_nothing() {
        for id in ["UTC", "Z", "Etc/UTC", "GMT", "Not/AZone"] {
            assert_eq!(generate_vtimezone(id), "");
        }
    }

    #[test]
    fn new_york_has_us_rules() {
        let block = generate_vtimezone("America/New_York");
        assert!(block.starts_with("BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\n"));
        assert!(block.contains("BEGIN:DAYLIGHT\r\nDTSTART:19700308T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nTZNAME:EDT\r\nEND:DAYLIGHT"), "{block}");
        assert!(block.contains("BEGIN:STANDARD\r\nDTSTART:19701101T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nTZNAME:EST\r\nEND:STANDARD"), "{block}");
        assert!(block.ends_with("END:VTIMEZONE\r\n"));
    }

    #[test]
    fn berlin_uses_last_sunday_at_local_wall_time() {
        let block = generate_vtimezone("Europe/Berlin");
        assert!(block.contains("DTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200"), "{block}");
        assert!(block.contains("DTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100"), "{block}");
    }

    #[test]
    fn fixed_offset_zone_has_single_standard_block() {
        let block = generate_vtimezone("Asia/Tokyo");
        assert!(block.contains("BEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0900\r\nTZOFFSETTO:+0900\r\nTZNAME:JST\r\nEND:STANDARD"), "{block}");
        assert!(!block.contains("DAYLIGHT"));
    }

    #[test]
    fn southern_hemisphere_and_half_hour_offsets() {
        let sydney = generate_vtimezone("Australia/Sydney");
        assert!(sydney.contains("BYMONTH=4;BYDAY=1SU") && sydney.contains("BYMONTH=10;BYDAY=1SU"), "{sydney}");
        let kolkata = generate_vtimezone("Asia/Kolkata");
        assert!(kolkata.contains("TZOFFSETTO:+0530"), "{kolkata}");
    }

    #[test]
    fn offsets_format_with_sign() {
        assert_eq!(format_offset(-5 * 3600), "-0500");
        assert_eq!(format_offset(5 * 3600 + 1800), "+0530");
        assert_eq!(format_offset(0), "+0000");
    }
}
