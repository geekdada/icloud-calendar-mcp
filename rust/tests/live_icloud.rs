//! Live round trips against real iCloud.
//!
//! Every test is `#[ignore]`d so `cargo test` stays hermetic. Opt in with
//!
//! ```text
//! ICLOUD_USERNAME=… ICLOUD_APP_PASSWORD=… cargo test --test live_icloud -- --ignored --test-threads=1
//! ```
//!
//! (`ICLOUD_PASSWORD` is accepted too, as is a `local.properties` file in the
//! crate or repository root.) Without credentials each test returns early.
//! Every event a test creates is prefixed `MCP-LIVE:` and deleted afterwards,
//! even when the test fails. Run single-threaded: the tests share one account.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use futures::FutureExt;
use icloud_calendar_mcp::caldav::{CalDavClient, CalDavCredentials, HttpCalDavClient};
use icloud_calendar_mcp::ics::{AlarmSpec, EventPatch, NewEvent};
use icloud_calendar_mcp::scope::EventScope;
use icloud_calendar_mcp::service::{CalendarService, EventInfo, handle};

const ICLOUD_BASE_URL: &str = "https://caldav.icloud.com";
const TEST_PREFIX: &str = "MCP-LIVE:";

fn credentials() -> Option<CalDavCredentials> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    if let (Some(user), Some(pass)) =
        (env("ICLOUD_USERNAME"), env("ICLOUD_PASSWORD").or_else(|| env("ICLOUD_APP_PASSWORD")))
    {
        return Some(CalDavCredentials::new(user, pass));
    }
    for path in ["local.properties", "../local.properties"] {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let prop = |key: &str| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        if let (Some(user), Some(pass)) =
            (prop("ICLOUD_USERNAME"), prop("ICLOUD_APP_PASSWORD").or_else(|| prop("ICLOUD_PASSWORD")))
        {
            return Some(CalDavCredentials::new(user, pass));
        }
    }
    None
}

fn new_client(creds: &CalDavCredentials) -> Arc<HttpCalDavClient> {
    Arc::new(HttpCalDavClient::new(ICLOUD_BASE_URL, creds.clone()).expect("client builds"))
}

/// Per-test context: a service on a writable calendar and a cleanup list.
struct Live {
    creds: CalDavCredentials,
    service: CalendarService,
    calendar_id: String,
    created: Mutex<Vec<String>>,
}

impl Live {
    fn prefixed(summary: &str) -> String {
        format!("{TEST_PREFIX} {summary}")
    }

    /// Track a reference (handle or UID) for deletion after the test.
    fn track(&self, reference: &str) {
        self.created.lock().unwrap().push(reference.to_string());
    }

    async fn create(&self, event: NewEvent) -> EventInfo {
        let event = NewEvent { summary: Self::prefixed(&event.summary), ..event };
        let info = self.service.create_event(&self.calendar_id, event).await.expect("create_event succeeds");
        self.track(info.handle.as_deref().unwrap_or(&info.uid));
        info
    }

    async fn occurrences(&self, uid: &str, from: chrono::NaiveDate, to: chrono::NaiveDate) -> Vec<EventInfo> {
        let events = self
            .service
            .get_events(&self.calendar_id, &from.to_string(), &to.to_string())
            .await
            .expect("get_events succeeds");
        events.into_iter().filter(|e| e.uid == uid).collect()
    }

    /// The stored ICS of `uid`, straight from iCloud through a fresh client.
    async fn raw_ics(&self, uid: &str, anchor: chrono::NaiveDate) -> Option<String> {
        let client = new_client(&self.creds);
        let events = client
            .get_events(
                &self.calendar_id,
                &(anchor - Duration::days(1)).to_string(),
                &(anchor + Duration::days(1)).to_string(),
            )
            .await
            .ok()?;
        events.into_iter().find(|e| e.uid == uid).map(|e| e.ical_data)
    }

    async fn cleanup(&self) {
        let references: Vec<String> = self.created.lock().unwrap().drain(..).rev().collect();
        for reference in references {
            let reference = match handle::EventHandle::decode(&reference) {
                // Delete whole resources; an occurrence handle is re-encoded as its master.
                Some(h) => handle::encode(&h.href, None, None),
                None => reference,
            };
            if let Err(e) = self.service.delete_event(&reference, None).await
                && e.code != 404
            {
                eprintln!("  ! cleanup delete failed: {} {}", e.code, e.message);
            }
        }
    }
}

/// Run `body` against iCloud, skipping without credentials and cleaning up
/// every tracked event whether or not the body panics.
async fn live_test<F, Fut>(body: F)
where
    F: FnOnce(Arc<Live>) -> Fut,
    Fut: Future<Output = ()>,
{
    let Some(creds) = credentials() else {
        eprintln!("iCloud credentials not set — skipping live test");
        return;
    };
    let service = CalendarService::new(new_client(&creds));
    let calendars = service.list_calendars().await.expect("list_calendars succeeds");
    let calendar_id = calendars
        .iter()
        .find(|c| !c.read_only && !c.id.is_empty() && !c.name.eq_ignore_ascii_case("Reminders"))
        .map(|c| c.id.clone())
        .expect("a writable calendar exists");
    let live = Arc::new(Live { creds, service, calendar_id, created: Mutex::new(Vec::new()) });
    let outcome = AssertUnwindSafe(body(live.clone())).catch_unwind().await;
    live.cleanup().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

fn day(offset: i64) -> chrono::NaiveDate {
    Utc::now().date_naive() + Duration::days(offset)
}

fn timed(summary: &str, date: chrono::NaiveDate, start: &str, end: &str) -> NewEvent {
    NewEvent {
        summary: summary.into(),
        start_time: Some(format!("{date}T{start}Z")),
        end_time: Some(format!("{date}T{end}Z")),
        ..NewEvent::default()
    }
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn lists_calendars() {
    live_test(|live| async move {
        let calendars = live.service.list_calendars().await.unwrap();
        assert!(!calendars.is_empty());
        assert!(calendars.iter().all(|c| !c.id.is_empty() && !c.name.is_empty()));
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn create_fetch_delete_with_crlf_and_timestamps() {
    live_test(|live| async move {
        let date = day(30);
        let created = live.create(timed("create-fetch-delete", date, "10:00:00", "11:00:00")).await;
        let raw = live.raw_ics(&created.uid, date).await.expect("event is stored");
        assert!(raw.contains("\r\n"), "stored body uses CRLF");
        assert!(raw.contains("CREATED:") && raw.contains("LAST-MODIFIED:"));
        live.service.delete_event(created.handle.as_deref().unwrap(), None).await.unwrap();
        assert!(live.raw_ics(&created.uid, date).await.is_none(), "deleted event is gone");
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn recurring_series_with_rdate_exdate_round_trips() {
    live_test(|live| async move {
        let date = day(40);
        let mut event = timed("recurring rdate/exdate", date, "09:00:00", "09:30:00");
        event.rrule = Some("FREQ=DAILY;COUNT=3".into());
        event.rdates = Some(vec![format!("{}T09:00:00Z", date + Duration::days(5))]);
        event.exdates = Some(vec![format!("{}T09:00:00Z", date + Duration::days(1))]);
        let created = live.create(event).await;
        let raw = live.raw_ics(&created.uid, date).await.expect("event is stored");
        assert!(raw.contains("DURATION:PT30M"), "{raw}");
        let occurrences = live.occurrences(&created.uid, date, date + Duration::days(6)).await;
        let days: Vec<String> = occurrences.iter().map(|o| o.start_time.clone().unwrap()[..10].to_string()).collect();
        assert_eq!(
            days,
            [date, date + Duration::days(2), date + Duration::days(5)].map(|d| d.to_string()),
            "RRULE minus EXDATE plus RDATE"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn two_alarms_round_trip() {
    live_test(|live| async move {
        let date = day(45);
        let mut event = timed("two alarms", date, "12:00:00", "13:00:00");
        event.alarms = Some(vec![
            AlarmSpec { trigger: "-PT15M".into(), ..AlarmSpec::default() },
            AlarmSpec { trigger: "-PT1H".into(), action: Some("AUDIO".into()), ..AlarmSpec::default() },
        ]);
        let created = live.create(event).await;
        assert_eq!(created.alarms.len(), 2);
        let raw = live.raw_ics(&created.uid, date).await.expect("event is stored");
        assert_eq!(raw.matches("BEGIN:VALARM").count(), 2, "{raw}");
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn description_only_update_preserves_summary() {
    live_test(|live| async move {
        let date = day(50);
        let created = live.create(timed("keep my title", date, "08:00:00", "09:00:00")).await;
        let patch = EventPatch { description: Some("only the description changes".into()), ..EventPatch::default() };
        let updated = live.service.update_event(created.handle.as_deref().unwrap(), patch, None).await.unwrap();
        assert!(updated.summary.ends_with("keep my title"));
        let raw = live.raw_ics(&created.uid, date).await.unwrap();
        assert!(raw.contains("keep my title") && raw.contains("only the description changes"), "{raw}");
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn stale_handle_recovers_through_412_retry() {
    live_test(|live| async move {
        let date = day(55);
        let created = live.create(timed("stale etag", date, "10:00:00", "10:30:00")).await;
        let stale = created.handle.clone().unwrap();
        let first = EventPatch { location: Some("first edit".into()), ..EventPatch::default() };
        live.service.update_event(&stale, first, None).await.unwrap();
        // The handle still carries the original ETag: the PUT trips a 412, the
        // service refetches and retries once.
        let second = EventPatch { location: Some("second edit".into()), ..EventPatch::default() };
        let result = live.service.update_event(&stale, second, None).await.unwrap();
        assert_eq!(result.location.as_deref(), Some("second edit"));
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn handle_works_from_a_cold_service() {
    live_test(|live| async move {
        let date = day(60);
        let created = live.create(timed("cold process", date, "15:00:00", "16:00:00")).await;
        let cold = CalendarService::new(new_client(&live.creds));
        let patch = EventPatch { summary: Some(Live::prefixed("cold process (edited)")), ..EventPatch::default() };
        let updated = cold.update_event(created.handle.as_deref().unwrap(), patch, None).await.unwrap();
        assert!(updated.summary.ends_with("(edited)"));
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn crlf_injection_is_stored_as_text() {
    live_test(|live| async move {
        let date = day(65);
        let created = live.create(timed("evil\r\nX-INJECTED:yes", date, "10:00:00", "11:00:00")).await;
        let raw = live.raw_ics(&created.uid, date).await.unwrap();
        assert!(!raw.contains("\r\nX-INJECTED:yes"), "{raw}");
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn backslash_and_commas_are_not_double_escaped() {
    live_test(|live| async move {
        let date = day(70);
        let mut event = timed(r"C:\Users\Name, notes; more", date, "10:00:00", "11:00:00");
        event.description = Some(r"path C:\Temp, a; b".into());
        let created = live.create(event).await;
        let events = live.occurrences(&created.uid, date, date).await;
        assert!(events[0].summary.ends_with(r"C:\Users\Name, notes; more"), "{:?}", events[0].summary);
        assert_eq!(events[0].description.as_deref(), Some(r"path C:\Temp, a; b"));
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn all_day_and_cross_zone_events() {
    live_test(|live| async move {
        let date = day(75);
        let all_day = live
            .create(NewEvent {
                summary: "all day".into(),
                is_all_day: true,
                start_date: Some(date.to_string()),
                end_date: Some((date + Duration::days(1)).to_string()),
                ..NewEvent::default()
            })
            .await;
        assert_eq!(all_day.start_date.as_deref(), Some(date.to_string().as_str()));
        assert_eq!(all_day.end_date.as_deref(), Some((date + Duration::days(1)).to_string().as_str()));

        let flight = live
            .create(NewEvent {
                summary: "flight".into(),
                start_time: Some(format!("{date}T09:00:00")),
                end_time: Some(format!("{date}T12:00:00")),
                timezone: Some("America/New_York".into()),
                end_timezone: Some("America/Los_Angeles".into()),
                ..NewEvent::default()
            })
            .await;
        let raw = live.raw_ics(&flight.uid, date).await.unwrap();
        assert!(raw.contains("TZID=America/New_York") && raw.contains("TZID=America/Los_Angeles"), "{raw}");
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn scoped_edit_and_delete_of_occurrences() {
    live_test(|live| async move {
        let start = day(90);
        let mut series = timed("scoped series", start, "09:00:00", "09:30:00");
        series.rrule = Some("FREQ=DAILY;COUNT=4".into());
        let created = live.create(series).await;
        let window = (start - Duration::days(1), start + Duration::days(5));
        let before = live.occurrences(&created.uid, window.0, window.1).await;
        assert_eq!(before.len(), 4);
        let on = |list: &[EventInfo], d: chrono::NaiveDate| {
            list.iter().find(|o| o.start_time.as_deref().unwrap().starts_with(&d.to_string())).cloned().unwrap()
        };

        let second = on(&before, start + Duration::days(1));
        let patch = EventPatch {
            start_time: Some(second.start_time.clone().unwrap().replace("T09:00", "T14:00")),
            end_time: Some(second.end_time.clone().unwrap().replace("T09:30", "T14:30")),
            ..EventPatch::default()
        };
        live.service
            .update_event(second.handle.as_deref().unwrap(), patch, Some(EventScope::ThisOccurrence))
            .await
            .unwrap();

        let third = on(&before, start + Duration::days(2));
        live.service.delete_event(third.handle.as_deref().unwrap(), Some(EventScope::ThisOccurrence)).await.unwrap();

        let after = live.occurrences(&created.uid, window.0, window.1).await;
        assert_eq!(after.len(), 3, "{after:?}");
        assert!(on(&after, start + Duration::days(1)).start_time.unwrap().contains("T14:00"));
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn this_and_future_edit_splits_the_series() {
    live_test(|live| async move {
        let start = day(110);
        let mut series = timed("split series", start, "09:00:00", "09:30:00");
        series.rrule = Some("FREQ=DAILY;COUNT=4".into());
        let created = live.create(series).await;
        let window = (start - Duration::days(1), start + Duration::days(5));
        let before = live.occurrences(&created.uid, window.0, window.1).await;
        let pivot_day = (start + Duration::days(2)).to_string();
        let pivot = before.iter().find(|o| o.start_time.as_deref().unwrap().starts_with(&pivot_day)).unwrap();

        let patch = EventPatch { summary: Some(Live::prefixed("split series (onward)")), ..EventPatch::default() };
        let split = live
            .service
            .update_event(pivot.handle.as_deref().unwrap(), patch, Some(EventScope::ThisAndFuture))
            .await
            .unwrap();
        live.track(split.handle.as_deref().unwrap());
        assert_ne!(split.uid, created.uid);

        let master = live.occurrences(&created.uid, window.0, window.1).await;
        assert_eq!(master.len(), 2, "{master:?}");
        let onward = live.occurrences(&split.uid, window.0, window.1).await;
        assert_eq!(onward.len(), 2, "{onward:?}");
        assert!(onward.iter().all(|o| o.summary.ends_with("(onward)")));
    })
    .await;
}

#[tokio::test]
#[ignore = "hits real iCloud; needs ICLOUD_USERNAME/ICLOUD_APP_PASSWORD"]
async fn double_delete_reports_not_found() {
    live_test(|live| async move {
        let created = live.create(timed("double delete", day(120), "10:00:00", "11:00:00")).await;
        live.service.delete_event(&created.uid, None).await.unwrap();
        // The UID resolved through the cache, which the first delete cleared, so
        // the second is a clean 404 rather than a server error.
        let second = live.service.delete_event(&created.uid, None).await.unwrap_err();
        assert_eq!(second.code, 404);
    })
    .await;
}
