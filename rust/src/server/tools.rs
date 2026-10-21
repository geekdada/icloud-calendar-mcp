//! The five MCP tools: definitions (schemas, annotations) and handlers.
//!
//! Every handler follows the same order: rate limit, read arguments, validate,
//! check credentials, call the service, shape the response. Failures become
//! sanitized error results; nothing internal reaches the caller.

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use super::CalendarServer;
use super::args::{self, Args};
use crate::error;
use crate::ics::{EventPatch, NewEvent, ParsedAlarm};
use crate::scope::EventScope;
use crate::service::EventInfo;
use crate::validation::{self as v, collect_errors};

const NOT_CONFIGURED: &str = "iCloud credentials not configured";

const GET_EVENTS_DESCRIPTION: &str = "Get events from a calendar within a date range. Reflects iCloud's server state at query time. The range is capped at 366 days, and end_date must not precede start_date; a range past the cap or an inverted one is rejected up front. The response is also capped: a query that would return more than 1000 events, or a single recurring series that expands to too many occurrences in the range, is rejected rather than returned. Query a week or a month at a time to stay under the caps. Day boundaries are UTC: start_date and end_date select whole UTC calendar days (start_date 00:00Z through the end of end_date, UTC), and timed events are returned with UTC startTime/endTime instants (all-day events as plain YYYY-MM-DD). To resolve a user's day in another timezone, request one extra calendar day on each side and keep only the events whose startTime, converted to that zone, lands on the wanted local day; all-day events are floating dates and need no conversion. Note on read-after-write: iCloud has no immediate-visibility guarantee, so an event created moments ago may not appear in the very next get_events (CDN indexing lag) — this is NOT a deletion. If a create_event/update_event returned success, that write landed; do not treat a briefly-missing just-created event as failed or deleted, and do not recreate it (that duplicates the event). Use the returned uid/handle to reference it directly.";

const CREATE_EVENT_DESCRIPTION: &str = "Create a new calendar event. A success response is authoritative: the event was persisted on iCloud and the returned uid/handle can be used immediately for update_event/delete_event. Do NOT verify the write by re-listing with get_events and retrying on absence — iCloud's CDN may not surface a just-created event in the next listing for a short window, and a retry would create a duplicate.";

const UPDATE_EVENT_DESCRIPTION: &str = "Update an existing calendar event. First use get_events (or create_event) to obtain the event's handle. For a recurring series, get_events returns one handle per occurrence (each carries a recurrenceId); use scope to say whether an edit changes just that occurrence or the whole series. If the handle is one occurrence of a series you must set scope, otherwise the edit is rejected rather than silently changing every occurrence.";

const DELETE_EVENT_DESCRIPTION: &str = "Delete a calendar event. First use get_events (or create_event) to obtain the event's handle. For a recurring series, use scope to say whether the delete removes just one occurrence or the whole series. If the handle is one occurrence of a series you must set scope, otherwise the delete is rejected rather than silently cancelling every occurrence.";

const RRULE_DESCRIPTION: &str = "Recurrence rule (e.g., FREQ=WEEKLY;BYDAY=MO). For biweekly schedules whose BYDAY includes Sunday, set WKST=<two-letter-day> explicitly; the default week start is MO, which can drop occurrences.";

/// Tool definitions in registration order.
pub fn definitions() -> Vec<Tool> {
    vec![list_calendars_tool(), get_events_tool(), create_event_tool(), update_event_tool(), delete_event_tool()]
}

fn tool(
    name: &'static str,
    title: &str,
    description: &'static str,
    input: Value,
    output: Value,
    annotations: ToolAnnotations,
) -> Tool {
    Tool::new(name, description, Arc::new(as_object(input)))
        .with_title(title)
        .with_raw_output_schema(Arc::new(as_object(output)))
        .with_annotations(annotations)
}

fn as_object(value: Value) -> JsonObject {
    match value {
        Value::Object(map) => map,
        _ => JsonObject::new(),
    }
}

fn annotations(read_only: bool, destructive: bool, idempotent: bool) -> ToolAnnotations {
    ToolAnnotations::new().read_only(read_only).destructive(destructive).idempotent(idempotent).open_world(true)
}

fn string_prop(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn alarm_item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "trigger": {"type": "string"},
            "action": {"type": "string"},
            "description": {"type": "string"},
            "summary": {"type": "string"},
            "repeat_count": {"type": "integer"},
            "repeat_duration": {"type": "string"}
        },
        "required": ["trigger"]
    })
}

fn list_calendars_tool() -> Tool {
    tool(
        "list_calendars",
        "List Calendars",
        "List all calendars from the connected iCloud account",
        json!({"type": "object", "properties": {}}),
        json!({
            "type": "object",
            "properties": {"calendars": {"type": "array", "description": "List of calendars", "items": {"type": "object"}}},
            "required": ["calendars"]
        }),
        annotations(true, false, true),
    )
}

fn get_events_tool() -> Tool {
    tool(
        "get_events",
        "Get Events",
        GET_EVENTS_DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "calendar_id": string_prop("Calendar ID"),
                "start_date": string_prop("Start date (YYYY-MM-DD), interpreted as a UTC calendar day (00:00Z)."),
                "end_date": string_prop("End date (YYYY-MM-DD), inclusive, interpreted as a UTC calendar day (through the end of the day, UTC). Must be on or after start_date, and within 366 days of it.")
            },
            "required": ["calendar_id", "start_date", "end_date"]
        }),
        json!({
            "type": "object",
            "properties": {"events": {"type": "array", "description": "List of events", "items": {"type": "object"}}},
            "required": ["events"]
        }),
        annotations(true, false, true),
    )
}

fn create_event_tool() -> Tool {
    tool(
        "create_event",
        "Create Event",
        CREATE_EVENT_DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "calendar_id": string_prop("Calendar ID to create event in"),
                "title": string_prop("Event title"),
                "summary": string_prop("Alias for 'title' (the field get_events and the create response echo back). Used only when 'title' is omitted."),
                "start_time": string_prop("Start time (ISO 8601). Naive local (2025-01-15T09:00:00): interpreted as UTC unless 'timezone' is set. UTC (2025-01-15T09:00:00Z) or offset (2025-01-15T18:00:00+09:00): an absolute instant that wins over 'timezone'."),
                "end_time": string_prop("End time (ISO 8601). Naive local (2025-01-15T10:00:00): interpreted as UTC unless 'timezone' is set. UTC (2025-01-15T10:00:00Z) or offset (2025-01-15T19:00:00+09:00): an absolute instant that wins over 'timezone'."),
                "location": string_prop("Event location (optional)"),
                "description": string_prop("Event description (optional)"),
                "is_all_day": {"type": "boolean", "description": "True for all-day events (optional)"},
                "start_date": string_prop("Start date for all-day events (YYYY-MM-DD)"),
                "end_date": string_prop("End date for all-day events (YYYY-MM-DD, inclusive)"),
                "timezone": string_prop("IANA timezone for non-UTC timed events (e.g., America/New_York)"),
                "end_timezone": string_prop("Optional IANA timezone for DTEND when distinct from start (e.g., flights JFK->LAX). Falls back to timezone when omitted."),
                "rrule": string_prop(RRULE_DESCRIPTION),
                "rdates": {"type": "array", "description": "Optional additional occurrence dates (RFC 5545 RDATE). ISO 8601 instants for timed events; YYYY-MM-DD for all-day. One value per array element.", "items": {"type": "string"}},
                "exdates": {"type": "array", "description": "Optional excluded occurrence dates (RFC 5545 EXDATE). Same format as rdates.", "items": {"type": "string"}},
                "alarms": {"type": "array", "description": "Optional VALARM components (RFC 5545 §3.6.6). Each entry is an object with: trigger (required: '-PT15M' duration form or '20260115T093000Z' absolute), action (DISPLAY|AUDIO|EMAIL, default DISPLAY), description, summary (EMAIL only), repeat_count, repeat_duration.", "items": alarm_item_schema()}
            },
            "required": ["calendar_id", "title"]
        }),
        json!({
            "type": "object",
            "properties": {
                "success": {"type": "boolean"},
                "uid": {"type": "string"},
                "handle": string_prop("Opaque, self-contained reference to pass to update_event/delete_event. Works even from a fresh process (no cache dependency)."),
                "summary": {"type": "string"},
                "message": {"type": "string"},
                "alarms": {"type": "array", "description": "VALARM components attached to the created event (RFC 5545 §3.6.6); present only when the event has alarms.", "items": alarm_item_schema()}
            },
            "required": ["success", "uid", "summary", "message"]
        }),
        annotations(false, false, false),
    )
}

fn scope_schema(description: &str) -> Value {
    json!({"type": "string", "enum": EventScope::TOKENS, "description": description})
}

fn update_event_tool() -> Tool {
    tool(
        "update_event",
        "Update Event",
        UPDATE_EVENT_DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "event_id": string_prop("Event reference to update. Prefer the opaque `handle` from get_events/create_event (works even from a fresh process); a bare event UID is also accepted for recently-fetched events."),
                "title": string_prop("New event title (optional)"),
                "start_time": string_prop("New start time (optional, ISO 8601). Naive local (2025-01-15T09:00:00): interpreted as UTC unless 'timezone' is set. UTC (…Z) or offset (…+09:00): an absolute instant that wins over 'timezone'."),
                "end_time": string_prop("New end time (optional, ISO 8601). Naive local (2025-01-15T10:00:00): interpreted as UTC unless 'timezone' is set. UTC (…Z) or offset (…+09:00): an absolute instant that wins over 'timezone'."),
                "location": string_prop("New location (optional)"),
                "description": string_prop("New description (optional)"),
                "is_all_day": {"type": "boolean", "description": "Change to all-day event (optional)"},
                "start_date": string_prop("New start date for all-day events (YYYY-MM-DD)"),
                "end_date": string_prop("New end date for all-day events (YYYY-MM-DD, inclusive)"),
                "timezone": string_prop("IANA timezone for non-UTC timed events (e.g., America/New_York)"),
                "end_timezone": string_prop("Optional IANA timezone for DTEND when distinct from start (e.g., flights JFK->LAX). Falls back to timezone when omitted."),
                "rrule": string_prop(RRULE_DESCRIPTION),
                "rdates": {"type": "array", "description": "Replace additional occurrence dates (RFC 5545 RDATE). ISO 8601 instants for timed; YYYY-MM-DD for all-day. Pass null/omit to leave existing untouched; pass empty array to clear.", "items": {"type": "string"}},
                "exdates": {"type": "array", "description": "Replace excluded occurrence dates (RFC 5545 EXDATE). Same semantics as rdates.", "items": {"type": "string"}},
                "alarms": {"type": "array", "description": "Replace VALARM components (RFC 5545 §3.6.6). Pass null/omit to preserve existing alarms; pass empty array to clear all; pass a list of alarm objects to replace. Each entry has: trigger (required), action (DISPLAY|AUDIO|EMAIL, default DISPLAY), description, summary (EMAIL only), repeat_count, repeat_duration.", "items": alarm_item_schema()},
                "scope": scope_schema("For a recurring series: which occurrences the edit affects. `this_occurrence` edits only the instance the handle points at (leaving siblings unchanged); `this_and_future` edits that instance and every later one (earlier ones unchanged); `all_events` edits the whole series. Required when the handle is one occurrence of a series; omit for standalone events. rrule/rdates/exdates cannot be combined with an occurrence scope.")
            },
            "required": ["event_id"]
        }),
        json!({
            "type": "object",
            "properties": {
                "success": {"type": "boolean"},
                "uid": {"type": "string"},
                "handle": string_prop("Refreshed opaque reference (carries the new etag) for a subsequent update_event/delete_event. For a this_occurrence edit this is an occurrence handle for the edited instance."),
                "message": {"type": "string"},
                "alarms": {"type": "array", "description": "VALARM components attached to the updated event (RFC 5545 §3.6.6); present only when the event has alarms.", "items": alarm_item_schema()}
            },
            "required": ["success", "uid", "message"]
        }),
        annotations(false, false, true),
    )
}

fn delete_event_tool() -> Tool {
    tool(
        "delete_event",
        "Delete Event",
        DELETE_EVENT_DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "event_id": string_prop("Event reference to delete. Prefer the opaque `handle` from get_events/create_event (works even from a fresh process); a bare event UID is also accepted for recently-fetched events."),
                "scope": scope_schema("For a recurring series: which occurrences the delete removes. `this_occurrence` cancels only the instance the handle points at (an EXDATE; siblings remain); `this_and_future` cancels that instance and every later one (earlier ones remain); `all_events` deletes the whole series. Required when the handle is one occurrence of a series; omit for standalone events.")
            },
            "required": ["event_id"]
        }),
        json!({
            "type": "object",
            "properties": {"success": {"type": "boolean"}, "message": {"type": "string"}},
            "required": ["success", "message"]
        }),
        annotations(false, true, true),
    )
}

/// A success result whose text is the compact JSON of `body`, also returned
/// as structured content.
fn json_result(body: Map<String, Value>) -> CallToolResult {
    let value = Value::Object(body);
    let mut result = CallToolResult::success(vec![ContentBlock::text(value.to_string())]);
    result.structured_content = Some(value);
    result
}

fn alarms_json(alarms: &[ParsedAlarm]) -> Value {
    Value::Array(
        alarms
            .iter()
            .map(|a| {
                let mut entry = Map::new();
                entry.insert("trigger".into(), json!(a.trigger));
                entry.insert("action".into(), json!(a.action));
                insert_opt(&mut entry, "description", a.description.as_deref());
                insert_opt(&mut entry, "summary", a.summary.as_deref());
                if let Some(count) = a.repeat_count {
                    entry.insert("repeat_count".into(), json!(count));
                }
                insert_opt(&mut entry, "repeat_duration", a.repeat_duration.as_deref());
                Value::Object(entry)
            })
            .collect(),
    )
}

fn insert_opt(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.into(), json!(value));
    }
}

fn event_json(event: &EventInfo) -> Value {
    let mut e = Map::new();
    e.insert("uid".into(), json!(event.uid));
    insert_opt(&mut e, "handle", event.handle.as_deref());
    e.insert("summary".into(), json!(event.summary));
    insert_opt(&mut e, "description", event.description.as_deref());
    insert_opt(&mut e, "location", event.location.as_deref());
    e.insert("isAllDay".into(), json!(event.is_all_day));
    if event.is_all_day {
        insert_opt(&mut e, "startDate", event.start_date.as_deref());
        insert_opt(&mut e, "endDate", event.end_date.as_deref());
    } else {
        insert_opt(&mut e, "startTime", event.start_time.as_deref());
        insert_opt(&mut e, "endTime", event.end_time.as_deref());
    }
    insert_opt(&mut e, "rrule", event.rrule.as_deref());
    insert_opt(&mut e, "recurrenceId", event.recurrence_id.as_deref());
    insert_opt(&mut e, "status", event.status.as_deref());
    insert_opt(&mut e, "url", event.url.as_deref());
    if !event.categories.is_empty() {
        e.insert("categories".into(), json!(event.categories));
    }
    if let Some(priority) = event.priority {
        e.insert("priority".into(), json!(priority));
    }
    insert_opt(&mut e, "organizer", event.organizer.as_deref());
    if event.attendee_count > 0 {
        e.insert("attendeeCount".into(), json!(event.attendee_count));
    }
    Value::Object(e)
}

/// Read every argument a handler needs, turning shape errors into a
/// validation error result.
macro_rules! read_args {
    ($($binding:ident = $expr:expr;)*) => {
        $(
            let $binding = match $expr {
                Ok(value) => value,
                Err(message) => return error::validation_error(&[message]),
            };
        )*
    };
}

impl CalendarServer {
    pub(super) async fn list_calendars(&self) -> CallToolResult {
        let rate = self.rate_limiter.acquire_read_with_status();
        if !rate.allowed {
            return error::rate_limit_error(rate.retry_after_ms);
        }
        let Some(service) = self.service.as_ref() else {
            return error::authentication_error(NOT_CONFIGURED);
        };
        match service.list_calendars().await {
            Ok(calendars) => {
                let list: Vec<Value> = calendars
                    .iter()
                    .map(|c| {
                        let mut entry = Map::new();
                        entry.insert("id".into(), json!(c.id));
                        entry.insert("name".into(), json!(c.name));
                        insert_opt(&mut entry, "color", c.color.as_deref());
                        entry.insert("readOnly".into(), json!(c.read_only));
                        if !c.supported_components.is_empty() {
                            entry.insert("supportedComponents".into(), json!(c.supported_components));
                        }
                        Value::Object(entry)
                    })
                    .collect();
                let mut body = Map::new();
                body.insert("calendars".into(), Value::Array(list));
                json_result(body)
            }
            Err(e) => error::service_error(e.code, &e.message),
        }
    }

    pub(super) async fn get_events(&self, args: &Args) -> CallToolResult {
        let rate = self.rate_limiter.acquire_read_with_status();
        if !rate.allowed {
            return error::rate_limit_error(rate.retry_after_ms);
        }
        read_args! {
            calendar_id = args::string(args, "calendar_id");
            start_date = args::string(args, "start_date");
            end_date = args::string(args, "end_date");
        }
        let errors = collect_errors(&[
            v::validate_calendar_id(calendar_id.as_deref(), "calendar_id"),
            v::validate_date(start_date.as_deref(), "start_date"),
            v::validate_date(end_date.as_deref(), "end_date"),
            // Rejects inverted or over-366-day ranges before any CalDAV fetch.
            v::validate_date_span(start_date.as_deref(), end_date.as_deref()),
        ]);
        if !errors.is_empty() {
            return error::validation_error(&errors);
        }
        let Some(service) = self.service.as_ref() else {
            return error::authentication_error(NOT_CONFIGURED);
        };
        let (calendar_id, start_date, end_date) =
            (calendar_id.unwrap_or_default(), start_date.unwrap_or_default(), end_date.unwrap_or_default());
        match service.get_events(&calendar_id, &start_date, &end_date).await {
            Ok(events) => {
                let mut body = Map::new();
                body.insert("events".into(), Value::Array(events.iter().map(event_json).collect()));
                json_result(body)
            }
            Err(e) => error::service_error(e.code, &e.message),
        }
    }

    pub(super) async fn create_event(&self, args: &Args) -> CallToolResult {
        let rate = self.rate_limiter.acquire_write_with_status();
        if !rate.allowed {
            return error::rate_limit_error(rate.retry_after_ms);
        }
        read_args! {
            calendar_id = args::string(args, "calendar_id");
            // `summary` is accepted as an alias: get_events and the create
            // response echo the title back under that name.
            title = args::string(args, "title").and_then(|t| match t {
                Some(t) => Ok(Some(t)),
                None => args::string(args, "summary"),
            });
            start_time = args::string(args, "start_time");
            end_time = args::string(args, "end_time");
            location = args::string(args, "location");
            description = args::string(args, "description");
            start_date = args::string(args, "start_date");
            end_date = args::string(args, "end_date");
            timezone = args::string(args, "timezone");
            end_timezone = args::string(args, "end_timezone");
            rrule = args::string(args, "rrule");
            rdates = args::string_list(args, "rdates");
            exdates = args::string_list(args, "exdates");
            alarms = args::alarms(args, "alarms");
        }
        let is_all_day = args::boolean(args, "is_all_day").unwrap_or(false);

        let mut errors = collect_errors(&[
            v::validate_calendar_id(calendar_id.as_deref(), "calendar_id"),
            v::validate_title(title.as_deref(), "title"),
        ]);
        if is_all_day {
            errors.extend(collect_errors(&[
                v::validate_date(start_date.as_deref(), "start_date"),
                v::validate_date(end_date.as_deref(), "end_date"),
            ]));
        } else {
            errors.extend(collect_errors(&[
                v::validate_date_time(start_time.as_deref(), "start_time"),
                v::validate_date_time(end_time.as_deref(), "end_time"),
            ]));
            // Naive endpoints resolve in the same zone the write path uses, so
            // a mixed naive/zoned range is ordered honestly.
            if let (Some(start), Some(end)) = (start_time.as_deref(), end_time.as_deref()) {
                errors.extend(v::validate_time_range(start, end, timezone.as_deref(), end_timezone.as_deref()).err());
            }
        }
        errors.extend(collect_errors(&[
            v::validate_optional_text(location.as_deref(), "location", v::MAX_LOCATION_LENGTH),
            v::validate_optional_text(description.as_deref(), "description", v::MAX_DESCRIPTION_LENGTH),
            v::validate_timezone(timezone.as_deref(), "timezone"),
            v::validate_timezone(end_timezone.as_deref(), "end_timezone"),
            v::validate_recurrence_date_list(rdates.as_deref(), "rdates"),
            v::validate_recurrence_date_list(exdates.as_deref(), "exdates"),
            v::validate_alarm_list(alarms.as_deref(), "alarms"),
        ]));
        if !errors.is_empty() {
            return error::validation_error(&errors);
        }
        let Some(service) = self.service.as_ref() else {
            return error::authentication_error(NOT_CONFIGURED);
        };

        let calendar_id = calendar_id.unwrap_or_default();
        let new_event = NewEvent {
            summary: v::sanitize_for_ics(title.as_deref().unwrap_or_default()),
            start_time: start_time.filter(|_| !is_all_day),
            end_time: end_time.filter(|_| !is_all_day),
            start_date: start_date.filter(|_| is_all_day),
            end_date: end_date.filter(|_| is_all_day),
            is_all_day,
            description: description.as_deref().map(v::sanitize_for_ics),
            location: location.as_deref().map(v::sanitize_for_ics),
            timezone,
            end_timezone,
            rrule,
            rdates,
            exdates,
            alarms: args::alarm_specs(alarms.as_deref()),
            ..NewEvent::default()
        };
        match service.create_event(&calendar_id, new_event).await {
            Ok(created) => {
                self.logger.info(
                    "Event created",
                    &[("tool", json!("create_event")), ("uid", json!(created.uid)), ("calendar", json!(calendar_id))],
                );
                let mut body = Map::new();
                body.insert("success".into(), json!(true));
                body.insert("uid".into(), json!(created.uid));
                insert_opt(&mut body, "handle", created.handle.as_deref());
                body.insert("summary".into(), json!(created.summary));
                body.insert("message".into(), json!("Event created successfully"));
                if !created.alarms.is_empty() {
                    body.insert("alarms".into(), alarms_json(&created.alarms));
                }
                json_result(body)
            }
            Err(e) => {
                self.logger.warning(
                    "Event creation failed",
                    &[("tool", json!("create_event")), ("calendar", json!(calendar_id)), ("code", json!(e.code))],
                );
                error::service_error(e.code, &e.message)
            }
        }
    }

    pub(super) async fn update_event(&self, args: &Args) -> CallToolResult {
        let rate = self.rate_limiter.acquire_write_with_status();
        if !rate.allowed {
            return error::rate_limit_error(rate.retry_after_ms);
        }
        read_args! {
            event_id = args::string(args, "event_id");
            title = args::string(args, "title");
            start_time = args::string(args, "start_time");
            end_time = args::string(args, "end_time");
            location = args::string(args, "location");
            description = args::string(args, "description");
            start_date = args::string(args, "start_date");
            end_date = args::string(args, "end_date");
            timezone = args::string(args, "timezone");
            end_timezone = args::string(args, "end_timezone");
            rrule = args::string(args, "rrule");
            rdates = args::string_list(args, "rdates");
            exdates = args::string_list(args, "exdates");
            scope = args::string(args, "scope");
            alarms = args::alarms(args, "alarms");
        }
        let is_all_day = args::boolean(args, "is_all_day");

        let mut errors = collect_errors(&[v::validate_calendar_id(event_id.as_deref(), "event_id")]);
        if title.is_some() {
            errors.extend(v::validate_title(title.as_deref(), "title").err());
        }
        if start_time.is_some() {
            errors.extend(v::validate_date_time(start_time.as_deref(), "start_time").err());
        }
        if end_time.is_some() {
            errors.extend(v::validate_date_time(end_time.as_deref(), "end_time").err());
        }
        if let (Some(start), Some(end)) = (start_time.as_deref(), end_time.as_deref()) {
            errors.extend(v::validate_time_range(start, end, timezone.as_deref(), end_timezone.as_deref()).err());
        }
        if start_date.is_some() {
            errors.extend(v::validate_date(start_date.as_deref(), "start_date").err());
        }
        if end_date.is_some() {
            errors.extend(v::validate_date(end_date.as_deref(), "end_date").err());
        }
        errors.extend(collect_errors(&[
            v::validate_optional_text(location.as_deref(), "location", v::MAX_LOCATION_LENGTH),
            v::validate_optional_text(description.as_deref(), "description", v::MAX_DESCRIPTION_LENGTH),
            v::validate_timezone(timezone.as_deref(), "timezone"),
            v::validate_timezone(end_timezone.as_deref(), "end_timezone"),
            v::validate_recurrence_date_list(rdates.as_deref(), "rdates"),
            v::validate_recurrence_date_list(exdates.as_deref(), "exdates"),
            v::validate_alarm_list(alarms.as_deref(), "alarms"),
            v::validate_scope(scope.as_deref(), "scope"),
            // Mirrors the service-layer check: an occurrence scope must not
            // carry series-level fields.
            v::validate_occurrence_scope_fields(
                scope.as_deref(),
                rrule.as_deref(),
                rdates.as_deref(),
                exdates.as_deref(),
                "scope",
            ),
        ]));
        if !errors.is_empty() {
            return error::validation_error(&errors);
        }
        let Some(service) = self.service.as_ref() else {
            return error::authentication_error(NOT_CONFIGURED);
        };

        let event_id = event_id.unwrap_or_default();
        let patch = EventPatch {
            summary: title.as_deref().map(v::sanitize_for_ics),
            start_time,
            end_time,
            start_date,
            end_date,
            is_all_day,
            description: description.as_deref().map(v::sanitize_for_ics),
            location: location.as_deref().map(v::sanitize_for_ics),
            timezone,
            end_timezone,
            rrule,
            rdates,
            exdates,
            alarms: args::alarm_specs(alarms.as_deref()),
            ..EventPatch::default()
        };
        match service.update_event(&event_id, patch, EventScope::from_token(scope.as_deref())).await {
            Ok(updated) => {
                self.logger.info(
                    "Event updated",
                    &[("tool", json!("update_event")), ("uid", json!(updated.uid)), ("eventId", json!(event_id))],
                );
                let mut body = Map::new();
                body.insert("success".into(), json!(true));
                body.insert("uid".into(), json!(updated.uid));
                insert_opt(&mut body, "handle", updated.handle.as_deref());
                body.insert("message".into(), json!("Event updated successfully"));
                if !updated.alarms.is_empty() {
                    body.insert("alarms".into(), alarms_json(&updated.alarms));
                }
                json_result(body)
            }
            Err(e) => {
                self.logger.warning(
                    "Event update failed",
                    &[("tool", json!("update_event")), ("eventId", json!(event_id)), ("code", json!(e.code))],
                );
                error::service_error(e.code, &e.message)
            }
        }
    }

    pub(super) async fn delete_event(&self, args: &Args) -> CallToolResult {
        let rate = self.rate_limiter.acquire_write_with_status();
        if !rate.allowed {
            return error::rate_limit_error(rate.retry_after_ms);
        }
        read_args! {
            event_id = args::string(args, "event_id");
            scope = args::string(args, "scope");
        }
        let errors = collect_errors(&[
            v::validate_calendar_id(event_id.as_deref(), "event_id"),
            v::validate_scope(scope.as_deref(), "scope"),
        ]);
        if !errors.is_empty() {
            return error::validation_error(&errors);
        }
        let Some(service) = self.service.as_ref() else {
            return error::authentication_error(NOT_CONFIGURED);
        };
        let event_id = event_id.unwrap_or_default();
        match service.delete_event(&event_id, EventScope::from_token(scope.as_deref())).await {
            Ok(()) => {
                self.logger.info("Event deleted", &[("tool", json!("delete_event")), ("eventId", json!(event_id))]);
                let mut body = Map::new();
                body.insert("success".into(), json!(true));
                body.insert("message".into(), json!("Event deleted successfully"));
                json_result(body)
            }
            Err(e) => {
                self.logger.warning(
                    "Event deletion failed",
                    &[("tool", json!("delete_event")), ("eventId", json!(event_id)), ("code", json!(e.code))],
                );
                error::service_error(e.code, &e.message)
            }
        }
    }
}
