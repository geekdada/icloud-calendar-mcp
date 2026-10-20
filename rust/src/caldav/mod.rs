//! CalDAV protocol layer: domain models, the client abstraction, and the
//! iCloud implementation over HTTP.

pub mod etag;
pub mod http;
pub mod url_normalizer;
pub mod xml;

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;

pub use etag::normalize_etag;
pub use http::HttpCalDavClient;
pub use url_normalizer::normalize_icloud_url;
pub use xml::ICloudXmlParser;

/// Credentials for CalDAV authentication: for iCloud, the Apple ID and an
/// app-specific password.
#[derive(Clone, PartialEq, Eq)]
pub struct CalDavCredentials {
    pub username: String,
    pub password: String,
}

impl CalDavCredentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self { username: username.into(), password: password.into() }
    }

    /// True when both parts are non-blank.
    pub fn is_valid(&self) -> bool {
        !self.username.trim().is_empty() && !self.password.trim().is_empty()
    }
}

/// Never prints the password.
impl fmt::Debug for CalDavCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CalDavCredentials(username={}, password=***)", self.username)
    }
}

/// A failed CalDAV operation, carrying an HTTP-like status code.
///
/// Code 0 marks a transport failure (network or TLS) with no HTTP status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalDavError {
    pub code: u16,
    pub message: String,
    pub retryable: bool,
}

impl CalDavError {
    pub fn new(code: u16, message: impl Into<String>, retryable: bool) -> Self {
        Self { code, message: message.into(), retryable }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message, false)
    }

    pub fn auth() -> Self {
        Self::new(401, "Authentication failed", false)
    }

    pub fn forbidden() -> Self {
        Self::new(403, "Access denied", false)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, message, false)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(408, message, true)
    }

    pub fn conflict() -> Self {
        Self::new(412, "Precondition failed (resource modified)", false)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(413, message, false)
    }

    pub fn rate_limited() -> Self {
        Self::new(429, "Rate limited", true)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(0, message, true)
    }

    pub fn ssl(message: impl Into<String>) -> Self {
        Self::new(0, message, false)
    }

    pub fn server(code: u16, message: impl Into<String>) -> Self {
        Self::new(code, message, true)
    }

    /// 401 or 403.
    pub fn is_auth_error(&self) -> bool {
        self.code == 401 || self.code == 403
    }

    pub fn is_not_found(&self) -> bool {
        self.code == 404
    }

    /// Any 5xx.
    pub fn is_server_error(&self) -> bool {
        (500..=599).contains(&self.code)
    }

    /// 409 or 412.
    pub fn is_conflict(&self) -> bool {
        self.code == 409 || self.code == 412
    }
}

impl fmt::Display for CalDavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for CalDavError {}

pub type CalDavResult<T> = Result<T, CalDavError>;

/// A CalDAV calendar collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalDavCalendar {
    /// Last path segment of the href.
    pub id: String,
    /// Path of the collection, e.g. `/1234567/calendars/home/`.
    pub href: String,
    /// Absolute URL of the collection.
    pub url: String,
    pub display_name: String,
    /// `#RRGGBB`, or `None` when absent or malformed.
    pub color: Option<String>,
    pub ctag: Option<String>,
    /// True when the current user lacks write privilege (e.g. a shared calendar).
    pub is_read_only: bool,
    /// Component names from `supported-calendar-component-set`, in server order.
    pub supported_components: Vec<String>,
}

/// A CalDAV event resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalDavEvent {
    /// UID of the (first) VEVENT in the body.
    pub uid: String,
    /// Path of the `.ics` resource.
    pub href: String,
    /// Absolute URL of the `.ics` resource.
    pub url: String,
    /// Normalized ETag for conditional writes.
    pub etag: Option<String>,
    /// Raw ICS body.
    pub ical_data: String,
}

impl CalDavEvent {
    /// True when the event carries a usable ETag for `If-Match`.
    pub fn has_etag(&self) -> bool {
        self.etag.as_deref().is_some_and(|e| !e.trim().is_empty())
    }
}

/// The last non-empty path segment of `href` (`/123/calendars/home/` -> `home`).
pub fn extract_calendar_id_from_href(href: &str) -> String {
    href.trim_matches('/')
        .split('/')
        .rfind(|segment| !segment.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| href.to_string())
}

static HEX_COLOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#?([0-9A-Fa-f]{6,8})$").expect("valid regex"));

/// Normalize a calendar color to `#RRGGBB`.
///
/// Accepts `#RRGGBB`, `RRGGBB` and iCloud's `#RRGGBBAA` (alpha dropped);
/// anything else yields `None`.
pub fn normalize_color(color: Option<&str>) -> Option<String> {
    let trimmed = color?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let caps = HEX_COLOR.captures(trimmed)?;
    let hex = &caps[1];
    Some(format!("#{}", &hex[..6]))
}

/// CalDAV operations the service layer needs. Implemented over HTTP by
/// [`HttpCalDavClient`]; tests substitute an in-memory fake.
#[async_trait]
pub trait CalDavClient: Send + Sync {
    /// Discover and list every calendar the account can see.
    async fn list_calendars(&self) -> CalDavResult<Vec<CalDavCalendar>>;

    /// Events of `calendar_id` overlapping `start_date..=end_date` (`YYYY-MM-DD`, UTC days).
    async fn get_events(
        &self,
        calendar_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> CalDavResult<Vec<CalDavEvent>>;

    /// Fetch one event resource by href (or absolute URL), with its current ETag.
    async fn get_event(&self, href: &str) -> CalDavResult<CalDavEvent>;

    /// Create a new resource holding `ical_data` in `calendar_id`.
    async fn create_event(&self, calendar_id: &str, ical_data: &str) -> CalDavResult<CalDavEvent>;

    /// Replace the resource at `href`, conditional on `etag` when given.
    async fn update_event(
        &self,
        href: &str,
        ical_data: &str,
        etag: Option<&str>,
    ) -> CalDavResult<CalDavEvent>;

    /// Delete the resource at `href`, conditional on `etag` when given.
    async fn delete_event(&self, href: &str, etag: Option<&str>) -> CalDavResult<()>;

    /// Verify the server speaks CalDAV (`calendar-access` in the DAV header).
    async fn check_connection(&self) -> CalDavResult<bool>;

    /// href -> ETag for the events in a range, without calendar data.
    async fn fetch_etags(
        &self,
        calendar_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> CalDavResult<HashMap<String, Option<String>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_id_is_last_segment() {
        assert_eq!(extract_calendar_id_from_href("/1234567/calendars/home/"), "home");
        assert_eq!(extract_calendar_id_from_href("/calendars/work"), "work");
        assert_eq!(extract_calendar_id_from_href("single"), "single");
        assert_eq!(extract_calendar_id_from_href("///"), "///");
    }

    #[test]
    fn color_normalization() {
        assert_eq!(normalize_color(Some("#FF0000")), Some("#FF0000".into()));
        assert_eq!(normalize_color(Some("FF0000")), Some("#FF0000".into()));
        assert_eq!(normalize_color(Some("#FF0000FF")), Some("#FF0000".into()));
        assert_eq!(normalize_color(Some("  #abcdef  ")), Some("#abcdef".into()));
        assert_eq!(normalize_color(Some("red")), None);
        assert_eq!(normalize_color(Some("#FFF")), None);
        assert_eq!(normalize_color(Some("")), None);
        assert_eq!(normalize_color(None), None);
    }

    #[test]
    fn credentials_debug_masks_password() {
        let creds = CalDavCredentials::new("user@icloud.com", "abcd-efgh-ijkl-mnop");
        let shown = format!("{creds:?}");
        assert!(shown.contains("user@icloud.com"));
        assert!(!shown.contains("abcd-efgh"));
        assert!(creds.is_valid());
        assert!(!CalDavCredentials::new(" ", "x").is_valid());
    }

    #[test]
    fn error_classification() {
        assert!(CalDavError::auth().is_auth_error());
        assert!(CalDavError::forbidden().is_auth_error());
        assert!(CalDavError::not_found("x").is_not_found());
        assert!(CalDavError::server(503, "x").is_server_error());
        assert!(CalDavError::conflict().is_conflict());
        assert!(CalDavError::new(409, "x", false).is_conflict());
        assert!(CalDavError::timeout("x").retryable);
        assert!(!CalDavError::ssl("x").retryable);
    }
}
