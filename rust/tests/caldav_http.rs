//! HTTP-level tests for `HttpCalDavClient` against a local mock server.
//!
//! The mock replays a queue of canned responses in order (like OkHttp's
//! MockWebServer), so each test scripts the exact exchange it expects. An
//! exhausted queue answers 404, which is non-retryable and fails fast.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use icloud_calendar_mcp::caldav::{CalDavClient, CalDavCredentials, HttpCalDavClient};
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

struct Harness {
    server: MockServer,
    queue: Queue,
}

impl Harness {
    async fn start() -> Self {
        let server = MockServer::start().await;
        let queue = Queue::default();
        Mock::given(any()).respond_with(queue.clone()).mount(&server).await;
        Self { server, queue }
    }

    fn base_url(&self) -> String {
        self.server.uri()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.server.uri())
    }

    /// A client that skips well-known discovery, like most Kotlin tests.
    fn client(&self) -> HttpCalDavClient {
        let client = self.fresh_client();
        client.skip_well_known_discovery();
        client
    }

    /// A client that still performs well-known discovery.
    fn fresh_client(&self) -> HttpCalDavClient {
        let builder = || {
            reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_millis(500))
                .read_timeout(Duration::from_secs(2))
        };
        HttpCalDavClient::with_clients(
            self.base_url(),
            CalDavCredentials::new("test@icloud.com", "test-password"),
            builder().build().unwrap(),
            builder().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
        )
    }

    fn enqueue(&self, response: ResponseTemplate) {
        self.queue.push(response);
    }

    fn enqueue_status(&self, status: u16) {
        self.enqueue(ResponseTemplate::new(status));
    }

    fn enqueue_xml(&self, status: u16, body: &str) {
        self.enqueue(ResponseTemplate::new(status).set_body_string(body));
    }

    fn enqueue_principal(&self) {
        self.enqueue_xml(207, PRINCIPAL);
    }

    fn enqueue_home_set(&self) {
        self.enqueue_xml(207, HOME_SET);
    }

    fn enqueue_calendar_list(&self) {
        self.enqueue_xml(207, CALENDAR_LIST);
    }

    fn enqueue_discovery(&self) {
        self.enqueue_principal();
        self.enqueue_home_set();
        self.enqueue_calendar_list();
    }

    async fn requests(&self) -> Vec<Request> {
        self.server.received_requests().await.unwrap_or_default()
    }

    async fn request_count(&self) -> usize {
        self.requests().await.len()
    }
}

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers.get(name).and_then(|v| v.to_str().ok())
}

fn body(request: &Request) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

const PRINCIPAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
<D:response>
<D:href>/</D:href>
<D:propstat>
<D:prop><D:current-user-principal><D:href>/123/principal/</D:href></D:current-user-principal></D:prop>
<D:status>HTTP/1.1 200 OK</D:status>
</D:propstat>
</D:response>
</D:multistatus>"#;

const HOME_SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response>
<D:href>/123/principal/</D:href>
<D:propstat>
<D:prop><C:calendar-home-set><D:href>/123/calendars/</D:href></C:calendar-home-set></D:prop>
<D:status>HTTP/1.1 200 OK</D:status>
</D:propstat>
</D:response>
</D:multistatus>"#;

const CALENDAR_LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response>
<D:href>/123/calendars/home/</D:href>
<D:propstat>
<D:prop><D:displayname>Home</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop>
<D:status>HTTP/1.1 200 OK</D:status>
</D:propstat>
</D:response>
</D:multistatus>"#;

const EMPTY_MULTISTATUS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"></D:multistatus>"#;

const GETETAG_PROPFIND: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
    <D:response>
        <D:href>/cal/event.ics</D:href>
        <D:propstat>
            <D:prop><D:getetag>"recovered-etag"</D:getetag></D:prop>
            <D:status>HTTP/1.1 200 OK</D:status>
        </D:propstat>
    </D:response>
</D:multistatus>"#;

fn ics(uid: &str) -> String {
    format!("BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:{uid}\nSUMMARY:Test\nEND:VEVENT\nEND:VCALENDAR")
}

fn etag_multistatus(href: &str, etag: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>{href}</D:href>
<D:propstat><D:prop><D:getetag>{etag}</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
</D:response></D:multistatus>"#
    )
}

// ── calendar discovery ──

#[tokio::test]
async fn list_calendars_performs_three_step_discovery() {
    let h = Harness::start().await;
    h.enqueue_principal();
    h.enqueue_home_set();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Home</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
<CS:getctag>ctag-abc</CS:getctag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
</D:response></D:multistatus>"#,
    );

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0].display_name, "Home");
    assert_eq!(calendars[0].id, "home");
    assert_eq!(calendars[0].ctag.as_deref(), Some("ctag-abc"));
    assert_eq!(calendars[0].url, h.url("/123/calendars/home/"));
    assert_eq!(h.request_count().await, 3);
}

#[tokio::test]
async fn list_calendars_returns_multiple_calendars() {
    let h = Harness::start().await;
    h.enqueue_principal();
    h.enqueue_home_set();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Home</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/123/calendars/work/</D:href>
<D:propstat><D:prop><D:displayname>Work</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#,
    );

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 2);
    assert_eq!(calendars[0].display_name, "Home");
    assert_eq!(calendars[1].display_name, "Work");
}

#[tokio::test]
async fn list_calendars_handles_auth_failure() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(401).set_body_string("Unauthorized"));

    let error = h.client().list_calendars().await.unwrap_err();

    assert_eq!(error.code, 401);
    assert!(error.is_auth_error());
}

#[tokio::test]
async fn list_calendars_handles_missing_principal() {
    let h = Harness::start().await;
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/</D:href>
<D:propstat><D:prop></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );

    let error = h.client().list_calendars().await.unwrap_err();

    assert!(error.message.to_lowercase().contains("principal"));
}

#[tokio::test]
async fn list_calendars_handles_missing_home_set() {
    let h = Harness::start().await;
    h.enqueue_principal();
    h.enqueue_xml(207, EMPTY_MULTISTATUS);

    let error = h.client().list_calendars().await.unwrap_err();

    assert!(error.message.contains("calendar-home-set"));
}

#[tokio::test]
async fn list_calendars_returns_cached_result_on_second_call() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    let client = h.client();

    client.list_calendars().await.unwrap();
    assert_eq!(h.request_count().await, 3);

    client.list_calendars().await.unwrap();
    assert_eq!(h.request_count().await, 3);
    assert_eq!(client.calendar_home_set().as_deref(), Some("/123/calendars/"));
}

#[tokio::test]
async fn list_calendars_aggregates_from_multiple_home_sets() {
    let h = Harness::start().await;
    h.enqueue_principal();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/principal/</D:href>
<D:propstat><D:prop><C:calendar-home-set><D:href>/123/calendars/</D:href><D:href>/123/shared/</D:href></C:calendar-home-set></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );
    h.enqueue_calendar_list();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/shared/team/</D:href>
<D:propstat><D:prop><D:displayname>Team</D:displayname><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.iter().map(|c| c.display_name.as_str()).collect::<Vec<_>>(), ["Home", "Team"]);
}

#[tokio::test]
async fn list_calendars_deduplicates_across_home_sets() {
    let h = Harness::start().await;
    h.enqueue_principal();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/principal/</D:href>
<D:propstat><D:prop><C:calendar-home-set><D:href>/123/calendars/</D:href><D:href>/123/calendars/</D:href></C:calendar-home-set></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );
    h.enqueue_calendar_list();
    h.enqueue_calendar_list();

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
}

#[tokio::test]
async fn failing_home_set_is_skipped() {
    let h = Harness::start().await;
    h.enqueue_principal();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/principal/</D:href>
<D:propstat><D:prop><C:calendar-home-set><D:href>/123/broken/</D:href><D:href>/123/calendars/</D:href></C:calendar-home-set></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );
    h.enqueue_status(403);
    h.enqueue_calendar_list();

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
}

#[tokio::test]
async fn list_calendars_sends_correct_propfind_requests() {
    let h = Harness::start().await;
    h.enqueue_discovery();

    h.client().list_calendars().await.unwrap();

    let requests = h.requests().await;
    assert_eq!(requests[0].method.as_str(), "PROPFIND");
    assert_eq!(requests[0].url.path(), "/");
    assert_eq!(header(&requests[0], "Depth"), Some("0"));
    assert!(body(&requests[0]).contains("current-user-principal"));
    assert_eq!(requests[1].method.as_str(), "PROPFIND");
    assert_eq!(requests[1].url.path(), "/123/principal/");
    assert!(body(&requests[1]).contains("calendar-home-set"));
    assert_eq!(requests[2].method.as_str(), "PROPFIND");
    assert_eq!(requests[2].url.path(), "/123/calendars/");
    assert_eq!(header(&requests[2], "Depth"), Some("1"));
    assert!(body(&requests[2]).contains("supported-calendar-component-set"));
}

#[tokio::test]
async fn requests_include_basic_authorization() {
    let h = Harness::start().await;
    h.enqueue_principal();

    let _ = h.client().list_calendars().await;

    let requests = h.requests().await;
    let auth = header(&requests[0], "Authorization").unwrap();
    assert!(auth.starts_with("Basic "));
}

#[tokio::test]
async fn propfind_redirect_is_followed() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(307).insert_header("Location", h.url("/redirected/").as_str()));
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/redirected/</D:href>
<D:propstat><D:prop><D:current-user-principal><D:href>/123/principal/</D:href></D:current-user-principal></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );
    h.enqueue_home_set();
    h.enqueue_calendar_list();

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    let requests = h.requests().await;
    assert_eq!(requests[1].method.as_str(), "PROPFIND");
    assert_eq!(requests[1].url.path(), "/redirected/");
}

// ── well-known discovery ──

#[tokio::test]
async fn well_known_discovery_follows_redirect_to_caldav_endpoint() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(301).insert_header("Location", h.url("/caldav/").as_str()));
    h.enqueue(ResponseTemplate::new(200).set_body_string("CalDAV endpoint"));
    h.enqueue_principal();
    h.enqueue_home_set();
    h.enqueue_calendar_list();

    let calendars = h.fresh_client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    let requests = h.requests().await;
    assert_eq!(requests[0].method.as_str(), "GET");
    assert!(requests[0].url.path().contains(".well-known/caldav"));
    assert!(header(&requests[0], "Authorization").is_some());
    // Principal discovery goes to the endpoint the redirect named.
    assert_eq!(requests[2].method.as_str(), "PROPFIND");
    assert_eq!(requests[2].url.path(), "/caldav/");
}

#[tokio::test]
async fn well_known_relative_location_is_resolved() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(302).insert_header("Location", "/dav/"));
    h.enqueue_status(207);
    h.enqueue_discovery();

    h.fresh_client().list_calendars().await.unwrap();

    assert_eq!(h.requests().await[2].url.path(), "/dav/");
}

#[tokio::test]
async fn well_known_404_falls_back_to_base_url() {
    let h = Harness::start().await;
    h.enqueue_status(404);
    h.enqueue_discovery();

    let calendars = h.fresh_client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    assert_eq!(h.request_count().await, 4);
    assert_eq!(h.requests().await[1].url.path(), "/");
}

#[tokio::test]
async fn well_known_detects_redirect_loop_and_gives_up() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(301).insert_header("Location", h.url("/a/").as_str()));
    h.enqueue(ResponseTemplate::new(301).insert_header("Location", h.url("/b/").as_str()));
    h.enqueue(ResponseTemplate::new(301).insert_header("Location", h.url("/a/").as_str()));
    h.enqueue_discovery();

    let calendars = h.fresh_client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    // well-known + /a/ + /b/, then discovery from the base URL.
    assert_eq!(h.requests().await[3].url.path(), "/");
}

#[tokio::test]
async fn well_known_stops_after_max_hops() {
    let h = Harness::start().await;
    for hop in 0..5 {
        h.enqueue(ResponseTemplate::new(302).insert_header("Location", h.url(&format!("/hop{hop}/")).as_str()));
    }
    h.enqueue_discovery();

    let calendars = h.fresh_client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    assert_eq!(h.requests().await[5].url.path(), "/");
}

#[tokio::test]
async fn well_known_result_is_cached_across_calls() {
    let h = Harness::start().await;
    h.enqueue_status(404);
    h.enqueue_discovery();
    let client = h.fresh_client();

    client.list_calendars().await.unwrap();
    let after_first = h.request_count().await;
    client.list_calendars().await.unwrap();

    assert_eq!(h.request_count().await, after_first);
}

// ── get events ──

#[tokio::test]
async fn get_events_returns_events_in_date_range() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_xml(
        207,
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/home/event1.ics</D:href>
<D:propstat><D:prop><D:getetag>"etag1"</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
            ics("event1@test")
        ),
    );

    let events = h.client().get_events("home", "2025-01-01", "2025-01-31").await.unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].uid, "event1@test");
    assert_eq!(events[0].etag.as_deref(), Some("etag1"));
    assert_eq!(events[0].url, h.url("/123/calendars/home/event1.ics"));

    let report = &h.requests().await[3];
    assert_eq!(report.method.as_str(), "REPORT");
    assert_eq!(report.url.path(), "/123/calendars/home/");
    assert_eq!(header(report, "Depth"), Some("1"));
    assert_eq!(header(report, "Content-Type"), Some("application/xml; charset=utf-8"));
    let report_body = body(report);
    assert!(report_body.contains(r#"start="20250101T000000Z" end="20250131T235959Z""#));
    assert!(report_body.contains("calendar-data"));
}

#[tokio::test]
async fn get_events_returns_empty_list_for_no_events() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_xml(207, EMPTY_MULTISTATUS);

    assert!(h.client().get_events("home", "2025-01-01", "2025-01-31").await.unwrap().is_empty());
}

#[tokio::test]
async fn get_events_returns_not_found_for_unknown_calendar() {
    let h = Harness::start().await;
    h.enqueue_discovery();

    let error = h.client().get_events("nonexistent", "2025-01-01", "2025-01-31").await.unwrap_err();

    assert!(error.is_not_found());
}

#[tokio::test]
async fn get_events_rejects_invalid_dates_before_any_request() {
    let h = Harness::start().await;
    let client = h.client();

    let error = client.get_events("home", "not-a-date", "2025-01-31").await.unwrap_err();
    assert_eq!(error.code, 400);
    assert!(error.message.contains("Invalid"));

    let error = client.get_events("home", "2025-01-01", "2025<script>").await.unwrap_err();
    assert_eq!(error.code, 400);

    assert_eq!(h.request_count().await, 0);
}

// ── get event ──

#[tokio::test]
async fn get_event_fetches_uid_and_etag_from_header() {
    let h = Harness::start().await;
    let ics = ics("solo@test");
    h.enqueue(ResponseTemplate::new(200).insert_header("ETag", "\"solo-etag\"").set_body_string(ics.as_str()));

    let event = h.client().get_event("/123/calendars/home/solo.ics").await.unwrap();

    assert_eq!(event.uid, "solo@test");
    assert_eq!(event.etag.as_deref(), Some("solo-etag"));
    assert_eq!(event.ical_data, ics);
    assert_eq!(event.href, "/123/calendars/home/solo.ics");
    let requests = h.requests().await;
    assert_eq!(requests[0].method.as_str(), "GET");
}

#[tokio::test]
async fn get_event_falls_back_to_propfind_without_etag_header() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(200).set_body_string(ics("noetag@test").as_str()));
    h.enqueue_xml(207, &etag_multistatus("/123/calendars/home/noetag.ics", "\"probed-etag\""));

    let event = h.client().get_event("/123/calendars/home/noetag.ics").await.unwrap();

    assert_eq!(event.uid, "noetag@test");
    assert_eq!(event.etag.as_deref(), Some("probed-etag"));
}

#[tokio::test]
async fn get_event_returns_not_found_when_gone() {
    let h = Harness::start().await;
    h.enqueue_status(404);

    let error = h.client().get_event("/123/calendars/home/missing.ics").await.unwrap_err();

    assert!(error.is_not_found());
}

#[tokio::test]
async fn get_event_without_uid_is_not_found() {
    let h = Harness::start().await;
    h.enqueue(
        ResponseTemplate::new(200).insert_header("ETag", "\"e\"").set_body_string("BEGIN:VCALENDAR\nEND:VCALENDAR"),
    );

    let error = h.client().get_event("/cal/x.ics").await.unwrap_err();

    assert!(error.is_not_found());
    assert!(error.message.contains("missing UID"));
}

#[tokio::test]
async fn get_event_normalizes_regional_href() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(200).insert_header("ETag", "\"e\"").set_body_string(ics("p@test").as_str()));

    let event = h.client().get_event("/123/calendars/home/p.ics").await.unwrap();

    assert_eq!(event.url, h.url("/123/calendars/home/p.ics"));
}

// ── create event ──

#[tokio::test]
async fn create_event_returns_created_event_with_etag() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue(ResponseTemplate::new(201).insert_header("ETag", "\"new-etag-123\""));

    let event = h.client().create_event("home", &ics("new-event@test")).await.unwrap();

    assert_eq!(event.uid, "new-event@test");
    assert_eq!(event.etag.as_deref(), Some("new-etag-123"));
    assert!(event.href.starts_with("/123/calendars/home/") && event.href.ends_with(".ics"));
    assert_eq!(event.url, h.url(&event.href));

    let put = &h.requests().await[3];
    assert_eq!(put.method.as_str(), "PUT");
    assert_eq!(header(put, "Content-Type"), Some("text/calendar; charset=utf-8"));
    assert_eq!(header(put, "If-None-Match"), Some("*"));
    assert_eq!(put.url.path(), event.href);
}

#[tokio::test]
async fn create_event_handles_conflict() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_status(412);

    let error = h.client().create_event("home", &ics("existing@test")).await.unwrap_err();

    assert_eq!(error.code, 412);
}

#[tokio::test]
async fn create_event_rejects_ics_without_uid() {
    let h = Harness::start().await;
    h.enqueue_discovery();

    let error = h.client().create_event("home", "BEGIN:VCALENDAR\nEND:VCALENDAR").await.unwrap_err();

    assert_eq!(error.code, 400);
    assert_eq!(h.request_count().await, 3);
}

#[tokio::test]
async fn create_event_in_unknown_calendar_is_not_found() {
    let h = Harness::start().await;
    h.enqueue_discovery();

    let error = h.client().create_event("missing", &ics("x@test")).await.unwrap_err();

    assert!(error.is_not_found());
}

#[tokio::test]
async fn create_event_falls_back_to_propfind_without_etag() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_status(201);
    h.enqueue_xml(207, &etag_multistatus("/123/calendars/home/event.ics", "\"fallback-etag-123\""));

    let event = h.client().create_event("home", &ics("new@test")).await.unwrap();

    assert_eq!(event.etag.as_deref(), Some("fallback-etag-123"));
    let probe = &h.requests().await[4];
    assert_eq!(probe.method.as_str(), "PROPFIND");
    assert_eq!(header(probe, "Depth"), Some("0"));
}

#[tokio::test]
async fn create_event_has_no_etag_when_put_and_propfind_both_miss() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_status(201);
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/event.ics</D:href>
<D:propstat><D:prop></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
    );

    let event = h.client().create_event("home", &ics("no-etag@test")).await.unwrap();

    assert_eq!(event.etag, None);
}

#[tokio::test]
async fn create_event_has_no_etag_when_propfind_fails() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_status(201);
    h.enqueue_status(500);

    let event = h.client().create_event("home", &ics("fail@test")).await.unwrap();

    assert_eq!(event.etag, None);
}

// ── update event ──

#[tokio::test]
async fn update_event_returns_new_etag_and_sends_if_match() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(204).insert_header("ETag", "\"updated-etag\""));

    let event = h
        .client()
        .update_event("/123/calendars/home/event1.ics", &ics("event1@test"), Some("\"old-etag\""))
        .await
        .unwrap();

    assert_eq!(event.etag.as_deref(), Some("updated-etag"));
    assert_eq!(event.href, "/123/calendars/home/event1.ics");
    let requests = h.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method.as_str(), "PUT");
    assert_eq!(header(&requests[0], "If-Match"), Some("\"old-etag\""));
}

#[tokio::test]
async fn update_event_handles_stale_etag() {
    let h = Harness::start().await;
    h.enqueue_status(412);

    let error = h.client().update_event("/cal/event.ics", &ics("test@test"), Some("\"stale\"")).await.unwrap_err();

    assert_eq!(error.code, 412);
    assert!(error.is_conflict());
}

#[tokio::test]
async fn update_event_handles_404() {
    let h = Harness::start().await;
    h.enqueue_status(404); // ETag probe
    h.enqueue_status(404); // PUT

    let error = h.client().update_event("/cal/missing.ics", &ics("test@test"), None).await.unwrap_err();

    assert!(error.is_not_found());
}

#[tokio::test]
async fn update_event_rejects_ics_without_uid_before_any_request() {
    let h = Harness::start().await;

    let error = h.client().update_event("/cal/x.ics", "BEGIN:VCALENDAR\nEND:VCALENDAR", None).await.unwrap_err();

    assert_eq!(error.code, 400);
    assert_eq!(h.request_count().await, 0);
}

#[tokio::test]
async fn update_event_with_null_etag_recovers_via_propfind() {
    let h = Harness::start().await;
    h.enqueue_xml(207, GETETAG_PROPFIND);
    h.enqueue(ResponseTemplate::new(204).insert_header("ETag", "\"new-etag\""));

    let event = h.client().update_event("/cal/event.ics", &ics("e@t"), None).await.unwrap();

    assert_eq!(event.etag.as_deref(), Some("new-etag"));
    let requests = h.requests().await;
    assert_eq!(requests[0].method.as_str(), "PROPFIND");
    assert_eq!(header(&requests[0], "Depth"), Some("0"));
    assert_eq!(requests[1].method.as_str(), "PUT");
    assert_eq!(header(&requests[1], "If-Match"), Some("recovered-etag"));
}

#[tokio::test]
async fn update_event_with_etag_skips_propfind_probe() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(204).insert_header("ETag", "\"new-etag\""));

    h.client().update_event("/cal/event.ics", &ics("e@t"), Some("\"caller-etag\"")).await.unwrap();

    assert_eq!(h.request_count().await, 1);
}

#[tokio::test]
async fn update_event_without_recoverable_etag_puts_unconditionally() {
    let h = Harness::start().await;
    h.enqueue_status(500); // probe fails
    h.enqueue(ResponseTemplate::new(204).insert_header("ETag", "\"new-etag\""));

    h.client().update_event("/cal/event.ics", &ics("e@t"), None).await.unwrap();

    let requests = h.requests().await;
    assert_eq!(requests[1].method.as_str(), "PUT");
    assert_eq!(header(&requests[1], "If-Match"), None);
}

#[tokio::test]
async fn update_event_falls_back_to_propfind_when_put_has_no_etag() {
    let h = Harness::start().await;
    h.enqueue_status(204);
    h.enqueue_xml(207, &etag_multistatus("/cal/event.ics", "\"update-fallback\""));

    let event = h.client().update_event("/cal/event.ics", &ics("upd@test"), Some("\"old\"")).await.unwrap();

    assert_eq!(event.etag.as_deref(), Some("update-fallback"));
}

#[tokio::test]
async fn update_event_accepts_absolute_url() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(204).insert_header("ETag", "\"n\""));
    let url = h.url("/abs/event.ics");

    let event = h.client().update_event(&url, &ics("a@t"), Some("e")).await.unwrap();

    assert_eq!(event.url, url);
    assert_eq!(h.requests().await[0].url.path(), "/abs/event.ics");
}

// ── delete event ──

#[tokio::test]
async fn delete_event_sends_if_match() {
    let h = Harness::start().await;
    h.enqueue_status(204);

    h.client().delete_event("/cal/event.ics", Some("\"etag\"")).await.unwrap();

    let requests = h.requests().await;
    assert_eq!(requests[0].method.as_str(), "DELETE");
    assert_eq!(header(&requests[0], "If-Match"), Some("\"etag\""));
}

#[tokio::test]
async fn delete_event_without_etag_still_works() {
    let h = Harness::start().await;
    h.enqueue_status(404); // probe finds nothing
    h.enqueue_status(204);

    h.client().delete_event("/cal/event.ics", None).await.unwrap();

    let requests = h.requests().await;
    assert_eq!(requests[1].method.as_str(), "DELETE");
    assert_eq!(header(&requests[1], "If-Match"), None);
}

#[tokio::test]
async fn delete_event_treats_404_as_success() {
    let h = Harness::start().await;
    h.enqueue_status(404);
    h.enqueue_status(404);

    assert!(h.client().delete_event("/cal/missing.ics", None).await.is_ok());
}

#[tokio::test]
async fn delete_event_handles_conflict() {
    let h = Harness::start().await;
    h.enqueue_status(412);

    let error = h.client().delete_event("/cal/event.ics", Some("\"wrong\"")).await.unwrap_err();

    assert_eq!(error.code, 412);
}

#[tokio::test]
async fn delete_event_with_null_etag_recovers_via_propfind() {
    let h = Harness::start().await;
    h.enqueue_xml(207, GETETAG_PROPFIND);
    h.enqueue_status(204);

    h.client().delete_event("/cal/event.ics", None).await.unwrap();

    let requests = h.requests().await;
    assert_eq!(requests[0].method.as_str(), "PROPFIND");
    assert_eq!(requests[1].method.as_str(), "DELETE");
    assert_eq!(header(&requests[1], "If-Match"), Some("recovered-etag"));
}

// ── errors, retries, circuit breaker ──

#[tokio::test]
async fn server_error_is_retried_once_then_returned() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(503).set_body_string("Service Unavailable"));
    h.enqueue(ResponseTemplate::new(503).set_body_string("Service Unavailable"));

    let error = h.client().list_calendars().await.unwrap_err();

    assert!(error.is_server_error());
    assert!(error.retryable);
    assert_eq!(h.request_count().await, 2);
}

#[tokio::test]
async fn forbidden_is_not_retried() {
    let h = Harness::start().await;
    h.enqueue_status(403);

    let error = h.client().list_calendars().await.unwrap_err();

    assert!(error.is_auth_error());
    assert!(!error.retryable);
    assert_eq!(h.request_count().await, 1);
}

#[tokio::test]
async fn auth_error_is_not_retried() {
    let h = Harness::start().await;
    h.enqueue_status(401);

    let error = h.client().list_calendars().await.unwrap_err();

    assert_eq!(error.code, 401);
    assert_eq!(h.request_count().await, 1);
}

#[tokio::test]
async fn retries_past_503_and_succeeds() {
    let h = Harness::start().await;
    h.enqueue_status(503);
    h.enqueue_discovery();

    let calendars = h.client().list_calendars().await.unwrap();

    assert_eq!(calendars.len(), 1);
    assert_eq!(h.request_count().await, 4);
}

#[tokio::test]
async fn retries_429_honoring_retry_after() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(429).insert_header("Retry-After", "1"));
    h.enqueue_principal();

    let start = Instant::now();
    let _ = h.client().list_calendars().await;
    let elapsed = start.elapsed();

    assert!(elapsed >= Duration::from_millis(900), "expected >= 900ms, got {elapsed:?}");
    assert!(h.request_count().await >= 2);
}

#[tokio::test]
async fn unexpected_status_is_not_retried() {
    let h = Harness::start().await;
    h.enqueue_status(418);

    let error = h.client().list_calendars().await.unwrap_err();

    assert_eq!(error.code, 418);
    assert_eq!(error.message, "Unexpected response: 418");
    assert_eq!(h.request_count().await, 1);
}

#[tokio::test]
async fn circuit_breaker_opens_after_consecutive_failures_and_resets_on_success() {
    let h = Harness::start().await;
    let client = h.client();
    for _ in 0..5 {
        h.enqueue_status(403);
        assert_eq!(client.get_event("/cal/x.ics").await.unwrap_err().code, 403);
    }

    let error = client.get_event("/cal/x.ics").await.unwrap_err();
    assert_eq!(error.code, 503);
    assert!(error.message.contains("Circuit breaker"));
    assert!(!error.retryable);
    assert_eq!(h.request_count().await, 5);
}

#[tokio::test]
async fn success_resets_failure_count() {
    let h = Harness::start().await;
    let client = h.client();
    for _ in 0..4 {
        h.enqueue_status(403);
        let _ = client.get_event("/cal/x.ics").await;
    }
    h.enqueue(ResponseTemplate::new(200).insert_header("ETag", "\"e\"").set_body_string(ics("ok@t").as_str()));
    client.get_event("/cal/x.ics").await.unwrap();
    for _ in 0..4 {
        h.enqueue_status(403);
        assert_eq!(client.get_event("/cal/x.ics").await.unwrap_err().code, 403);
    }
}

#[tokio::test]
async fn network_error_is_classified_and_retryable() {
    let client = HttpCalDavClient::with_clients(
        "http://127.0.0.1:1",
        CalDavCredentials::new("u@icloud.com", "p"),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    );
    client.skip_well_known_discovery();

    let error = client.get_event("/cal/x.ics").await.unwrap_err();

    assert_eq!(error.code, 0);
    assert!(error.retryable);
    assert!(error.message.starts_with("Network error"));
}

#[tokio::test]
async fn read_timeout_is_classified_as_timeout() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)));
    h.enqueue(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)));
    let client = HttpCalDavClient::with_clients(
        h.base_url(),
        CalDavCredentials::new("u@icloud.com", "p"),
        reqwest::Client::builder().no_proxy().timeout(Duration::from_millis(200)).build().unwrap(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    );
    client.skip_well_known_discovery();

    let error = client.get_event("/cal/x.ics").await.unwrap_err();

    assert_eq!(error.code, 408);
    assert!(error.retryable);
}

// ── body size limit ──

#[tokio::test]
async fn response_under_limit_is_read_normally() {
    let h = Harness::start().await;
    let padding = "x".repeat(1_000_000);
    h.enqueue_xml(207, &format!("{PRINCIPAL}<!-- {padding} -->"));

    let result = h.client().list_calendars().await;

    if let Err(error) = result {
        assert!(!error.message.contains("too large"), "1MB body tripped the limit: {}", error.message);
    }
}

#[tokio::test]
async fn response_over_limit_is_rejected() {
    let h = Harness::start().await;
    h.enqueue_xml(207, &"x".repeat(3_000_000));

    let error = h.client().list_calendars().await.unwrap_err();

    assert_eq!(error.code, 413);
    assert!(error.message.contains("too large"));
}

#[tokio::test]
async fn oversized_write_response_is_rejected() {
    let h = Harness::start().await;
    h.enqueue_xml(201, &"x".repeat(3_000_001));

    let error = h.client().update_event("/cal/event.ics", &ics("test@test"), Some("\"etag\"")).await.unwrap_err();

    assert_eq!(error.code, 413);
    assert!(error.message.contains("too large"));
}

// ── connection check ──

#[tokio::test]
async fn check_connection_succeeds_with_calendar_access() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(200).insert_header("DAV", "1, calendar-access"));

    assert!(h.client().check_connection().await.unwrap());
    assert_eq!(h.requests().await[0].method.as_str(), "OPTIONS");
}

#[tokio::test]
async fn check_connection_fails_without_calendar_access() {
    let h = Harness::start().await;
    h.enqueue(ResponseTemplate::new(200).insert_header("DAV", "1"));

    let error = h.client().check_connection().await.unwrap_err();

    assert_eq!(error.code, 400);
    assert!(error.message.contains("calendar-access"));
}

#[tokio::test]
async fn check_connection_fails_on_auth_error() {
    let h = Harness::start().await;
    h.enqueue_status(401);

    assert_eq!(h.client().check_connection().await.unwrap_err().code, 401);
}

// ── fetch etags ──

#[tokio::test]
async fn fetch_etags_returns_href_to_etag_map() {
    let h = Harness::start().await;
    h.enqueue_discovery();
    h.enqueue_xml(
        207,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
<D:response><D:href>/123/calendars/home/event1.ics</D:href><D:propstat><D:prop><D:getetag>"etag-1"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/123/calendars/home/event2.ics</D:href><D:propstat><D:prop><D:getetag>"etag-2"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#,
    );

    let etags = h.client().fetch_etags("home", "2025-01-01", "2025-01-31").await.unwrap();

    assert_eq!(etags.len(), 2);
    assert_eq!(etags["/123/calendars/home/event1.ics"].as_deref(), Some("etag-1"));
    assert_eq!(etags["/123/calendars/home/event2.ics"].as_deref(), Some("etag-2"));
    let report = h.requests().await.pop().unwrap();
    assert_eq!(report.method.as_str(), "REPORT");
    assert!(!body(&report).contains("calendar-data"));
}

#[tokio::test]
async fn fetch_etags_validates_date_format() {
    let h = Harness::start().await;

    let error = h.client().fetch_etags("home", "bad-date", "2025-01-31").await.unwrap_err();

    assert_eq!(error.code, 400);
    assert_eq!(h.request_count().await, 0);
}
