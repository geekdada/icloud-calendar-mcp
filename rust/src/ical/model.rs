//! The VEVENT / VALARM model the parser produces and the generator writes.

use chrono::TimeDelta;

use super::content::ContentLine;
use super::datetime::ICalDateTime;
use super::rrule::RRule;

/// RFC 5545 §3.8.1.11 STATUS for events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

impl EventStatus {
    /// Unknown values read as CONFIRMED.
    pub fn from_value(value: &str) -> Self {
        match value.trim().to_ascii_uppercase().as_str() {
            "TENTATIVE" => EventStatus::Tentative,
            "CANCELLED" => EventStatus::Cancelled,
            _ => EventStatus::Confirmed,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EventStatus::Confirmed => "CONFIRMED",
            EventStatus::Tentative => "TENTATIVE",
            EventStatus::Cancelled => "CANCELLED",
        }
    }
}

/// RFC 5545 §3.8.2.7 TRANSP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transparency {
    #[default]
    Opaque,
    Transparent,
}

impl Transparency {
    /// Anything but TRANSPARENT reads as OPAQUE.
    pub fn from_value(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_uppercase()) {
            Some(v) if v == "TRANSPARENT" => Transparency::Transparent,
            _ => Transparency::Opaque,
        }
    }
}

/// RFC 5545 §3.8.6.1 ACTION for alarms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmAction {
    Audio,
    Display,
    Email,
    /// RFC 9074 "no action" sentinel.
    None,
}

impl AlarmAction {
    /// Unknown or missing values read as DISPLAY.
    pub fn from_value(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_uppercase()).as_deref() {
            Some("AUDIO") => AlarmAction::Audio,
            Some("EMAIL") => AlarmAction::Email,
            Some("NONE") => AlarmAction::None,
            _ => AlarmAction::Display,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AlarmAction::Audio => "AUDIO",
            AlarmAction::Display => "DISPLAY",
            AlarmAction::Email => "EMAIL",
            AlarmAction::None => "NONE",
        }
    }
}

/// A VALARM component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ICalAlarm {
    pub action: AlarmAction,
    /// Relative trigger (negative = before the event).
    pub trigger: Option<TimeDelta>,
    /// Absolute trigger, used when `trigger` is `None`.
    pub trigger_absolute: Option<ICalDateTime>,
    /// TRIGGER;RELATED=END.
    pub trigger_related_to_end: bool,
    pub description: Option<String>,
    pub summary: Option<String>,
    pub repeat_count: i32,
    pub repeat_duration: Option<TimeDelta>,
    /// Alarm UID (RFC 9074); one is minted on output when absent.
    pub uid: Option<String>,
    /// Properties not modelled above, re-emitted verbatim.
    pub raw_properties: Vec<ContentLine>,
}

impl ICalAlarm {
    /// A DISPLAY alarm with no trigger and no extra properties.
    pub fn new(action: AlarmAction) -> Self {
        Self {
            action,
            trigger: None,
            trigger_absolute: None,
            trigger_related_to_end: false,
            description: None,
            summary: None,
            repeat_count: 0,
            repeat_duration: None,
            uid: None,
            raw_properties: Vec::new(),
        }
    }
}

/// ORGANIZER, reduced to what the MCP layer reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Organizer {
    pub email: String,
    /// CN parameter.
    pub name: Option<String>,
}

/// ATTENDEE, reduced to what the MCP layer reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    pub email: String,
    pub name: Option<String>,
}

/// A VEVENT component.
///
/// Properties this model does not interpret (ORGANIZER, ATTENDEE, X-*, CLASS,
/// GEO, ...) are kept verbatim in `raw_properties` and written back unchanged,
/// so a parse/patch/generate round trip never drops server or client data.
/// `organizer` and `attendees` are read-only views over those raw lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ICalEvent {
    pub uid: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub dt_start: ICalDateTime,
    /// DTEND (mutually exclusive with `duration` on output; DTEND wins).
    pub dt_end: Option<ICalDateTime>,
    pub duration: Option<TimeDelta>,
    pub is_all_day: bool,
    /// `None` when the source had no STATUS (RFC 5545 gives it no default).
    pub status: Option<EventStatus>,
    pub sequence: i32,
    pub rrule: Option<RRule>,
    pub exdates: Vec<ICalDateTime>,
    pub rdates: Vec<ICalDateTime>,
    /// Set on a modified instance of a recurring series.
    pub recurrence_id: Option<ICalDateTime>,
    /// RANGE parameter of RECURRENCE-ID (`THISANDFUTURE`), kept for round trips.
    pub recurrence_id_range: Option<String>,
    pub alarms: Vec<ICalAlarm>,
    pub categories: Vec<String>,
    pub organizer: Option<Organizer>,
    pub attendees: Vec<Attendee>,
    pub dtstamp: Option<ICalDateTime>,
    pub last_modified: Option<ICalDateTime>,
    pub created: Option<ICalDateTime>,
    pub transparency: Transparency,
    pub url: Option<String>,
    /// 0 = undefined, 1 = highest, 9 = lowest.
    pub priority: i32,
    pub raw_properties: Vec<ContentLine>,
}

impl ICalEvent {
    /// A minimal event: `uid` starting at `dt_start`, everything else empty.
    pub fn new(uid: impl Into<String>, dt_start: ICalDateTime) -> Self {
        Self {
            uid: uid.into(),
            summary: None,
            description: None,
            location: None,
            dt_start,
            dt_end: None,
            duration: None,
            is_all_day: dt_start.is_date,
            status: None,
            sequence: 0,
            rrule: None,
            exdates: Vec::new(),
            rdates: Vec::new(),
            recurrence_id: None,
            recurrence_id_range: None,
            alarms: Vec::new(),
            categories: Vec::new(),
            organizer: None,
            attendees: Vec::new(),
            dtstamp: None,
            last_modified: None,
            created: None,
            transparency: Transparency::Opaque,
            url: None,
            priority: 0,
            raw_properties: Vec::new(),
        }
    }

    /// True when the event carries an RRULE or RDATE.
    pub fn is_recurring(&self) -> bool {
        self.rrule.is_some() || !self.rdates.is_empty()
    }
}
