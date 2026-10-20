//! CalDAV client over HTTP, tuned for iCloud.
//!
//! - RFC 6764 well-known discovery, then 3-step discovery (principal ->
//!   calendar-home-set -> calendars), cached for five minutes.
//! - `calendar-query` REPORTs for reads; PUT/DELETE with ETag preconditions for
//!   writes, probing the ETag with a PROPFIND when the caller has none.
//! - Every response body is capped at 2 MB. Retryable failures get one quick
//!   retry (MCP tools must answer fast), and a circuit breaker fails fast after
//!   repeated consecutive failures.
//!
//! Safe to share across tasks: discovery caches sit behind a mutex, and a
//! double fetch under contention is harmless because discovery is idempotent.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode, redirect};

use super::etag::normalize_etag;
use super::url_normalizer::normalize_icloud_url;
use super::xml::ICloudXmlParser;
use super::{CalDavCalendar, CalDavClient, CalDavCredentials, CalDavError, CalDavEvent, CalDavResult};

/// Largest response body read into memory.
pub const MAX_BODY_SIZE: usize = 2_000_000;
/// Retries after the first attempt for a retryable failure.
pub const MAX_RETRIES: u32 = 1;
pub const BASE_DELAY_MS: u64 = 100;
pub const MAX_JITTER_MS: u64 = 50;
/// Upper bound honored for a server's Retry-After, so a tool call stays responsive.
pub const MAX_RETRY_AFTER_MS: u64 = 2_000;
/// Consecutive failures after which requests fail fast.
pub const CIRCUIT_BREAKER_THRESHOLD: u32 = 5;
/// Redirect hops followed during well-known discovery.
pub const MAX_WELL_KNOWN_HOPS: usize = 5;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_LOG_CHARS: usize = 1000;

const XML_CONTENT_TYPE: &str = "application/xml; charset=utf-8";
const ICS_CONTENT_TYPE: &str = "text/calendar; charset=utf-8";

// ASCII digits only: these values are interpolated into REPORT XML.
static DATE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$").expect("valid regex"));

const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:propfind xmlns:D="DAV:">
    <D:prop>
        <D:current-user-principal/>
    </D:prop>
</D:propfind>"#;

const PROPFIND_HOME_SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
    <D:prop>
        <C:calendar-home-set/>
    </D:prop>
</D:propfind>"#;

const PROPFIND_CALENDARS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/" xmlns:ICAL="http://apple.com/ns/ical/">
    <D:prop>
        <D:displayname/>
        <D:resourcetype/>
        <CS:getctag/>
        <ICAL:calendar-color/>
        <D:current-user-privilege-set/>
        <C:supported-calendar-component-set/>
    </D:prop>
</D:propfind>"#;

const PROPFIND_ETAG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:propfind xmlns:D="DAV:">
    <D:prop>
        <D:getetag/>
    </D:prop>
</D:propfind>"#;

/// A `calendar-query` REPORT over whole UTC days; dates are `YYYYMMDD`.
fn calendar_query_report(start: &str, end: &str, include_data: bool) -> String {
    let data = if include_data { "\n        <C:calendar-data/>" } else { "" };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
    <D:prop>
        <D:getetag/>{data}
    </D:prop>
    <C:filter>
        <C:comp-filter name="VCALENDAR">
            <C:comp-filter name="VEVENT">
                <C:time-range start="{start}T000000Z" end="{end}T235959Z"/>
            </C:comp-filter>
        </C:comp-filter>
    </C:filter>
</C:calendar-query>"#
    )
}

/// A request that can be re-sent on retry.
struct RequestSpec {
    method: Method,
    url: String,
    body: Option<(&'static str, String)>,
    headers: Vec<(&'static str, String)>,
}

impl RequestSpec {
    fn new(method: Method, url: impl Into<String>) -> Self {
        Self { method, url: url.into(), body: None, headers: Vec::new() }
    }

    fn propfind(url: impl Into<String>, depth: &str, body: &str) -> Self {
        Self::new(propfind(), url).xml_body(body).header("Depth", depth)
    }

    fn xml_body(self, body: impl Into<String>) -> Self {
        Self { body: Some((XML_CONTENT_TYPE, body.into())), ..self }
    }

    fn ics_body(self, body: impl Into<String>) -> Self {
        Self { body: Some((ICS_CONTENT_TYPE, body.into())), ..self }
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

fn propfind() -> Method {
    Method::from_bytes(b"PROPFIND").expect("valid method")
}

fn report() -> Method {
    Method::from_bytes(b"REPORT").expect("valid method")
}

/// A response whose body fit under [`MAX_BODY_SIZE`].
struct HttpResponse {
    headers: HeaderMap,
    body: String,
}

impl HttpResponse {
    fn etag_header(&self) -> Option<String> {
        normalize_etag(self.headers.get("ETag").and_then(|v| v.to_str().ok()))
    }
}

/// One attempt's outcome plus the server's Retry-After hint, in milliseconds.
struct Attempt<T> {
    result: CalDavResult<T>,
    retry_after_ms: u64,
}

impl<T> Attempt<T> {
    fn done(result: CalDavResult<T>) -> Self {
        Self { result, retry_after_ms: 0 }
    }
}

#[derive(Default)]
struct DiscoveryState {
    calendar_home_set: Option<String>,
    calendars: Option<(Vec<CalDavCalendar>, Instant)>,
    well_known_attempted: bool,
    discovered_base_url: Option<String>,
}

/// [`CalDavClient`] over HTTP via reqwest.
pub struct HttpCalDavClient {
    base_url: String,
    credentials: CalDavCredentials,
    client: reqwest::Client,
    /// Redirects are followed by hand during well-known discovery.
    well_known_client: reqwest::Client,
    xml: ICloudXmlParser,
    log_http: bool,
    state: Mutex<DiscoveryState>,
    consecutive_failures: AtomicU32,
}

impl HttpCalDavClient {
    /// A client with production timeouts and the system proxy settings.
    pub fn new(base_url: impl Into<String>, credentials: CalDavCredentials) -> CalDavResult<Self> {
        let client = default_builder().build().map_err(client_build_error)?;
        Self::with_client(base_url, credentials, client)
    }

    /// A client using a prebuilt reqwest client for every request except
    /// well-known discovery, which gets a non-redirecting default client.
    pub fn with_client(
        base_url: impl Into<String>,
        credentials: CalDavCredentials,
        client: reqwest::Client,
    ) -> CalDavResult<Self> {
        let well_known_client =
            default_builder().redirect(redirect::Policy::none()).build().map_err(client_build_error)?;
        Ok(Self::with_clients(base_url, credentials, client, well_known_client))
    }

    /// A client with both reqwest clients supplied (tests use this to bypass
    /// proxies and shorten timeouts). `well_known_client` must not follow redirects.
    pub fn with_clients(
        base_url: impl Into<String>,
        credentials: CalDavCredentials,
        client: reqwest::Client,
        well_known_client: reqwest::Client,
    ) -> Self {
        let log_http = std::env::var("CALDAV_LOG_HTTP").is_ok_and(|v| v.eq_ignore_ascii_case("true"));
        Self {
            base_url: base_url.into(),
            credentials,
            client,
            well_known_client,
            xml: ICloudXmlParser::new(),
            log_http,
            state: Mutex::new(DiscoveryState::default()),
            consecutive_failures: AtomicU32::new(0),
        }
    }

    /// Skip RFC 6764 discovery and use the base URL directly (for tests).
    pub fn skip_well_known_discovery(&self) {
        self.state().well_known_attempted = true;
    }

    /// The first calendar-home-set found by the last discovery.
    pub fn calendar_home_set(&self) -> Option<String> {
        self.state().calendar_home_set.clone()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, DiscoveryState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn absolute_url(&self, href: &str) -> String {
        if href.starts_with("http") { href.to_string() } else { format!("{}{href}", self.base_url) }
    }

    async fn find_calendar(&self, calendar_id: &str) -> CalDavResult<CalDavCalendar> {
        self.list_calendars()
            .await?
            .into_iter()
            .find(|calendar| calendar.id == calendar_id)
            .ok_or_else(|| CalDavError::not_found(format!("Calendar not found: {calendar_id}")))
    }

    // ── discovery ──

    async fn discover_principal(&self, discovery_url: &str) -> CalDavResult<Option<String>> {
        let spec = RequestSpec::propfind(discovery_url, "0", PROPFIND_PRINCIPAL);
        self.execute(&spec, false, |resp| async move { Ok(self.xml.parse_current_user_principal(&resp.body)) }).await
    }

    async fn discover_home_sets(&self, principal_href: &str) -> CalDavResult<Vec<String>> {
        let spec = RequestSpec::propfind(self.absolute_url(principal_href), "0", PROPFIND_HOME_SET);
        self.execute(&spec, false, |resp| async move { Ok(self.xml.parse_calendar_home_sets(&resp.body)) }).await
    }

    async fn fetch_calendars(&self, home_set_href: &str) -> CalDavResult<Vec<CalDavCalendar>> {
        let spec = RequestSpec::propfind(self.absolute_url(home_set_href), "1", PROPFIND_CALENDARS);
        self.execute(&spec, false, |resp| async move { Ok(self.xml.parse_calendars(&resp.body, &self.base_url)) }).await
    }

    /// RFC 6764: follow `/.well-known/caldav` redirects by hand to find the
    /// CalDAV endpoint. Best-effort and attempted once per client; `None` means
    /// "use the base URL".
    async fn discover_well_known(&self) -> Option<String> {
        {
            let mut state = self.state();
            if state.well_known_attempted {
                return state.discovered_base_url.clone();
            }
            state.well_known_attempted = true;
        }
        let discovered = self.follow_well_known().await;
        if discovered.is_some() {
            self.state().discovered_base_url = discovered.clone();
        }
        discovered
    }

    async fn follow_well_known(&self) -> Option<String> {
        let mut current = format!("{}/.well-known/caldav", self.base_url.trim_end_matches('/'));
        let mut visited = HashSet::new();
        for _ in 0..MAX_WELL_KNOWN_HOPS {
            if !visited.insert(current.clone()) {
                return None; // redirect loop
            }
            let response = self
                .well_known_client
                .get(&current)
                .basic_auth(&self.credentials.username, Some(&self.credentials.password))
                .send()
                .await
                .ok()?;
            match response.status().as_u16() {
                301 | 302 | 307 | 308 => {
                    let location = response.headers().get("Location")?.to_str().ok()?;
                    let resolved = response.url().join(location).ok()?.to_string();
                    current = self.preserve_https(resolved);
                }
                200..=299 => return Some(current),
                _ => return None,
            }
        }
        None
    }

    /// Never let a redirect downgrade an HTTPS base URL to plain HTTP.
    fn preserve_https(&self, resolved: String) -> String {
        match resolved.strip_prefix("http://") {
            Some(rest) if self.base_url.starts_with("https://") => format!("https://{rest}"),
            _ => resolved,
        }
    }

    /// The ETag of `url` via a Depth:0 PROPFIND (RFC 4791 §5.3.4 fallback when a
    /// response omitted it). Best-effort: any failure yields `None`.
    async fn fetch_etag(&self, url: &str) -> Option<String> {
        let spec = RequestSpec::propfind(url, "0", PROPFIND_ETAG);
        let response = self.send(&self.client, &spec).await.ok()?;
        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return None;
        }
        let body = read_body_with_limit(response).await.ok()??;
        self.xml.parse_etag(&body)
    }

    // ── HTTP plumbing ──

    async fn send(&self, client: &reqwest::Client, spec: &RequestSpec) -> reqwest::Result<reqwest::Response> {
        if self.log_http {
            eprintln!("--> {} {}", spec.method, spec.url);
        }
        let mut request = client
            .request(spec.method.clone(), &spec.url)
            .basic_auth(&self.credentials.username, Some(&self.credentials.password));
        for (name, value) in &spec.headers {
            request = request.header(*name, value);
        }
        if let Some((content_type, body)) = &spec.body {
            request = request.header("Content-Type", *content_type).body(body.clone());
        }
        request.send().await
    }

    fn log_response(&self, status: u16, url: &str, body: &str) {
        if !self.log_http {
            return;
        }
        let shown: String = body.chars().take(MAX_BODY_LOG_CHARS).collect();
        let shown = if body.chars().count() > MAX_BODY_LOG_CHARS { format!("{shown}... [truncated]") } else { shown };
        eprintln!("<-- {status} {url} ({} chars)", shown.chars().count());
        if !shown.trim().is_empty() {
            eprintln!("{shown}");
        }
    }

    /// One request, mapped to a success response or a classified error.
    async fn attempt(&self, spec: &RequestSpec, allow_404: bool) -> Attempt<HttpResponse> {
        let response = match self.send(&self.client, spec).await {
            Ok(response) => response,
            Err(e) => return Attempt::done(Err(transport_error(&e))),
        };
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = match read_body_with_limit(response).await {
            Ok(Some(body)) => body,
            Ok(None) => {
                return Attempt::done(Err(CalDavError::payload_too_large(format!(
                    "Response too large (exceeds {}MB limit)",
                    MAX_BODY_SIZE / 1_000_000
                ))));
            }
            Err(e) => return Attempt::done(Err(transport_error(&e))),
        };
        self.log_response(status, &spec.url, &body);
        if (200..=299).contains(&status) || (status == 404 && allow_404) {
            return Attempt::done(Ok(HttpResponse { headers, body }));
        }
        let retry_after_ms = if status == 429 { parse_retry_after(&headers) } else { 0 };
        Attempt { result: Err(status_error(status)), retry_after_ms }
    }

    /// Send `spec` with retry, then turn the success into a result. A failure
    /// from `process` counts against the circuit breaker like a request failure.
    async fn execute<T, P, F>(&self, spec: &RequestSpec, allow_404: bool, process: P) -> CalDavResult<T>
    where
        P: Fn(HttpResponse) -> F,
        F: Future<Output = CalDavResult<T>>,
    {
        self.with_retry(|| async {
            let attempt = self.attempt(spec, allow_404).await;
            match attempt.result {
                Ok(response) => Attempt::done(process(response).await),
                Err(e) => Attempt { result: Err(e), retry_after_ms: attempt.retry_after_ms },
            }
        })
        .await
    }

    /// Exponential backoff with jitter for retryable errors, honoring a capped
    /// Retry-After, behind a consecutive-failure circuit breaker.
    async fn with_retry<T, A, F>(&self, mut attempt: A) -> CalDavResult<T>
    where
        A: FnMut() -> F,
        F: Future<Output = Attempt<T>>,
    {
        if self.consecutive_failures.load(Ordering::SeqCst) >= CIRCUIT_BREAKER_THRESHOLD {
            return Err(CalDavError::new(503, "Circuit breaker open: too many consecutive failures", false));
        }
        let mut last_error = None;
        for retry in 0..=MAX_RETRIES {
            let Attempt { result, retry_after_ms } = attempt().await;
            let error = match result {
                Ok(value) => {
                    self.consecutive_failures.store(0, Ordering::SeqCst);
                    return Ok(value);
                }
                Err(error) => error,
            };
            if !error.retryable {
                self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
                return Err(error);
            }
            last_error = Some(error);
            if retry == MAX_RETRIES {
                break;
            }
            let backoff = BASE_DELAY_MS * (1 << retry) + fastrand::u64(0..MAX_JITTER_MS);
            tokio::time::sleep(Duration::from_millis(backoff.max(retry_after_ms))).await;
        }
        self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
        Err(last_error.unwrap_or_else(|| CalDavError::network("Request failed after retries")))
    }
}

fn default_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT).read_timeout(READ_TIMEOUT)
}

fn client_build_error(e: reqwest::Error) -> CalDavError {
    CalDavError::new(0, format!("Failed to create HTTP client: {e}"), false)
}

/// The body as text, or `None` when it exceeds [`MAX_BODY_SIZE`]. A declared
/// Content-Length over the cap is rejected before reading anything; chunked
/// bodies are cut off as soon as they cross it.
async fn read_body_with_limit(mut response: reqwest::Response) -> reqwest::Result<Option<String>> {
    if response.content_length().is_some_and(|len| len > MAX_BODY_SIZE as u64) {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_BODY_SIZE {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn status_error(status: u16) -> CalDavError {
    match status {
        401 => CalDavError::auth(),
        403 => CalDavError::forbidden(),
        404 => CalDavError::not_found("Resource not found"),
        412 => CalDavError::conflict(),
        429 => CalDavError::rate_limited(),
        500..=599 => CalDavError::server(status, "Server error"),
        _ => CalDavError::new(status, format!("Unexpected response: {status}"), false),
    }
}

/// The error and its causes, joined, for diagnostics.
fn error_chain(e: &reqwest::Error) -> String {
    let mut message = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// reqwest does not expose TLS failures as a kind, so look for them among the
/// error's causes (not its own message, which may include the URL).
fn is_tls_error(e: &reqwest::Error) -> bool {
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        let text = cause.to_string().to_ascii_lowercase();
        if text.contains("certificate") || text.contains("tls") || text.contains("ssl") {
            return true;
        }
        source = cause.source();
    }
    false
}

fn transport_error(e: &reqwest::Error) -> CalDavError {
    let detail = error_chain(e);
    if is_tls_error(e) {
        CalDavError::ssl(format!("SSL error: {detail}"))
    } else if e.is_timeout() {
        CalDavError::timeout(format!("Request timed out: {detail}"))
    } else {
        CalDavError::network(format!("Network error: {detail}"))
    }
}

/// Retry-After as seconds or an HTTP-date (RFC 7231), in milliseconds, capped
/// at [`MAX_RETRY_AFTER_MS`]. Absent or unparseable yields 0.
pub fn parse_retry_after(headers: &HeaderMap) -> u64 {
    headers.get("Retry-After").and_then(|v| v.to_str().ok()).map_or(0, retry_after_millis)
}

fn retry_after_millis(value: &str) -> u64 {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<i64>() {
        return u64::try_from(seconds.saturating_mul(1000)).unwrap_or(0).min(MAX_RETRY_AFTER_MS);
    }
    match chrono::DateTime::parse_from_rfc2822(value) {
        Ok(date) => {
            let delay = date.timestamp_millis() - chrono::Utc::now().timestamp_millis();
            u64::try_from(delay.max(0)).unwrap_or(0).min(MAX_RETRY_AFTER_MS)
        }
        Err(_) => 0,
    }
}

fn check_date(date: &str) -> bool {
    DATE_PATTERN.is_match(date)
}

#[async_trait]
impl CalDavClient for HttpCalDavClient {
    async fn list_calendars(&self) -> CalDavResult<Vec<CalDavCalendar>> {
        if let Some((calendars, fetched_at)) = &self.state().calendars
            && fetched_at.elapsed() < CACHE_TTL
        {
            return Ok(calendars.clone());
        }

        let effective_base = self.discover_well_known().await.unwrap_or_else(|| self.base_url.clone());
        let principal = self
            .discover_principal(&effective_base)
            .await?
            .ok_or_else(|| CalDavError::server(500, "Could not discover principal"))?;
        let home_sets = self.discover_home_sets(&principal).await?;
        let Some(first_home_set) = home_sets.first() else {
            return Err(CalDavError::server(500, "Could not discover calendar-home-set"));
        };
        self.state().calendar_home_set = Some(first_home_set.clone());

        // A home set that fails to list is skipped rather than failing the whole
        // discovery; calendars shared by several home sets are listed once.
        let mut calendars = Vec::new();
        let mut seen_hrefs = HashSet::new();
        for home_set in &home_sets {
            if let Ok(found) = self.fetch_calendars(home_set).await {
                calendars.extend(found.into_iter().filter(|cal| seen_hrefs.insert(cal.href.clone())));
            }
        }

        self.state().calendars = Some((calendars.clone(), Instant::now()));
        Ok(calendars)
    }

    async fn get_events(&self, calendar_id: &str, start_date: &str, end_date: &str) -> CalDavResult<Vec<CalDavEvent>> {
        // Defense in depth: the dates are interpolated into the REPORT body.
        if !check_date(start_date) {
            return Err(CalDavError::bad_request("Invalid start date format: expected YYYY-MM-DD"));
        }
        if !check_date(end_date) {
            return Err(CalDavError::bad_request("Invalid end date format: expected YYYY-MM-DD"));
        }
        let calendar = self.find_calendar(calendar_id).await?;
        let body = calendar_query_report(&start_date.replace('-', ""), &end_date.replace('-', ""), true);
        let spec = RequestSpec::new(report(), calendar.url).xml_body(body).header("Depth", "1");
        self.execute(&spec, false, |resp| async move { Ok(self.xml.parse_events(&resp.body, &self.base_url)) }).await
    }

    async fn get_event(&self, href: &str) -> CalDavResult<CalDavEvent> {
        let normalized_href = normalize_icloud_url(href);
        let url = self.absolute_url(&normalized_href);
        let spec = RequestSpec::new(Method::GET, url.clone());
        self.execute(&spec, false, |resp| {
            let (url, href) = (&url, &normalized_href);
            async move {
                let uid = self
                    .xml
                    .extract_uid(&resp.body)
                    .ok_or_else(|| CalDavError::not_found(format!("Event body missing UID at {href}")))?;
                let etag = match resp.etag_header() {
                    Some(etag) => Some(etag),
                    None => self.fetch_etag(url).await,
                };
                Ok(CalDavEvent { uid, href: href.clone(), url: url.clone(), etag, ical_data: resp.body })
            }
        })
        .await
    }

    async fn create_event(&self, calendar_id: &str, ical_data: &str) -> CalDavResult<CalDavEvent> {
        let calendar = self.find_calendar(calendar_id).await?;
        let uid = self.xml.extract_uid(ical_data).ok_or_else(|| CalDavError::bad_request("ICS data missing UID"))?;
        let event_url = format!("{}/{}.ics", calendar.url.trim_end_matches('/'), uuid::Uuid::new_v4());
        let spec = RequestSpec::new(Method::PUT, event_url.clone()).ics_body(ical_data).header("If-None-Match", "*");
        self.execute(&spec, false, |resp| {
            let (uid, event_url) = (&uid, &event_url);
            async move {
                let etag = match resp.etag_header() {
                    Some(etag) => Some(etag),
                    None => self.fetch_etag(event_url).await,
                };
                let href = event_url
                    .find(&self.base_url)
                    .map_or_else(|| event_url.clone(), |at| event_url[at + self.base_url.len()..].to_string());
                Ok(CalDavEvent {
                    uid: uid.clone(),
                    href,
                    url: event_url.clone(),
                    etag,
                    ical_data: ical_data.to_string(),
                })
            }
        })
        .await
    }

    async fn update_event(&self, href: &str, ical_data: &str, etag: Option<&str>) -> CalDavResult<CalDavEvent> {
        let url = self.absolute_url(href);
        let uid = self.xml.extract_uid(ical_data).ok_or_else(|| CalDavError::bad_request("ICS data missing UID"))?;
        // Without a caller ETag (some REPORT responses omit getetag), probe for
        // one so the PUT still carries If-Match; a failed probe falls back to an
        // unconditional PUT.
        let effective_etag = match etag {
            Some(etag) => Some(etag.to_string()),
            None => self.fetch_etag(&url).await,
        };
        let mut spec = RequestSpec::new(Method::PUT, url.clone()).ics_body(ical_data);
        if let Some(etag) = effective_etag {
            spec = spec.header("If-Match", etag);
        }
        self.execute(&spec, false, |resp| {
            let (uid, url) = (&uid, &url);
            async move {
                let new_etag = match resp.etag_header() {
                    Some(etag) => Some(etag),
                    None => self.fetch_etag(url).await,
                };
                Ok(CalDavEvent {
                    uid: uid.clone(),
                    href: href.to_string(),
                    url: url.clone(),
                    etag: new_etag,
                    ical_data: ical_data.to_string(),
                })
            }
        })
        .await
    }

    async fn delete_event(&self, href: &str, etag: Option<&str>) -> CalDavResult<()> {
        let url = self.absolute_url(href);
        let effective_etag = match etag {
            Some(etag) => Some(etag.to_string()),
            None => self.fetch_etag(&url).await,
        };
        let mut spec = RequestSpec::new(Method::DELETE, url);
        if let Some(etag) = effective_etag {
            spec = spec.header("If-Match", etag);
        }
        // Already gone counts as deleted.
        self.execute(&spec, true, |_| async { Ok(()) }).await
    }

    async fn check_connection(&self) -> CalDavResult<bool> {
        let spec = RequestSpec::new(Method::OPTIONS, self.base_url.clone());
        self.with_retry(|| async {
            let response = match self.send(&self.client, &spec).await {
                Ok(response) => response,
                Err(e) => return Attempt::done(Err(connection_error(&e))),
            };
            let result = match response.status() {
                StatusCode::UNAUTHORIZED => Err(CalDavError::auth()),
                StatusCode::FORBIDDEN => Err(CalDavError::forbidden()),
                status if !status.is_success() => Err(CalDavError::server(status.as_u16(), "Server error")),
                _ => {
                    let dav = response.headers().get("DAV").and_then(|v: &HeaderValue| v.to_str().ok()).unwrap_or("");
                    if dav.contains("calendar-access") {
                        Ok(true)
                    } else {
                        Err(CalDavError::bad_request(
                            "Server does not support CalDAV (missing 'calendar-access' in DAV header)",
                        ))
                    }
                }
            };
            Attempt::done(result)
        })
        .await
    }

    async fn fetch_etags(
        &self,
        calendar_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> CalDavResult<HashMap<String, Option<String>>> {
        if !check_date(start_date) || !check_date(end_date) {
            return Err(CalDavError::bad_request("Invalid date format"));
        }
        let calendar = self.find_calendar(calendar_id).await?;
        let body = calendar_query_report(&start_date.replace('-', ""), &end_date.replace('-', ""), false);
        let spec = RequestSpec::new(report(), calendar.url).xml_body(body).header("Depth", "1");
        self.execute(&spec, false, |resp| async move { Ok(self.xml.parse_etags(&resp.body)) }).await
    }
}

fn connection_error(e: &reqwest::Error) -> CalDavError {
    if is_tls_error(e) {
        CalDavError::ssl(format!("SSL error: {}", error_chain(e)))
    } else if e.is_timeout() {
        CalDavError::timeout("Connection timed out")
    } else {
        CalDavError::network(format!("Unable to connect: {}", error_chain(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(name: &'static str, value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(name, HeaderValue::from_str(value).unwrap());
        map
    }

    #[test]
    fn retry_after_seconds_is_capped() {
        assert_eq!(parse_retry_after(&headers("Retry-After", "1")), 1000);
        assert_eq!(parse_retry_after(&headers("Retry-After", "120")), MAX_RETRY_AFTER_MS);
        assert_eq!(parse_retry_after(&headers("Retry-After", "0")), 0);
        assert_eq!(parse_retry_after(&headers("Retry-After", "-5")), 0);
    }

    #[test]
    fn retry_after_http_date() {
        let future = (chrono::Utc::now() + chrono::TimeDelta::seconds(60)).to_rfc2822();
        assert_eq!(parse_retry_after(&headers("Retry-After", &future)), MAX_RETRY_AFTER_MS);
        assert_eq!(parse_retry_after(&headers("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")), 0);
    }

    #[test]
    fn retry_after_missing_or_garbage_is_zero() {
        assert_eq!(parse_retry_after(&HeaderMap::new()), 0);
        assert_eq!(parse_retry_after(&headers("Retry-After", "soon")), 0);
    }

    #[test]
    fn status_mapping() {
        assert_eq!(status_error(401), CalDavError::auth());
        assert_eq!(status_error(403), CalDavError::forbidden());
        assert!(status_error(404).is_not_found());
        assert_eq!(status_error(412).code, 412);
        assert!(status_error(429).retryable);
        let server = status_error(502);
        assert!(server.is_server_error() && server.retryable);
        let other = status_error(418);
        assert_eq!(other.message, "Unexpected response: 418");
        assert!(!other.retryable);
    }

    #[test]
    fn date_guard_is_ascii_only() {
        assert!(check_date("2025-01-31"));
        assert!(!check_date("2025<script>"));
        assert!(!check_date("not-a-date"));
        assert!(!check_date("２０２５-01-31"));
        assert!(!check_date("2025-01-31\n"));
    }

    #[test]
    fn report_body_includes_calendar_data_only_when_asked() {
        let full = calendar_query_report("20250101", "20250131", true);
        assert!(full.contains("<C:calendar-data/>"));
        assert!(full.contains(r#"start="20250101T000000Z" end="20250131T235959Z""#));
        assert!(!calendar_query_report("20250101", "20250131", false).contains("calendar-data"));
    }

    #[test]
    fn https_is_never_downgraded() {
        let client = HttpCalDavClient::with_clients(
            "https://caldav.icloud.com",
            CalDavCredentials::new("u@icloud.com", "p"),
            reqwest::Client::new(),
            reqwest::Client::new(),
        );
        assert_eq!(client.preserve_https("http://x.icloud.com/a".into()), "https://x.icloud.com/a");
        assert_eq!(client.preserve_https("https://x/a".into()), "https://x/a");
    }
}
