//! Port of EventCacheTest: UID cache TTL, capacity and concurrency.

mod support;

use std::sync::Arc;
use std::time::Duration;

use icloud_calendar_mcp::caldav::CalDavEvent;
use icloud_calendar_mcp::service::CalendarService;
use support::*;

fn event(uid: &str, summary: &str) -> CalDavEvent {
    caldav_event(uid, &vcalendar(&timed_vevent(uid, summary, "20250115T090000Z", "20250115T100000Z")))
}

fn service_with(mock: &Arc<MockCalDavClient>, ttl: Duration, max: usize) -> CalendarService {
    CalendarService::with_cache(mock.clone(), ttl, max)
}

#[tokio::test]
async fn cache_expires_entries_after_ttl() {
    let mock = MockCalDavClient::new();
    mock.state().events_response = vec![event("test-uid-123", "Test Event")];
    let service = service_with(&mock, Duration::from_millis(200), 100);

    service.get_events("test-calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert!(service.get_event_by_id("test-uid-123").is_ok());

    tokio::time::sleep(Duration::from_millis(400)).await;
    let err = service.get_event_by_id("test-uid-123").unwrap_err();
    assert_eq!(err.code, 404);
}

#[tokio::test]
async fn cache_clears_expired_entries_at_capacity() {
    let mock = MockCalDavClient::new();
    mock.state().events_response = (1..=3).map(|i| event(&format!("uid-{i}"), &format!("Event {i}"))).collect();
    let service = service_with(&mock, Duration::from_millis(200), 3);

    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert_eq!(service.cache_size(), 3);

    tokio::time::sleep(Duration::from_millis(400)).await;
    mock.state().events_response = (4..=5).map(|i| event(&format!("uid-{i}"), &format!("Event {i}"))).collect();
    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert!(service.cache_size() <= 3, "cache should not exceed max after clearing expired: {}", service.cache_size());
}

#[tokio::test]
async fn clear_cache_removes_all_entries() {
    let mock = MockCalDavClient::new();
    mock.state().events_response = vec![event("clear-test-uid", "Test Event")];
    let service = service_with(&mock, Duration::from_secs(60), 100);

    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    assert!(service.cache_size() > 0);
    service.clear_cache();
    assert_eq!(service.cache_size(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cache_is_safe_under_concurrent_access() {
    let mock = MockCalDavClient::new();
    mock.state().events_response =
        (1..=100).map(|i| event(&format!("concurrent-uid-{i}"), &format!("Event {i}"))).collect();
    let service = Arc::new(service_with(&mock, Duration::from_secs(60), 1000));

    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let service = service.clone();
            tokio::spawn(async move {
                service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
                for i in 1..=10 {
                    let _ = service.get_event_by_id(&format!("concurrent-uid-{i}"));
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(service.cache_size(), 100);
}

#[tokio::test]
async fn cached_events_are_retrievable_within_ttl() {
    let mock = MockCalDavClient::new();
    mock.state().events_response = vec![event("retrieve-test-uid", "Retrievable Event")];
    let service = service_with(&mock, Duration::from_secs(60), 100);

    service.get_events("calendar", "2025-01-01", "2025-01-31").await.unwrap();
    let info = service.get_event_by_id("retrieve-test-uid").unwrap();
    assert_eq!(info.summary, "Retrievable Event");
}
