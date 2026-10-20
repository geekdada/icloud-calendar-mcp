//! Full-stack end-to-end: MCP tool calls through the server, the service and
//! the real HTTP CalDAV client against a scripted mock CalDAV server.
//! Port of `EndToEndIntegrationTest.kt` (which drove the HTTP client alone),
//! lifted to the tool level: create → get → update (preserving unmodelled
//! properties) → delete, plus the ETag-only REPORT and the 412 conflict.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use icloud_calendar_mcp::caldav::{CalDavClient, CalDavCredentials, HttpCalDavClient};
use icloud_calendar_mcp::logging::{McpLogger, MemorySink};
use icloud_calendar_mcp::ratelimit::RateLimiter;
use icloud_calendar_mcp::server::CalendarServer;
use icloud_calendar_mcp::service::CalendarService;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

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

fn multistatus(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(207).set_body_string(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">{body}</D:multistatus>"#
    ))
}

fn discovery(queue: &Queue) {
    queue.push(multistatus(
        "<D:response><D:href>/</D:href><D:propstat><D:prop><D:current-user-principal><D:href>/123/principal/</D:href></D:current-user-principal></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
    ));
    queue.push(multistatus(
        "<D:response><D:href>/123/principal/</D:href><D:propstat><D:prop><C:calendar-home-set><D:href>/123/calendars/</D:href></C:calendar-home-set></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
    ));
    queue.push(multistatus(
        "<D:response><D:href>/123/calendars/home/</D:href><D:propstat><D:prop><D:displayname>Home</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
    ));
}

fn report(href: &str, etag: &str, ics: &str) -> ResponseTemplate {
    let safe = ics.replace('&', "&amp;").replace('<', "&lt;");
    multistatus(&format!(
        "<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:getetag>\"{etag}\"</D:getetag><C:calendar-data>{safe}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
    ))
}

struct Stack {
    mock: MockServer,
    queue: Queue,
    client: Arc<HttpCalDavClient>,
    server: CalendarServer,
    sink: Arc<MemorySink>,
}

impl Stack {
    async fn start() -> Self {
        let mock = MockServer::start().await;
        let queue = Queue::default();
        Mock::given(any()).respond_with(queue.clone()).mount(&mock).await;
        let builder = || {
            reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_millis(500))
                .read_timeout(Duration::from_secs(2))
        };
        let client = Arc::new(HttpCalDavClient::with_clients(
            mock.uri(),
            CalDavCredentials::new("test@icloud.com", "test-password-1234"),
            builder().build().unwrap(),
            builder().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
        ));
        client.skip_well_known_discovery();
        let sink = Arc::new(MemorySink::new());
        let server = CalendarServer::with_parts(
            Some(Arc::new(CalendarService::new(client.clone()))),
            Arc::new(RateLimiter::new(1000, 1000, 60_000)),
            Arc::new(McpLogger::new(sink.clone())),
        );
        Self { mock, queue, client, server, sink }
    }

    async fn call(&self, name: &str, args: Value) -> CallToolResult {
        self.server.call_tool_by_name(name, args.as_object().cloned()).await.unwrap()
    }

    async fn requests(&self) -> Vec<Request> {
        self.mock.received_requests().await.unwrap()
    }
}

fn text(result: &CallToolResult) -> String {
    result.content[0].as_text().unwrap().text.clone()
}

fn body(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{}", text(result));
    serde_json::from_str(&text(result)).unwrap()
}

const SERVER_SIDE_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Apple Inc.//iCloud//EN\r\nBEGIN:VEVENT\r\nUID:e2e-test-001@icloud-calendar-mcp\r\nDTSTAMP:20250115T100000Z\r\nDTSTART:20250115T100000Z\r\nDTEND:20250115T110000Z\r\nSUMMARY:E2E Meeting\r\nDESCRIPTION:End-to-end test event\r\nLOCATION:Conference Room B\r\nSTATUS:CONFIRMED\r\nCATEGORIES:Work,Testing\r\nORGANIZER;CN=Alice:mailto:alice@example.com\r\nATTENDEE;CN=Bob:mailto:bob@example.com\r\nATTENDEE;CN=Carol:mailto:carol@example.com\r\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:15 min reminder\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[tokio::test]
async fn create_get_update_delete_through_the_tools() {
    let stack = Stack::start().await;
    let q = &stack.queue;

    // ── create: discovery + PUT ──
    discovery(q);
    q.push(ResponseTemplate::new(201).insert_header("ETag", "\"etag-v1\""));
    let created = body(
        &stack
            .call(
                "create_event",
                json!({"calendar_id": "home", "title": "E2E Meeting", "start_time": "2025-01-15T10:00:00Z", "end_time": "2025-01-15T11:00:00Z",
                       "location": "Conference Room B", "description": "End-to-end test event", "rrule": "FREQ=WEEKLY;BYDAY=WE",
                       "alarms": [{"trigger": "-PT15M"}]}),
            )
            .await,
    );
    assert_eq!(created["summary"], "E2E Meeting");
    assert_eq!(created["alarms"][0]["trigger"], "-PT15M");
    let requests = stack.requests().await;
    assert_eq!(requests.len(), 4, "3 discovery + 1 PUT");
    let put = &requests[3];
    assert_eq!(put.method.as_str(), "PUT");
    assert_eq!(put.headers.get("If-None-Match").unwrap(), "*");
    assert!(put.headers.get("Authorization").unwrap().to_str().unwrap().starts_with("Basic "));
    let put_body = String::from_utf8(put.body.clone()).unwrap();
    for needle in [
        "SUMMARY:E2E Meeting",
        "LOCATION:Conference Room B",
        "STATUS:CONFIRMED",
        "RRULE:FREQ=WEEKLY;BYDAY=WE",
        "BEGIN:VALARM",
        "CALSCALE:GREGORIAN",
        "SEQUENCE:0",
    ] {
        assert!(put_body.contains(needle), "PUT body lacks {needle}");
    }
    assert!(put.url.path().starts_with("/123/calendars/home/") && put.url.path().ends_with(".ics"));

    // ── get_events: one REPORT (discovery cached), server body with extra properties ──
    let href = "/123/calendars/home/e2e-test-001.ics";
    q.push(report(href, "etag-v1", SERVER_SIDE_ICS));
    let listed = body(
        &stack
            .call("get_events", json!({"calendar_id": "home", "start_date": "2025-01-15", "end_date": "2025-01-31"}))
            .await,
    );
    let events = listed["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["uid"], "e2e-test-001@icloud-calendar-mcp");
    assert_eq!(event["location"], "Conference Room B");
    assert_eq!(event["categories"], json!(["Work", "Testing"]));
    assert_eq!(event["organizer"], "Alice <alice@example.com>");
    assert_eq!(event["attendeeCount"], 2);
    let requests = stack.requests().await;
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[4].method.as_str(), "REPORT");
    assert_eq!(requests[4].headers.get("Depth").unwrap(), "1");
    let report_body = String::from_utf8(requests[4].body.clone()).unwrap();
    assert!(report_body.contains("calendar-data"));
    assert!(report_body.contains(r#"start="20250115T000000Z""#) && report_body.contains(r#"end="20250131T235959Z""#));
    let handle = event["handle"].as_str().unwrap().to_string();

    // ── update_event via the handle: GET current body, then PUT with the handle's etag ──
    q.push(ResponseTemplate::new(200).insert_header("ETag", "\"etag-v1\"").set_body_string(SERVER_SIDE_ICS));
    q.push(ResponseTemplate::new(204).insert_header("ETag", "\"etag-v2\""));
    let updated = body(&stack.call("update_event", json!({"event_id": handle, "title": "Updated E2E Meeting"})).await);
    assert_eq!(updated["uid"], "e2e-test-001@icloud-calendar-mcp");
    let requests = stack.requests().await;
    assert_eq!(requests.len(), 7, "GET + PUT");
    assert_eq!(requests[5].method.as_str(), "GET");
    assert_eq!(requests[5].url.path(), href);
    let update = &requests[6];
    assert_eq!(update.method.as_str(), "PUT");
    assert_eq!(update.headers.get("If-Match").unwrap(), "etag-v1");
    let update_body = String::from_utf8(update.body.clone()).unwrap();
    for needle in [
        "SUMMARY:Updated E2E Meeting",
        "LOCATION:Conference Room B",
        "STATUS:CONFIRMED",
        "CATEGORIES:Work,Testing",
        "ORGANIZER;CN=Alice:mailto:alice@example.com",
        "ATTENDEE;CN=Bob:mailto:bob@example.com",
        "ATTENDEE;CN=Carol:mailto:carol@example.com",
        "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC",
        "BEGIN:VALARM",
        "TRIGGER:-PT15M",
        "SEQUENCE:1",
    ] {
        assert!(update_body.contains(needle), "update lost {needle}:\n{update_body}");
    }
    let new_handle = updated["handle"].as_str().unwrap().to_string();
    assert_ne!(new_handle, handle);

    // ── delete_event via the refreshed handle: DELETE with the new etag ──
    q.push(ResponseTemplate::new(204));
    let deleted = body(&stack.call("delete_event", json!({"event_id": new_handle})).await);
    assert_eq!(deleted["success"], true);
    let requests = stack.requests().await;
    assert_eq!(requests.len(), 8);
    let delete = &requests[7];
    assert_eq!(delete.method.as_str(), "DELETE");
    assert_eq!(delete.headers.get("If-Match").unwrap(), "etag-v2");
    assert_eq!(delete.url.path(), href);

    assert_eq!(stack.sink.count(), 3, "create, update and delete are audited");
}

#[tokio::test]
async fn list_calendars_checks_the_connection_once() {
    let stack = Stack::start().await;
    stack.queue.push(ResponseTemplate::new(200).insert_header("DAV", "1, 2, calendar-access"));
    discovery(&stack.queue);
    let listed = body(&stack.call("list_calendars", json!({})).await);
    assert_eq!(listed["calendars"][0]["id"], "home");
    assert_eq!(listed["calendars"][0]["name"], "Home");
    assert_eq!(listed["calendars"][0]["readOnly"], false);
    // Calendars are cached and the connection is validated once.
    let again = body(&stack.call("list_calendars", json!({})).await);
    assert_eq!(again, listed);
    let requests = stack.requests().await;
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method.as_str(), "OPTIONS");
}

#[tokio::test]
async fn non_caldav_server_is_reported() {
    let stack = Stack::start().await;
    stack.queue.push(ResponseTemplate::new(200).insert_header("DAV", "1, 2"));
    let result = stack.call("list_calendars", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("VALIDATION_ERROR") && text(&result).contains("calendar-access"),
        "{}",
        text(&result)
    );
}

#[tokio::test]
async fn authentication_failure_is_reported_without_credentials() {
    let stack = Stack::start().await;
    stack.queue.push(ResponseTemplate::new(401));
    let result = stack.call("list_calendars", json!({})).await;
    let text = text(&result);
    assert!(text.contains("AUTH_ERROR"), "{text}");
    assert!(!text.contains("test-password-1234"));
}

#[tokio::test]
async fn fetch_etags_sends_report_without_calendar_data() {
    let stack = Stack::start().await;
    discovery(&stack.queue);
    stack.queue.push(multistatus(
        r#"<D:response><D:href>/123/calendars/home/event1.ics</D:href><D:propstat><D:prop><D:getetag>"etag-abc"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/123/calendars/home/event2.ics</D:href><D:propstat><D:prop><D:getetag>"etag-def"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#,
    ));
    let etags = stack.client.fetch_etags("home", "2025-01-01", "2025-01-31").await.unwrap();
    assert_eq!(etags.len(), 2);
    assert_eq!(etags["/123/calendars/home/event1.ics"].as_deref(), Some("etag-abc"));
    assert_eq!(etags["/123/calendars/home/event2.ics"].as_deref(), Some("etag-def"));
    let requests = stack.requests().await;
    let report = String::from_utf8(requests[3].body.clone()).unwrap();
    assert!(!report.contains("calendar-data"), "ETag-only REPORT must not request calendar-data");
}

#[tokio::test]
async fn stale_handle_conflict_is_retried_once_then_reported() {
    let stack = Stack::start().await;
    let href = "/123/calendars/home/conflict.ics";
    let handle = icloud_calendar_mcp::service::handle::encode(href, Some("stale-etag"), None);
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:conflict@test\r\nDTSTART:20250115T100000Z\r\nSUMMARY:Conflict\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let q = &stack.queue;
    q.push(ResponseTemplate::new(200).insert_header("ETag", "\"fresh-1\"").set_body_string(ics)); // resolve
    q.push(ResponseTemplate::new(412)); // PUT with the handle's etag
    q.push(ResponseTemplate::new(200).insert_header("ETag", "\"fresh-2\"").set_body_string(ics)); // refetch
    q.push(ResponseTemplate::new(412)); // retried PUT
    let result = stack.call("update_event", json!({"event_id": handle, "title": "Mine"})).await;
    assert_eq!(result.is_error, Some(true));
    let text = text(&result);
    let error: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["error"]["code"], "CONFLICT", "{text}");
    assert_eq!(error["error"]["message"], "Precondition failed (resource modified)");
    let requests = stack.requests().await;
    let if_matches: Vec<String> = requests
        .iter()
        .filter(|r| r.method.as_str() == "PUT")
        .map(|r| r.headers.get("If-Match").unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(if_matches, ["stale-etag", "fresh-2"], "first PUT uses the handle's etag, the retry the fresh one");
    assert_eq!(requests.len(), 4, "exactly one retry");
}

#[tokio::test]
async fn create_without_etag_header_falls_back_to_propfind() {
    let stack = Stack::start().await;
    discovery(&stack.queue);
    stack.queue.push(ResponseTemplate::new(201));
    stack.queue.push(multistatus(
        r#"<D:response><D:href>/123/calendars/home/fallback.ics</D:href><D:propstat><D:prop><D:getetag>"fallback-etag"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#,
    ));
    let created = body(
        &stack
            .call("create_event", json!({"calendar_id": "home", "title": "Fallback", "start_time": "2025-01-15T10:00:00Z", "end_time": "2025-01-15T11:00:00Z"}))
            .await,
    );
    let handle = icloud_calendar_mcp::service::EventHandle::decode(created["handle"].as_str().unwrap()).unwrap();
    assert_eq!(handle.etag.as_deref(), Some("fallback-etag"));
    let requests = stack.requests().await;
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[4].method.as_str(), "PROPFIND");
    assert!(String::from_utf8(requests[4].body.clone()).unwrap().contains("getetag"));
}
