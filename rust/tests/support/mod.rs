//! Shared test support: the in-memory [`CalDavClient`] double and fixtures.
//!
//! Behavior is resolved by precedence, and every override defaults to a no-op,
//! so a test only configures what it needs:
//! 1. `error_to_return`: a blanket error for every call;
//! 2. per-operation canned results (`list_calendars_result`, `get_event_result`,
//!    `update_event_result`, `delete_event_result`, `created_event_to_return`,
//!    `updated_event_to_return`, `delete_success`);
//! 3. the stateful default: `create_event` stores the event in
//!    `registered_events`, which `get_event`/`update_event`/`delete_event` use.
//!
//! `get_events` returns `events_to_return`, else `events_response`; it is kept
//! apart from the created-event store so read-after-write lag can be simulated.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use icloud_calendar_mcp::caldav::{CalDavCalendar, CalDavClient, CalDavError, CalDavEvent, CalDavResult};
use icloud_calendar_mcp::logging::{McpLogger, MemorySink};
use icloud_calendar_mcp::ratelimit::RateLimiter;
use icloud_calendar_mcp::server::CalendarServer;
use icloud_calendar_mcp::service::CalendarService;

#[derive(Default)]
pub struct MockState {
    pub calendars: Vec<CalDavCalendar>,
    pub list_calendars_result: Option<CalDavResult<Vec<CalDavCalendar>>>,
    pub calendars_to_return: Option<Vec<CalDavCalendar>>,

    pub events_response: Vec<CalDavEvent>,
    pub events_to_return: Option<Vec<CalDavEvent>>,
    /// Keyed by UID.
    pub registered_events: HashMap<String, CalDavEvent>,

    pub error_to_return: Option<CalDavError>,

    pub get_event_result: Option<CalDavResult<CalDavEvent>>,
    pub created_event_to_return: Option<CalDavEvent>,
    pub updated_event_to_return: Option<CalDavEvent>,
    pub update_event_result: Option<CalDavResult<CalDavEvent>>,
    pub delete_event_result: Option<CalDavResult<()>>,
    /// When `Some(false)`, delete returns a 500.
    pub delete_success: Option<bool>,
    pub check_connection_result: Option<CalDavResult<bool>>,

    /// The next N update/delete calls return 412 before succeeding.
    pub fail_412_updates_remaining: u32,
    pub fail_412_deletes_remaining: u32,

    pub last_created_ics: Option<String>,
    pub last_updated_ics: Option<String>,
    pub last_deleted_href: Option<String>,
    pub last_calendar_id: Option<String>,
    pub last_start_date: Option<String>,
    pub last_end_date: Option<String>,
    pub update_etags_seen: Vec<Option<String>>,
    pub delete_etags_seen: Vec<Option<String>>,
    pub created_ics_history: Vec<String>,
    pub updated_ics_history: Vec<String>,

    pub list_calendars_called: u32,
    pub get_events_called: u32,
    pub get_event_called: u32,
    pub create_event_called: u32,
    pub update_event_called: u32,
    pub delete_event_called: u32,
    pub check_connection_called: u32,
}

#[derive(Default)]
pub struct MockCalDavClient {
    state: Mutex<MockState>,
}

impl MockCalDavClient {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Lock the state to configure the mock or inspect captured calls.
    pub fn state(&self) -> MutexGuard<'_, MockState> {
        self.state.lock().unwrap()
    }

    pub fn with_calendars(calendars: Vec<CalDavCalendar>) -> Arc<Self> {
        let mock = Self::new();
        mock.state().calendars = calendars;
        mock
    }
}

fn uid_of(ics: &str) -> String {
    ics.lines()
        .find_map(|l| l.trim_end_matches('\r').strip_prefix("UID:"))
        .map(str::to_string)
        .unwrap_or_else(|| "generated-uid".into())
}

#[async_trait]
impl CalDavClient for MockCalDavClient {
    async fn list_calendars(&self) -> CalDavResult<Vec<CalDavCalendar>> {
        let mut s = self.state();
        s.list_calendars_called += 1;
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if let Some(r) = &s.list_calendars_result {
            return r.clone();
        }
        Ok(s.calendars_to_return.clone().unwrap_or_else(|| s.calendars.clone()))
    }

    async fn get_events(&self, calendar_id: &str, start_date: &str, end_date: &str) -> CalDavResult<Vec<CalDavEvent>> {
        let mut s = self.state();
        s.get_events_called += 1;
        s.last_calendar_id = Some(calendar_id.into());
        s.last_start_date = Some(start_date.into());
        s.last_end_date = Some(end_date.into());
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if !s.calendars.is_empty() && !s.calendars.iter().any(|c| c.id == calendar_id) {
            return Err(CalDavError::not_found(format!("Calendar not found: {calendar_id}")));
        }
        Ok(s.events_to_return.clone().unwrap_or_else(|| s.events_response.clone()))
    }

    async fn get_event(&self, href: &str) -> CalDavResult<CalDavEvent> {
        let mut s = self.state();
        s.get_event_called += 1;
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if let Some(r) = &s.get_event_result {
            return r.clone();
        }
        s.registered_events
            .values()
            .find(|e| e.href == href)
            .or_else(|| s.events_response.iter().find(|e| e.href == href))
            .or_else(|| s.events_to_return.as_ref().and_then(|list| list.iter().find(|e| e.href == href)))
            .cloned()
            .ok_or_else(|| CalDavError::not_found(format!("Event not found: {href}")))
    }

    async fn create_event(&self, _calendar_id: &str, ical_data: &str) -> CalDavResult<CalDavEvent> {
        let mut s = self.state();
        s.create_event_called += 1;
        s.last_created_ics = Some(ical_data.into());
        s.created_ics_history.push(ical_data.into());
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if let Some(e) = &s.created_event_to_return {
            return Ok(e.clone());
        }
        let uid = uid_of(ical_data);
        let event = CalDavEvent {
            uid: uid.clone(),
            href: format!("/cal/{uid}.ics"),
            url: format!("https://test.com/cal/{uid}.ics"),
            etag: Some("\"new-etag\"".into()),
            ical_data: ical_data.into(),
        };
        s.registered_events.insert(uid, event.clone());
        Ok(event)
    }

    async fn update_event(&self, href: &str, ical_data: &str, etag: Option<&str>) -> CalDavResult<CalDavEvent> {
        let mut s = self.state();
        s.update_event_called += 1;
        s.update_etags_seen.push(etag.map(str::to_string));
        s.last_updated_ics = Some(ical_data.into());
        s.updated_ics_history.push(ical_data.into());
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if s.fail_412_updates_remaining > 0 {
            s.fail_412_updates_remaining -= 1;
            return Err(CalDavError::new(412, "Precondition failed", false));
        }
        if let Some(r) = &s.update_event_result {
            return r.clone();
        }
        if let Some(e) = &s.updated_event_to_return {
            return Ok(e.clone());
        }
        let existing = s
            .registered_events
            .values()
            .find(|e| e.href == href)
            .cloned()
            .ok_or_else(|| CalDavError::not_found("Event not found"))?;
        let updated = CalDavEvent { ical_data: ical_data.into(), etag: Some("\"updated-etag\"".into()), ..existing };
        s.registered_events.insert(updated.uid.clone(), updated.clone());
        Ok(updated)
    }

    async fn delete_event(&self, href: &str, etag: Option<&str>) -> CalDavResult<()> {
        let mut s = self.state();
        s.delete_event_called += 1;
        s.delete_etags_seen.push(etag.map(str::to_string));
        s.last_deleted_href = Some(href.into());
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if s.fail_412_deletes_remaining > 0 {
            s.fail_412_deletes_remaining -= 1;
            return Err(CalDavError::new(412, "Precondition failed", false));
        }
        if let Some(r) = &s.delete_event_result {
            return r.clone();
        }
        if s.delete_success == Some(false) {
            return Err(CalDavError::server(500, "Delete failed"));
        }
        let uid = s.registered_events.values().find(|e| e.href == href).map(|e| e.uid.clone());
        if let Some(uid) = uid {
            s.registered_events.remove(&uid);
        }
        Ok(())
    }

    async fn check_connection(&self) -> CalDavResult<bool> {
        let mut s = self.state();
        s.check_connection_called += 1;
        s.check_connection_result.clone().unwrap_or(Ok(true))
    }

    async fn fetch_etags(&self, calendar_id: &str, _start: &str, _end: &str) -> CalDavResult<HashMap<String, Option<String>>> {
        let s = self.state();
        if let Some(e) = &s.error_to_return {
            return Err(e.clone());
        }
        if !s.calendars.is_empty() && !s.calendars.iter().any(|c| c.id == calendar_id) {
            return Err(CalDavError::not_found(format!("Calendar not found: {calendar_id}")));
        }
        let events = s.events_to_return.clone().unwrap_or_else(|| s.events_response.clone());
        Ok(events.into_iter().map(|e| (e.href, e.etag)).collect())
    }
}

// ───────────────────────────── fixtures ─────────────────────────────

/// A writable calendar `id` named `name`.
pub fn calendar(id: &str, name: &str) -> CalDavCalendar {
    CalDavCalendar {
        id: id.into(),
        href: format!("/123/calendars/{id}/"),
        url: format!("https://caldav.icloud.com/123/calendars/{id}/"),
        display_name: name.into(),
        color: Some("#FF0000".into()),
        ctag: None,
        is_read_only: false,
        supported_components: vec!["VEVENT".into()],
    }
}

/// A read-only calendar `id` named `name`.
pub fn read_only_calendar(id: &str, name: &str) -> CalDavCalendar {
    CalDavCalendar { is_read_only: true, ..calendar(id, name) }
}

/// A server event resource holding `ics` at `/cal/<uid>.ics`.
pub fn caldav_event(uid: &str, ics: &str) -> CalDavEvent {
    CalDavEvent {
        uid: uid.into(),
        href: format!("/cal/{uid}.ics"),
        url: format!("https://test.com/cal/{uid}.ics"),
        etag: Some("\"etag-1\"".into()),
        ical_data: ics.into(),
    }
}

/// Wrap VEVENT bodies in a VCALENDAR.
pub fn vcalendar(body: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//Test//EN\r\n{body}END:VCALENDAR\r\n")
}

/// A timed UTC VEVENT `uid` with `summary`, from `start` to `end` (`YYYYMMDDTHHMMSSZ`).
pub fn timed_vevent(uid: &str, summary: &str, start: &str, end: &str) -> String {
    format!("BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20240101T000000Z\r\nDTSTART:{start}\r\nDTEND:{end}\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\n")
}

/// A service over `mock`.
pub fn service(mock: &Arc<MockCalDavClient>) -> Arc<CalendarService> {
    Arc::new(CalendarService::new(mock.clone()))
}

/// A server over `mock` with generous rate limits and a recording log sink.
pub fn server(mock: &Arc<MockCalDavClient>) -> (CalendarServer, Arc<MemorySink>) {
    let sink = Arc::new(MemorySink::new());
    let server = CalendarServer::with_parts(
        Some(service(mock)),
        Arc::new(RateLimiter::new(10_000, 10_000, 60_000)),
        Arc::new(McpLogger::new(sink.clone())),
    );
    (server, sink)
}

/// The text of the first content block of a tool result.
pub fn result_text(result: &rmcp::model::CallToolResult) -> String {
    result.content.first().and_then(|c| c.as_text()).map(|t| t.text.clone()).unwrap_or_default()
}

/// The text of a tool result parsed as JSON.
pub fn result_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    serde_json::from_str(&result_text(result)).expect("tool result text is JSON")
}
