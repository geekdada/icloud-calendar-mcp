//! Builds the ICS body of a new event from tool inputs.

use chrono::{DateTime, TimeDelta, Utc};

use super::{parse_date, recurrence_date, resolve_timed};
use crate::ical::duration::parse_duration;
use crate::ical::generator::generate_calendar;
use crate::ical::{AlarmAction, EventStatus, ICalAlarm, ICalDateTime, ICalError, ICalEvent, RRule, Transparency};
use crate::patterns::ICAL_ABSOLUTE_TRIGGER;

/// A VALARM as given on `create_event` / `update_event` (RFC 5545 §3.6.6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlarmSpec {
    /// A duration (`-PT15M`) or a basic-format UTC instant (`20260115T093000Z`).
    pub trigger: String,
    /// DISPLAY (default), AUDIO or EMAIL.
    pub action: Option<String>,
    /// Defaults to "Reminder" for DISPLAY/EMAIL on output.
    pub description: Option<String>,
    /// Subject line; EMAIL only.
    pub summary: Option<String>,
    /// Emitted only together with `repeat_duration`.
    pub repeat_count: Option<i32>,
    pub repeat_duration: Option<String>,
}

impl AlarmSpec {
    /// The VALARM for this spec. REPEAT/DURATION are written only as a pair.
    pub fn to_ical_alarm(&self) -> ICalAlarm {
        let mut alarm = ICalAlarm::new(AlarmAction::from_value(self.action.as_deref()));
        if ICAL_ABSOLUTE_TRIGGER.is_match(&self.trigger) {
            alarm.trigger_absolute = ICalDateTime::parse(&self.trigger, None).ok();
        } else {
            alarm.trigger = parse_duration(&self.trigger);
        }
        alarm.description = self.description.clone().filter(|d| !d.trim().is_empty());
        alarm.summary = self.summary.clone().filter(|s| !s.trim().is_empty());
        let repeat_duration = self.repeat_duration.as_deref().filter(|d| !d.trim().is_empty()).and_then(parse_duration);
        if let (Some(count), Some(duration)) = (self.repeat_count.filter(|c| *c > 0), repeat_duration) {
            alarm.repeat_count = count;
            alarm.repeat_duration = Some(duration);
        }
        alarm
    }
}

/// Inputs for a new event. Timed events use `start_time`/`end_time`; all-day
/// events use `start_date`/`end_date` (inclusive) with `is_all_day`.
#[derive(Debug, Clone, Default)]
pub struct NewEvent {
    /// Generated when absent.
    pub uid: Option<String>,
    pub summary: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub is_all_day: bool,
    pub description: Option<String>,
    pub location: Option<String>,
    /// IANA zone for naive timed values.
    pub timezone: Option<String>,
    /// IANA zone for a naive `end_time` when it differs (e.g. a flight).
    pub end_timezone: Option<String>,
    pub rrule: Option<String>,
    pub status: Option<String>,
    pub url: Option<String>,
    pub categories: Option<Vec<String>>,
    pub priority: Option<i32>,
    pub transp: Option<String>,
    /// CREATED; set on first creation only.
    pub created_at: Option<DateTime<Utc>>,
    pub last_modified: Option<DateTime<Utc>>,
    pub rdates: Option<Vec<String>>,
    pub exdates: Option<Vec<String>>,
    pub alarms: Option<Vec<AlarmSpec>>,
}

/// The ICS body for `input`.
///
/// A recurring event gets DTSTART + DURATION (each occurrence carries its own
/// length); a single event gets DTSTART + DTEND. A malformed RRULE is dropped
/// rather than written.
pub fn build_event(input: &NewEvent) -> Result<String, ICalError> {
    let uid = input.uid.clone().unwrap_or_else(|| format!("{}@icloud-calendar-mcp", uuid::Uuid::new_v4()));
    let rrule_text = input.rrule.as_deref().filter(|r| !r.trim().is_empty());
    let times = resolve_times(input, rrule_text.is_some())?;

    let mut event = ICalEvent::new(uid, times.start);
    event.dt_end = times.end;
    event.duration = times.duration;
    event.is_all_day = input.is_all_day;
    event.summary = Some(input.summary.clone());
    event.description = input.description.clone().filter(|d| !d.trim().is_empty());
    event.location = input.location.clone().filter(|l| !l.trim().is_empty());
    event.status = input.status.as_deref().filter(|s| !s.trim().is_empty()).map(EventStatus::from_value);
    event.rrule = rrule_text.and_then(|r| RRule::parse(r).ok());
    event.exdates = recurrence_dates(input.exdates.as_deref(), input.is_all_day)?;
    event.rdates = recurrence_dates(input.rdates.as_deref(), input.is_all_day)?;
    event.alarms = input.alarms.iter().flatten().map(AlarmSpec::to_ical_alarm).collect();
    event.categories = input.categories.clone().unwrap_or_default();
    event.last_modified = input.last_modified.map(|t| ICalDateTime::utc(t.timestamp_millis()));
    event.created = input.created_at.map(|t| ICalDateTime::utc(t.timestamp_millis()));
    event.transparency = Transparency::from_value(input.transp.as_deref());
    event.url = input.url.clone().filter(|u| !u.trim().is_empty());
    event.priority = input.priority.unwrap_or(0);
    Ok(generate_calendar(&[event]))
}

pub(crate) fn recurrence_dates(values: Option<&[String]>, is_all_day: bool) -> Result<Vec<ICalDateTime>, ICalError> {
    values.unwrap_or_default().iter().map(|v| recurrence_date(v, is_all_day)).collect()
}

struct ResolvedTimes {
    start: ICalDateTime,
    end: Option<ICalDateTime>,
    duration: Option<TimeDelta>,
}

fn resolve_times(input: &NewEvent, recurring: bool) -> Result<ResolvedTimes, ICalError> {
    if let (true, Some(start_date)) = (input.is_all_day, input.start_date.as_deref()) {
        let start = parse_date(start_date)?;
        let inclusive_end = parse_date(input.end_date.as_deref().unwrap_or(start_date))?;
        let exclusive_end = inclusive_end + TimeDelta::days(1);
        let dt_start = ICalDateTime::from_local_date(start);
        return Ok(if recurring {
            ResolvedTimes { start: dt_start, end: None, duration: Some(exclusive_end - start) }
        } else {
            ResolvedTimes { start: dt_start, end: Some(ICalDateTime::from_local_date(exclusive_end)), duration: None }
        });
    }
    if let (Some(start_time), Some(end_time)) = (input.start_time.as_deref(), input.end_time.as_deref()) {
        let start = resolve_timed(start_time, input.timezone.as_deref())?;
        let end_zone = input.end_timezone.as_deref().or(input.timezone.as_deref());
        let end = resolve_timed(end_time, end_zone)?;
        return Ok(if recurring {
            ResolvedTimes { start, end: None, duration: Some(TimeDelta::milliseconds(end.timestamp - start.timestamp)) }
        } else {
            ResolvedTimes { start, end: Some(end), duration: None }
        });
    }
    // No usable times (the patch-or-create fallback given only a summary). A
    // VEVENT still needs a DTSTART, so default to a one-hour event starting now.
    let now = Utc::now().timestamp_millis();
    let start = ICalDateTime::utc(now);
    Ok(if recurring {
        ResolvedTimes { start, end: None, duration: Some(TimeDelta::hours(1)) }
    } else {
        ResolvedTimes { start, end: Some(ICalDateTime::utc(now + 3_600_000)), duration: None }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::parser::parse_all_events;

    fn timed() -> NewEvent {
        NewEvent {
            summary: "Meeting".into(),
            start_time: Some("2025-01-15T10:00:00Z".into()),
            end_time: Some("2025-01-15T11:00:00Z".into()),
            ..NewEvent::default()
        }
    }

    #[test]
    fn builds_timed_event() {
        let ics = build_event(&timed()).unwrap();
        assert!(ics.contains("DTSTART:20250115T100000Z\r\n"));
        assert!(ics.contains("DTEND:20250115T110000Z\r\n"));
        assert!(ics.contains("SUMMARY:Meeting\r\n"));
        assert!(ics.contains("@icloud-calendar-mcp\r\n"));
        assert!(!ics.contains("DURATION"));
    }

    #[test]
    fn builds_all_day_with_exclusive_end() {
        let ics = build_event(&NewEvent {
            summary: "Trip".into(),
            is_all_day: true,
            start_date: Some("2025-01-15".into()),
            end_date: Some("2025-01-17".into()),
            ..NewEvent::default()
        })
        .unwrap();
        assert!(ics.contains("DTSTART;VALUE=DATE:20250115\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20250118\r\n"));
    }

    #[test]
    fn recurring_uses_duration() {
        let ics = build_event(&NewEvent { rrule: Some("FREQ=WEEKLY;BYDAY=MO".into()), ..timed() }).unwrap();
        assert!(ics.contains("DURATION:PT1H\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=MO\r\n"));
        assert!(!ics.contains("DTEND"));
    }

    #[test]
    fn malformed_rrule_is_dropped() {
        let ics = build_event(&NewEvent { rrule: Some("FREQ=SOMETIMES".into()), ..timed() }).unwrap();
        assert!(!ics.contains("RRULE"));
    }

    #[test]
    fn naive_times_anchor_to_timezones() {
        let ics = build_event(&NewEvent {
            start_time: Some("2025-01-15T09:00:00".into()),
            end_time: Some("2025-01-15T12:00:00".into()),
            timezone: Some("America/New_York".into()),
            end_timezone: Some("America/Los_Angeles".into()),
            ..timed()
        })
        .unwrap();
        assert!(ics.contains("DTSTART;TZID=America/New_York:20250115T090000\r\n"));
        assert!(ics.contains("DTEND;TZID=America/Los_Angeles:20250115T120000\r\n"));
        assert!(ics.contains("TZID:America/New_York\r\n") && ics.contains("TZID:America/Los_Angeles\r\n"));
    }

    #[test]
    fn explicit_offset_wins_over_timezone() {
        let ics = build_event(&NewEvent {
            start_time: Some("2025-01-15T18:00:00+09:00".into()),
            end_time: Some("2025-01-15T19:00:00+09:00".into()),
            timezone: Some("America/New_York".into()),
            ..timed()
        })
        .unwrap();
        assert!(ics.contains("DTSTART:20250115T090000Z\r\n"));
    }

    #[test]
    fn alarms_rdates_exdates_and_metadata() {
        let created = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let ics = build_event(&NewEvent {
            rrule: Some("FREQ=DAILY".into()),
            rdates: Some(vec!["2025-02-01T10:00:00Z".into()]),
            exdates: Some(vec!["2025-01-16T10:00:00".into()]),
            alarms: Some(vec![
                AlarmSpec { trigger: "-PT15M".into(), ..AlarmSpec::default() },
                AlarmSpec {
                    trigger: "20250115T093000Z".into(),
                    action: Some("EMAIL".into()),
                    repeat_count: Some(2),
                    repeat_duration: Some("PT5M".into()),
                    ..AlarmSpec::default()
                },
                AlarmSpec { trigger: "-PT5M".into(), repeat_count: Some(3), ..AlarmSpec::default() },
            ]),
            created_at: Some(created),
            last_modified: Some(created),
            ..timed()
        })
        .unwrap();
        assert!(ics.contains("RDATE:20250201T100000Z\r\n"));
        assert!(ics.contains("EXDATE:20250116T100000Z\r\n"));
        assert!(ics.contains("CREATED:20250101T000000Z\r\n"));
        assert!(ics.contains("LAST-MODIFIED:20250101T000000Z\r\n"));
        let event = &parse_all_events(&ics).unwrap()[0];
        assert_eq!(event.alarms.len(), 3);
        assert_eq!(event.alarms[1].trigger_absolute.unwrap().to_ical_string(), "20250115T093000Z");
        assert_eq!(event.alarms[1].repeat_count, 2);
        assert_eq!(event.alarms[2].repeat_count, 0, "REPEAT without DURATION is not written");
    }

    #[test]
    fn falls_back_to_one_hour_from_now() {
        let ics = build_event(&NewEvent { summary: "Only title".into(), ..NewEvent::default() }).unwrap();
        let event = &parse_all_events(&ics).unwrap()[0];
        assert_eq!(event.dt_end.unwrap().timestamp - event.dt_start.timestamp, 3_600_000);
    }

    #[test]
    fn invalid_dates_are_errors() {
        let result = build_event(&NewEvent {
            summary: "x".into(),
            is_all_day: true,
            start_date: Some("2025-13-45".into()),
            ..NewEvent::default()
        });
        assert!(result.is_err());
    }
}
