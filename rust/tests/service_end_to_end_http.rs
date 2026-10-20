//! Service-level port of EndToEndIntegrationTest: CalendarService driving the
//! real HTTP CalDAV client against a scripted local server, through
//! list → create → get → update (title only) → delete, checking that unmodelled
//! properties survive the round trip on the wire.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use icloud_calendar_mcp::caldav::{CalDavCredentials, HttpCalDavClient};
use icloud_calendar_mcp::ics::{EventPatch, NewEvent};
use icloud_calendar_mcp::service::CalendarService;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Replays canned responses in order; an exhausted queue answers 404.
#[derive(Clone, Default)]
struct Queue(Arc<Mutex<VecDeque<ResponseTemplate>>>);

impl Queue {
    fn push(&self, response: ResponseTemplate) {
        self.0.lock().unwrap().push_back(response);
    }
}

impl Respond for Queue {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0.lock().unwrap().pop_front().unwrap_or_else(|| ResponseTemplate::new(404))
    }
}

const PRINCIPAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/</D:href><D:propstat>
<D:prop><D:current-user-principal><D:href>/123/principal/</D:href></D:current-user-principal></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;

const HOME_SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/123/principal/</D:href>
<D:propstat><D:prop><C:calendar-home-set><D:href>/123/calendars/</D:href></C:calendar-home-set></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;

const CALENDAR_LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Home</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;

/// What another client (Apple Calendar) stored: attendees, an alarm and X-props.
const RICH_ICS: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Apple Inc.//iCloud//EN\nBEGIN:VEVENT\nUID:e2e-test-001@icloud-calendar-mcp\nDTSTAMP:20250101T000000Z\nDTSTART:20250115T100000Z\nDTEND:20250115T110000Z\nSUMMARY:E2E Meeting\nLOCATION:Conference Room B\nSTATUS:CONFIRMED\nCATEGORIES:Work,Testing\nRRULE:FREQ=WEEKLY;COUNT=4\nORGANIZER;CN=Alice:mailto:alice@example.com\nATTENDEE;CN=Bob;PARTSTAT=ACCEPTED:mailto:bob@example.com\nATTENDEE;CN=Carol:mailto:carol@example.com\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nDESCRIPTION:Reminder\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR";

const EVENT_PATH: &str = "/123/calendars/home/e2e-test-001.ics";

fn report(ics: &str, etag: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>{EVENT_PATH}</D:href>
<D:propstat><D:prop><D:getetag>"{etag}"</D:getetag><C:calendar-data>{ics}</C:calendar-data></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
    )
}

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers.get(name).and_then(|v| v.to_str().ok())
}

fn body(request: &Request) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

#[tokio::test]
async fn service_round_trip_over_http_preserves_foreign_properties() {
    let server = MockServer::start().await;
    let queue = Queue::default();
    Mock::given(any()).respond_with(queue.clone()).mount(&server).await;

    let builder = || reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_millis(500));
    let client = HttpCalDavClient::with_clients(
        server.uri(),
        CalDavCredentials::new("test@icloud.com", "test-password-1234"),
        builder().build().unwrap(),
        builder().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
    );
    client.skip_well_known_discovery();
    let service = CalendarService::new(Arc::new(client));

    // ── list: OPTIONS connection check + three-step discovery ──
    queue.push(ResponseTemplate::new(200).insert_header("DAV", "1, 2, calendar-access"));
    queue.push(ResponseTemplate::new(207).set_body_string(PRINCIPAL));
    queue.push(ResponseTemplate::new(207).set_body_string(HOME_SET));
    queue.push(ResponseTemplate::new(207).set_body_string(CALENDAR_LIST));
    let calendars = service.list_calendars().await.unwrap();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0].id, "home");

    // ── create: discovery cached, one conditional PUT ──
    queue.push(ResponseTemplate::new(201).insert_header("ETag", "\"etag-v0\""));
    let created = service
        .create_event(
            "home",
            NewEvent {
                summary: "Created via MCP".into(),
                start_time: Some("2025-01-20T09:00:00Z".into()),
                end_time: Some("2025-01-20T10:00:00Z".into()),
                location: Some("Room 1".into()),
                ..NewEvent::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(created.etag.as_deref(), Some("etag-v0"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 5, "1 OPTIONS + 3 discovery + 1 PUT");
    let put = &requests[4];
    assert_eq!(put.method.as_str(), "PUT");
    assert_eq!(header(put, "If-None-Match"), Some("*"));
    assert!(put.url.path().starts_with("/123/calendars/home/") && put.url.path().ends_with(".ics"));
    let put_body = body(put);
    assert!(put_body.contains("SUMMARY:Created via MCP"));
    assert!(put_body.contains("LOCATION:Room 1"));
    assert!(put_body.contains("STATUS:CONFIRMED") && put_body.contains("SEQUENCE:0"), "iCloud-required properties");

    // ── get: one REPORT returning the foreign event, expanded per occurrence ──
    queue.push(ResponseTemplate::new(207).set_body_string(report(RICH_ICS, "etag-v1")));
    let events = service.get_events("home", "2025-01-15", "2025-01-31").await.unwrap();
    assert_eq!(events.len(), 3, "weekly occurrences on 01-15, 01-22 and 01-29");
    let first = &events[0];
    assert_eq!(first.uid, "e2e-test-001@icloud-calendar-mcp");
    assert_eq!(first.organizer.as_deref(), Some("Alice <alice@example.com>"));
    assert_eq!(first.attendee_count, 2);
    assert_eq!(first.categories, ["Work", "Testing"]);
    let requests = server.received_requests().await.unwrap();
    let report_request = &requests[5];
    assert_eq!(report_request.method.as_str(), "REPORT");
    assert!(body(report_request).contains("calendar-data"));

    // ── update the whole series (title only) via the first occurrence's handle ──
    queue.push(ResponseTemplate::new(200).insert_header("ETag", "\"etag-v1\"").set_body_string(RICH_ICS));
    queue.push(ResponseTemplate::new(204).insert_header("ETag", "\"etag-v2\""));
    let updated = service
        .update_event(
            first.handle.as_deref().unwrap(),
            EventPatch { summary: Some("Updated E2E Meeting".into()), ..EventPatch::default() },
            Some(icloud_calendar_mcp::scope::EventScope::AllEvents),
        )
        .await
        .unwrap();
    assert_eq!(updated.etag.as_deref(), Some("etag-v2"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 8, "+1 GET +1 PUT");
    assert_eq!(requests[6].method.as_str(), "GET");
    assert_eq!(requests[6].url.path(), EVENT_PATH);
    let update = &requests[7];
    assert_eq!(update.method.as_str(), "PUT");
    assert!(header(update, "If-Match").is_some_and(|v| v.contains("etag-v1")), "conditional on the version listed");
    let update_body = body(update);
    assert!(update_body.contains("SUMMARY:Updated E2E Meeting"));
    for preserved in [
        "LOCATION:Conference Room B",
        "STATUS:CONFIRMED",
        "CATEGORIES:Work,Testing",
        "RRULE:FREQ=WEEKLY;COUNT=4",
        "ORGANIZER;CN=Alice:mailto:alice@example.com",
        "ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED:mailto:bob@example.com",
        "carol@example.com",
        "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC",
        "BEGIN:VALARM",
        "TRIGGER:-PT15M",
        "SEQUENCE:1",
    ] {
        assert!(update_body.contains(preserved), "{preserved} preserved:\n{update_body}");
    }

    // ── delete with the refreshed handle ──
    queue.push(ResponseTemplate::new(204));
    service.delete_event(updated.handle.as_deref().unwrap(), None).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 9);
    let delete = &requests[8];
    assert_eq!(delete.method.as_str(), "DELETE");
    assert_eq!(delete.url.path(), EVENT_PATH);
    assert!(header(delete, "If-Match").is_some_and(|v| v.contains("etag-v2")));
}

#[tokio::test]
async fn stale_handle_update_over_http_retries_once_with_the_fresh_etag() {
    let server = MockServer::start().await;
    let queue = Queue::default();
    Mock::given(any()).respond_with(queue.clone()).mount(&server).await;
    let client = HttpCalDavClient::with_clients(
        server.uri(),
        CalDavCredentials::new("test@icloud.com", "test-password-1234"),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
    );
    client.skip_well_known_discovery();
    let service = CalendarService::new(Arc::new(client));

    let token = icloud_calendar_mcp::service::handle::encode(EVENT_PATH, Some("etag-old"), None);
    queue.push(ResponseTemplate::new(200).insert_header("ETag", "\"etag-new\"").set_body_string(RICH_ICS));
    queue.push(ResponseTemplate::new(412));
    queue.push(ResponseTemplate::new(200).insert_header("ETag", "\"etag-new\"").set_body_string(RICH_ICS));
    queue.push(ResponseTemplate::new(204).insert_header("ETag", "\"etag-v3\""));

    let updated = service
        .update_event(&token, EventPatch { location: Some("Moved".into()), ..EventPatch::default() }, None)
        .await
        .unwrap();
    assert_eq!(updated.etag.as_deref(), Some("etag-v3"));

    let requests = server.received_requests().await.unwrap();
    let methods: Vec<&str> = requests.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["GET", "PUT", "GET", "PUT"]);
    assert!(
        header(&requests[1], "If-Match").is_some_and(|v| v.contains("etag-old")),
        "first PUT uses the handle's etag"
    );
    assert!(header(&requests[3], "If-Match").is_some_and(|v| v.contains("etag-new")), "retry uses the refetched etag");
    assert!(body(&requests[3]).contains("LOCATION:Moved"));
}
