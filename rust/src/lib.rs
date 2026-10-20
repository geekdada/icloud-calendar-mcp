//! iCloud Calendar MCP server.
//!
//! Gives MCP clients access to iCloud Calendar over CalDAV. The crate is layered:
//!
//! - [`server`]: the MCP surface (tools, the calendar resource, prompts), with
//!   rate limiting, input validation and sanitized errors around every call.
//! - [`service`]: CalDAV orchestration, the short-lived event cache and the
//!   recurring-event scope logic.
//! - [`caldav`]: the HTTP/XML CalDAV client.
//! - [`ics`]: the MCP-facing ICS parse/build/patch layer.
//! - [`ical`]: the RFC 5545 engine underneath it (content lines, model,
//!   generator, recurrence expansion, VTIMEZONE generation).

pub mod caldav;
pub mod cancellation;
pub mod credentials;
pub mod error;
pub mod ical;
pub mod ics;
pub mod logging;
pub mod patterns;
pub mod progress;
pub mod ratelimit;
pub mod scope;
pub mod server;
pub mod service;
pub mod validation;
