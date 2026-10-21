//! User-initiated prompt templates.
//!
//! Prompts are menu entries a client shows its user; the model does not
//! consult them during tool use. Each expands to one user message that steers
//! the client through the right tool workflow. They are deliberately thin: the
//! tools remain the source of truth for what happens against iCloud.

use rmcp::model::{GetPromptResult, JsonObject, Prompt, PromptArgument, PromptMessage, Role};
use serde_json::Value;

/// Prompt definitions in registration order.
pub fn definitions() -> Vec<Prompt> {
    let arg = |name: &str, description: &str, required: bool| {
        PromptArgument::new(name).with_description(description).with_required(required)
    };
    vec![
        Prompt::new(
            "schedule_meeting",
            Some("Draft a calendar event from a title, attendees, and duration, then create it with create_event."),
            Some(vec![
                arg("title", "Meeting title", true),
                arg("attendees", "Comma-separated attendee emails", false),
                arg("duration", "Duration, e.g. 30m, 1h (default 30m)", false),
            ]),
        ),
        Prompt::new(
            "reschedule",
            Some(
                "Move an existing event to a new time by looking it up with get_events and editing it with update_event.",
            ),
            Some(vec![
                arg("event", "Title or description of the event to move", true),
                arg("new_time", "New start time (ISO 8601, e.g. 2026-02-01T15:00:00Z)", false),
            ]),
        ),
        Prompt::new(
            "find_conflicts",
            Some("List a day's events with get_events and reason about overlapping/conflicting time slots."),
            Some(vec![arg("date", "Day to check (YYYY-MM-DD)", true)]),
        ),
    ]
}

/// The expanded prompt `name`, or `None` for an unknown prompt.
pub fn render(name: &str, arguments: Option<&JsonObject>) -> Option<GetPromptResult> {
    let arg = |key: &str, fallback: &str| -> String {
        arguments
            .and_then(|a| a.get(key))
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Null => None,
                other => Some(other.to_string()),
            })
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string())
    };
    let (description, text) = match name {
        "schedule_meeting" => {
            let title = arg("title", "Untitled meeting");
            let attendees = arg("attendees", "(none specified)");
            let duration = arg("duration", "30m");
            ("Schedule a meeting via create_event", schedule_meeting_text(&title, &attendees, &duration))
        }
        "reschedule" => {
            let event = arg("event", "(describe the event)");
            let new_time = arg("new_time", "(ask the user for the new time)");
            (
                "Reschedule an event via the get_events → update_event handle round-trip",
                reschedule_text(&event, &new_time),
            )
        }
        "find_conflicts" => {
            let date = arg("date", "(ask the user which day)");
            ("Find scheduling conflicts on a given day", find_conflicts_text(&date))
        }
        _ => return None,
    };
    let mut result = GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]);
    result.description = Some(description.to_string());
    Some(result)
}

fn schedule_meeting_text(title: &str, attendees: &str, duration: &str) -> String {
    format!(
        "Schedule a calendar meeting using the create_event tool.

Details:
- Title: {title}
- Attendees: {attendees}
- Duration: {duration}

Steps:
1. If you don't already know the target calendar, call list_calendars and pick the user's default writable calendar.
2. Choose a concrete start_time from the conversation (ask if the user hasn't given one). Compute end_time from the {duration} duration.
3. Call create_event with calendar_id, title=\"{title}\", start_time, end_time, and a sensible timezone (IANA, e.g. America/New_York) and a default reminder if the user wants one.
4. Mention the attendees ({attendees}) in the description; note that this server does not send invitations.
5. Treat the create_event success response as authoritative — use the returned uid/handle and do NOT re-list to verify (that risks duplicates)."
    )
}

fn reschedule_text(event: &str, new_time: &str) -> String {
    format!(
        "Reschedule an existing calendar event.

Target event: {event}
New start time: {new_time}

Steps:
1. Call get_events over a date range that should contain \"{event}\" and identify the matching event.
2. Take the opaque `handle` from that event in the get_events result — do NOT reconstruct an id yourself; the handle is what update_event needs and it survives a cold process.
3. Call update_event with event_id set to that handle and start_time={new_time} (adjust end_time to preserve the original duration).
4. update_event returns a refreshed handle carrying the new etag — keep it for any follow-up edit.
5. If update_event reports a conflict (stale etag), re-run get_events to get the current handle and retry once."
    )
}

fn find_conflicts_text(date: &str) -> String {
    format!(
        "Find scheduling conflicts on {date}.

Steps:
1. Call get_events with start_date={date} and end_date={date} (or the day after, if the client treats end_date as exclusive) for the user's calendars.
2. Sort the returned events by start time and compare adjacent events.
3. Report any pairs whose time ranges overlap as conflicts, plus any back-to-back events with no gap that the user may want to space out.
4. For all-day events, note overlaps with timed events separately (they rarely constitute a hard conflict).
5. Present the conflicts clearly; do not modify anything unless the user asks."
    )
}
