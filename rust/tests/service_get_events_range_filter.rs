//! Port of GetEventsRangeFilterTest: get_events re-checks what the server sent
//! against the requested range (RFC 4791 §9.9 overlap), after recurrence expansion.
//!
//! The mock returns whatever it is given regardless of the requested dates,
//! which is the server misbehavior being defended against.

mod support;

use std::sync::Arc;

use icloud_calendar_mcp::caldav::CalDavEvent;
use icloud_calendar_mcp::service::{CalendarService, EventInfo};
use support::*;

const CALENDAR_ID: &str = "cal-1";

fn setup() -> (Arc<MockCalDavClient>, CalendarService) {
    let mock = MockCalDavClient::with_calendars(vec![calendar(CALENDAR_ID, "Private")]);
    let service = CalendarService::new(mock.clone());
    (mock, service)
}

fn serve(mock: &MockCalDavClient, vevents: &[&str]) {
    mock.state().events_response = vevents
        .iter()
        .enumerate()
        .map(|(i, body)| CalDavEvent {
            uid: format!("uid-{i}"),
            href: format!("/caldav/cal-1/uid-{i}.ics"),
            url: format!("https://caldav.icloud.com/caldav/cal-1/uid-{i}.ics"),
            etag: Some(format!("\"etag-{i}\"")),
            ical_data: format!("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\n{body}\nEND:VCALENDAR"),
        })
        .collect();
}

fn all_day(uid: &str, start: &str, end: &str, summary: &str) -> String {
    format!(
        "BEGIN:VEVENT\nUID:{uid}\nDTSTART;VALUE=DATE:{start}\nDTEND;VALUE=DATE:{end}\nSUMMARY:{summary}\nEND:VEVENT"
    )
}

fn timed(uid: &str, start: &str, end: &str, summary: &str) -> String {
    format!("BEGIN:VEVENT\nUID:{uid}\nDTSTART:{start}\nDTEND:{end}\nSUMMARY:{summary}\nEND:VEVENT")
}

async fn get(service: &CalendarService, start: &str, end: &str) -> Vec<EventInfo> {
    service.get_events(CALENDAR_ID, start, end).await.expect("get_events succeeds")
}

const TWO_DAY: &str = "BEGIN:VEVENT\nUID:boundary@example.com\nDTSTART;VALUE=DATE:20260725\nDTEND;VALUE=DATE:20260727\nSUMMARY:Two-day all-day event\nEND:VEVENT";

const YEARLY: &str = "BEGIN:VEVENT\nUID:yearly@example.com\nDTSTART;VALUE=DATE:20230517\nDTEND;VALUE=DATE:20230518\nRRULE:FREQ=YEARLY\nSUMMARY:Recurring yearly event\nEND:VEVENT";

#[tokio::test]
async fn all_day_event_whose_exclusive_end_touches_query_start_is_excluded() {
    let (mock, service) = setup();
    serve(&mock, &[TWO_DAY]);
    assert!(get(&service, "2026-07-27", "2026-07-27").await.is_empty(), "ended on 7/26");
}

#[tokio::test]
async fn same_event_is_returned_on_the_days_it_occupies() {
    let (mock, service) = setup();
    serve(&mock, &[TWO_DAY]);
    assert_eq!(get(&service, "2026-07-25", "2026-07-25").await.len(), 1);
    assert_eq!(get(&service, "2026-07-26", "2026-07-26").await.len(), 1);
    assert!(get(&service, "2026-07-24", "2026-07-24").await.is_empty());
}

#[tokio::test]
async fn recurring_event_in_range_is_kept_despite_old_master_start() {
    let (mock, service) = setup();
    serve(&mock, &[YEARLY]);
    let events = get(&service, "2026-05-17", "2026-05-17").await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].start_date.as_deref(), Some("2026-05-17"));
}

#[tokio::test]
async fn recurring_event_not_in_range_is_excluded() {
    let (mock, service) = setup();
    serve(&mock, &[YEARLY]);
    assert!(get(&service, "2026-05-18", "2026-05-18").await.is_empty());
}

#[tokio::test]
async fn overnight_occurrence_spanning_into_the_day_is_returned() {
    let (mock, service) = setup();
    serve(
        &mock,
        &[
            "BEGIN:VEVENT\nUID:overnight@example.com\nDTSTART:20260601T220000Z\nDTEND:20260602T060000Z\nRRULE:FREQ=DAILY\nSUMMARY:Overnight shift\nEND:VEVENT",
        ],
    );
    let mut starts: Vec<String> =
        get(&service, "2026-06-10", "2026-06-10").await.into_iter().filter_map(|e| e.start_time).collect();
    starts.sort();
    assert_eq!(starts, ["2026-06-09T22:00:00Z", "2026-06-10T22:00:00Z"]);
}

#[tokio::test]
async fn widened_full_range_daily_series_does_not_413() {
    let (mock, service) = setup();
    serve(
        &mock,
        &[
            "BEGIN:VEVENT\nUID:year-daily@example.com\nDTSTART:20260101T090000Z\nDTEND:20260101T100000Z\nRRULE:FREQ=DAILY\nSUMMARY:Daily standup\nEND:VEVENT",
        ],
    );
    let events = get(&service, "2026-01-01", "2026-06-30").await;
    assert_eq!(events.len(), 181, "one occurrence per day, no leading double count");
}

#[tokio::test]
async fn pathological_sub_daily_series_with_near_year_duration_is_413() {
    let (mock, service) = setup();
    serve(
        &mock,
        &[
            "BEGIN:VEVENT\nUID:pathological@example.com\nDTSTART:20260101T000000Z\nDTEND:20261215T000000Z\nRRULE:FREQ=MINUTELY\nSUMMARY:Degenerate overlapping series\nEND:VEVENT",
        ],
    );
    let err = service.get_events(CALENDAR_ID, "2026-06-01", "2026-06-01").await.unwrap_err();
    assert_eq!(err.code, 413);
}

#[tokio::test]
async fn single_and_multi_day_all_day_events_on_the_day_are_kept() {
    let (mock, service) = setup();
    serve(&mock, &[&all_day("single@example.com", "20260727", "20260728", "One-day")]);
    assert_eq!(get(&service, "2026-07-27", "2026-07-27").await.len(), 1);
    serve(&mock, &[&all_day("spanning@example.com", "20260726", "20260730", "Spanning")]);
    assert_eq!(get(&service, "2026-07-27", "2026-07-27").await.len(), 1);
}

#[tokio::test]
async fn timed_event_inside_the_day_is_kept() {
    let (mock, service) = setup();
    serve(&mock, &[&timed("timed@example.com", "20260727T090000Z", "20260727T100000Z", "Morning meeting")]);
    assert_eq!(get(&service, "2026-07-27", "2026-07-27").await.len(), 1);
}

#[tokio::test]
async fn timed_event_starting_at_next_midnight_is_excluded() {
    let (mock, service) = setup();
    serve(&mock, &[&timed("next@example.com", "20260728T000000Z", "20260728T010000Z", "Just after")]);
    assert!(get(&service, "2026-07-27", "2026-07-27").await.is_empty());
}

#[tokio::test]
async fn timed_event_ending_at_the_days_midnight_is_excluded() {
    let (mock, service) = setup();
    serve(&mock, &[&timed("prev@example.com", "20260726T230000Z", "20260727T000000Z", "Ends as window opens")]);
    assert!(get(&service, "2026-07-27", "2026-07-27").await.is_empty());
}

#[tokio::test]
async fn multi_day_query_keeps_every_event_within_it() {
    let (mock, service) = setup();
    serve(
        &mock,
        &[
            &all_day("a@example.com", "20260727", "20260728", "Day one"),
            &all_day("b@example.com", "20260729", "20260730", "Day three"),
            &all_day("c@example.com", "20260801", "20260802", "Outside"),
        ],
    );
    let mut summaries: Vec<String> =
        get(&service, "2026-07-27", "2026-07-30").await.into_iter().map(|e| e.summary).collect();
    summaries.sort();
    assert_eq!(summaries, ["Day one", "Day three"]);
}

#[tokio::test]
async fn end_date_is_inclusive() {
    let (mock, service) = setup();
    serve(&mock, &[&all_day("on-end@example.com", "20260730", "20260731", "On the last day")]);
    assert_eq!(get(&service, "2026-07-27", "2026-07-30").await.len(), 1);
}
