//! RFC 5545 engine: the subset of an iCalendar library the server needs.
//!
//! - [`content`]: unfolding, content-line lexing, components, TEXT escaping, folding.
//! - [`model`] / [`parser`] / [`generator`]: VEVENT and VALARM in and out.
//! - [`datetime`], [`duration`], [`rrule`]: value types.
//! - [`recur`] / [`expander`]: recurrence expansion with work bounds.
//! - [`vtimezone`] / [`timezone`]: zone resolution and VTIMEZONE output.

pub mod content;
pub mod datetime;
pub mod duration;
pub mod expander;
pub mod generator;
pub mod model;
pub mod parser;
pub mod recur;
pub mod rrule;
pub mod timezone;
pub mod vtimezone;
mod windows_zones;

use std::fmt;

pub use datetime::ICalDateTime;
pub use model::{AlarmAction, EventStatus, ICalAlarm, ICalEvent, Transparency};
pub use rrule::{Frequency, RRule, WeekdayNum};

/// A malformed iCalendar value or document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ICalError {
    pub message: String,
}

impl ICalError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl fmt::Display for ICalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ICalError {}
