//! Secure error results for MCP tool responses.
//!
//! Errors reach an LLM client verbatim, so every message is scrubbed of
//! credentials, home-directory paths and email addresses before it leaves the
//! server, and internal failures are reported without type names or details.

use std::sync::LazyLock;

use regex::{NoExpand, Regex};
use rmcp::model::{CallToolResult, ContentBlock};

/// Categorized error codes surfaced in the `error.code` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ValidationError,
    AuthenticationError,
    AuthorizationError,
    NotFound,
    Conflict,
    RateLimited,
    PayloadTooLarge,
    CaldavError,
    InternalError,
}

impl ErrorCode {
    /// The wire value of the code.
    pub fn code(self) -> &'static str {
        match self {
            ErrorCode::ValidationError => "VALIDATION_ERROR",
            ErrorCode::AuthenticationError => "AUTH_ERROR",
            ErrorCode::AuthorizationError => "FORBIDDEN",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::RateLimited => "RATE_LIMITED",
            ErrorCode::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            ErrorCode::CaldavError => "CALDAV_ERROR",
            ErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }

    /// The HTTP status the code corresponds to.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::ValidationError => 400,
            ErrorCode::AuthenticationError => 401,
            ErrorCode::AuthorizationError => 403,
            ErrorCode::NotFound => 404,
            ErrorCode::Conflict => 409,
            ErrorCode::RateLimited => 429,
            ErrorCode::PayloadTooLarge => 413,
            ErrorCode::CaldavError => 502,
            ErrorCode::InternalError => 500,
        }
    }
}

/// Error categories with context, retryability and a suggested next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarError {
    Validation {
        field: Option<String>,
        detail: String,
    },
    Authentication {
        reason: String,
    },
    Authorization {
        calendar_name: Option<String>,
        reason: String,
    },
    NotFound {
        resource_type: String,
        resource_id: Option<String>,
    },
    Conflict {
        event_title: Option<String>,
        reason: String,
    },
    RateLimited {
        retry_after_ms: u64,
    },
    Network {
        reason: String,
    },
    Timeout {
        timeout_ms: u64,
    },
    ServerError {
        server_code: u16,
        reason: String,
    },
    CalDav {
        reason: String,
    },
    Internal {
        reason: String,
    },
}

impl CalendarError {
    pub fn validation() -> Self {
        CalendarError::Validation {
            field: None,
            detail: "Invalid input".into(),
        }
    }

    pub fn authentication() -> Self {
        CalendarError::Authentication {
            reason: "Authentication failed".into(),
        }
    }

    pub fn authorization() -> Self {
        CalendarError::Authorization {
            calendar_name: None,
            reason: "Access denied".into(),
        }
    }

    pub fn not_found() -> Self {
        CalendarError::NotFound {
            resource_type: "resource".into(),
            resource_id: None,
        }
    }

    pub fn conflict() -> Self {
        CalendarError::Conflict {
            event_title: None,
            reason: "Resource was modified".into(),
        }
    }

    pub fn rate_limited() -> Self {
        CalendarError::RateLimited { retry_after_ms: 0 }
    }

    pub fn network() -> Self {
        CalendarError::Network {
            reason: "Network error".into(),
        }
    }

    pub fn timeout() -> Self {
        CalendarError::Timeout { timeout_ms: 0 }
    }

    pub fn server_error() -> Self {
        CalendarError::ServerError {
            server_code: 500,
            reason: "Server error".into(),
        }
    }

    pub fn caldav() -> Self {
        CalendarError::CalDav {
            reason: "Calendar service error".into(),
        }
    }

    pub fn internal() -> Self {
        CalendarError::Internal {
            reason: "Internal error".into(),
        }
    }

    /// The category's wire code.
    pub fn code(&self) -> &'static str {
        match self {
            CalendarError::Validation { .. } => "VALIDATION_ERROR",
            CalendarError::Authentication { .. } => "AUTH_ERROR",
            CalendarError::Authorization { .. } => "FORBIDDEN",
            CalendarError::NotFound { .. } => "NOT_FOUND",
            CalendarError::Conflict { .. } => "CONFLICT",
            CalendarError::RateLimited { .. } => "RATE_LIMITED",
            CalendarError::Network { .. } => "NETWORK_ERROR",
            CalendarError::Timeout { .. } => "TIMEOUT",
            CalendarError::ServerError { .. } => "SERVER_ERROR",
            CalendarError::CalDav { .. } => "CALDAV_ERROR",
            CalendarError::Internal { .. } => "INTERNAL_ERROR",
        }
    }

    /// HTTP-like status; 0 for a network failure with no response.
    pub fn http_status(&self) -> u16 {
        match self {
            CalendarError::Validation { .. } => 400,
            CalendarError::Authentication { .. } => 401,
            CalendarError::Authorization { .. } => 403,
            CalendarError::NotFound { .. } => 404,
            CalendarError::Conflict { .. } => 409,
            CalendarError::RateLimited { .. } => 429,
            CalendarError::Network { .. } => 0,
            CalendarError::Timeout { .. } => 408,
            CalendarError::ServerError { server_code, .. } => *server_code,
            CalendarError::CalDav { .. } => 502,
            CalendarError::Internal { .. } => 500,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CalendarError::RateLimited { .. }
                | CalendarError::Network { .. }
                | CalendarError::Timeout { .. }
                | CalendarError::ServerError { .. }
                | CalendarError::CalDav { .. }
        )
    }

    pub fn suggested_action(&self) -> &'static str {
        match self {
            CalendarError::Validation { .. } => "Check input parameters and retry",
            CalendarError::Authentication { .. } => "Verify credentials and retry",
            CalendarError::Authorization { .. } => "Check calendar sharing permissions",
            CalendarError::NotFound { .. } => "Verify the resource exists",
            CalendarError::Conflict { .. } => "Refresh and retry with updated data",
            CalendarError::RateLimited { .. } => "Wait and retry after the indicated delay",
            CalendarError::Network { .. } => "Check network connection and retry",
            CalendarError::Timeout { .. } => "Retry the request",
            CalendarError::ServerError { .. } => "Retry after a short delay",
            CalendarError::CalDav { .. } => "Retry or check server status",
            CalendarError::Internal { .. } => "Contact support if the issue persists",
        }
    }

    /// The coarser [`ErrorCode`] used in tool results.
    pub fn to_error_code(&self) -> ErrorCode {
        match self {
            CalendarError::Validation { .. } => ErrorCode::ValidationError,
            CalendarError::Authentication { .. } => ErrorCode::AuthenticationError,
            CalendarError::Authorization { .. } => ErrorCode::AuthorizationError,
            CalendarError::NotFound { .. } => ErrorCode::NotFound,
            CalendarError::Conflict { .. } => ErrorCode::Conflict,
            CalendarError::RateLimited { .. } => ErrorCode::RateLimited,
            CalendarError::Network { .. }
            | CalendarError::Timeout { .. }
            | CalendarError::ServerError { .. }
            | CalendarError::CalDav { .. } => ErrorCode::CaldavError,
            CalendarError::Internal { .. } => ErrorCode::InternalError,
        }
    }

    /// A message safe to show the caller.
    pub fn safe_message(&self) -> String {
        match self {
            CalendarError::Validation { detail, .. } => detail.clone(),
            CalendarError::Authentication { reason }
            | CalendarError::Authorization { reason, .. }
            | CalendarError::Conflict { reason, .. }
            | CalendarError::Network { reason }
            | CalendarError::ServerError { reason, .. }
            | CalendarError::CalDav { reason } => reason.clone(),
            CalendarError::NotFound { resource_type, .. } => format!("{resource_type} not found"),
            CalendarError::RateLimited { .. } => "Rate limit exceeded".into(),
            CalendarError::Timeout { .. } => "Request timed out".into(),
            CalendarError::Internal { .. } => "An internal error occurred".into(),
        }
    }
}

/// Transport failure categories, the Rust stand-in for the JVM exception types
/// the Kotlin build mapped in `fromException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    Timeout,
    UnknownHost(String),
    Tls,
    ConnectionRefused,
    Other(Option<String>),
}

/// Map a transport failure to an error category.
pub fn from_failure(kind: &FailureKind) -> CalendarError {
    match kind {
        FailureKind::Timeout => CalendarError::timeout(),
        FailureKind::UnknownHost(host) => CalendarError::Network {
            reason: format!("Unknown host: {host}"),
        },
        FailureKind::Tls => CalendarError::Authentication {
            reason: "Certificate error".into(),
        },
        FailureKind::ConnectionRefused => CalendarError::Network {
            reason: "Connection refused".into(),
        },
        FailureKind::Other(message) => CalendarError::Internal {
            reason: message.clone().unwrap_or_else(|| "Unknown error".into()),
        },
    }
}

/// Map an HTTP status to an error category.
pub fn from_http_code(code: u16, message: &str) -> CalendarError {
    let message = message.trim();
    match code {
        401 => CalendarError::authentication(),
        403 => CalendarError::authorization(),
        404 => CalendarError::not_found(),
        409 => CalendarError::conflict(),
        429 => CalendarError::rate_limited(),
        500..=599 => CalendarError::ServerError {
            server_code: code,
            reason: if message.is_empty() {
                "Server error".into()
            } else {
                message.into()
            },
        },
        _ => CalendarError::Internal {
            reason: if message.is_empty() {
                format!("Unexpected error ({code})")
            } else {
                message.into()
            },
        },
    }
}

/// Build an error tool result whose text is a small JSON document.
///
/// The JSON is assembled by hand (with spaces after `:` and `,`) so the text is
/// byte-identical to the Kotlin build's output.
pub fn create_error_result(
    code: ErrorCode,
    message: &str,
    details: Option<&str>,
) -> CallToolResult {
    let mut json = format!(
        r#"{{"success": false, "error": {{"code": "{}", "message": "{}""#,
        code.code(),
        sanitize_error_message(message)
    );
    if let Some(details) = details {
        json.push_str(&format!(
            r#", "details": "{}""#,
            sanitize_error_message(details)
        ));
    }
    json.push_str("}}");
    CallToolResult::error(vec![ContentBlock::text(json)])
}

/// Error result for failed input validation: the message itself when there is
/// one error, otherwise a summary with every message in `details`.
pub fn validation_error(errors: &[String]) -> CallToolResult {
    match errors {
        [single] => create_error_result(ErrorCode::ValidationError, single, None),
        _ => create_error_result(
            ErrorCode::ValidationError,
            "Multiple validation errors",
            Some(&errors.join("; ")),
        ),
    }
}

pub fn authentication_error(reason: &str) -> CallToolResult {
    create_error_result(ErrorCode::AuthenticationError, reason, None)
}

/// Error result for a service-layer failure carrying an HTTP-like status.
pub fn service_error(code: u16, message: &str) -> CallToolResult {
    let error_code = match code {
        400 => ErrorCode::ValidationError,
        401 => ErrorCode::AuthenticationError,
        403 => ErrorCode::AuthorizationError,
        404 => ErrorCode::NotFound,
        // A 412 that survives the service's one refetch-and-retry means the
        // event changed since the caller's handle was issued: a conflict.
        409 | 412 => ErrorCode::Conflict,
        413 => ErrorCode::PayloadTooLarge,
        429 => ErrorCode::RateLimited,
        500..=599 => ErrorCode::CaldavError,
        _ => ErrorCode::InternalError,
    };
    create_error_result(error_code, message, None)
}

/// Error result for a CalDAV transport failure, reduced to a fixed category
/// message so no internal detail leaks.
pub fn caldav_error(failure_message: Option<&str>) -> CallToolResult {
    let message = failure_message.unwrap_or("");
    let lower = message.to_lowercase();
    let safe_message = if message.contains("401") {
        "Authentication failed with iCloud"
    } else if message.contains("403") {
        "Access denied to calendar"
    } else if message.contains("404") {
        "Calendar or event not found"
    } else if lower.contains("timeout") {
        "Connection to iCloud timed out"
    } else if lower.contains("connect") {
        "Unable to connect to iCloud"
    } else {
        "Calendar service error"
    };
    create_error_result(ErrorCode::CaldavError, safe_message, None)
}

pub fn rate_limit_error(retry_after_ms: u64) -> CallToolResult {
    let retry_seconds = (retry_after_ms / 1000).max(1);
    create_error_result(
        ErrorCode::RateLimited,
        &format!("Rate limit exceeded. Please retry in {retry_seconds} seconds."),
        None,
    )
}

/// Error result for an unexpected failure. Only the failure's kind is logged
/// (to stderr); its message may hold sensitive data, so neither reaches the caller.
pub fn internal_error(kind: &str) -> CallToolResult {
    eprintln!("Internal error: {kind}");
    create_error_result(
        ErrorCode::InternalError,
        "An internal error occurred. Please try again.",
        None,
    )
}

/// Mask an email for display: `john.doe@icloud.com` -> `joh***@***.com`.
pub fn mask_email(email: &str) -> String {
    let keep_three = |part: &str| {
        if part.chars().count() <= 3 {
            "***".to_string()
        } else {
            format!("{}***", take_chars(part, 3))
        }
    };
    let Some((local, domain)) = email.split_once('@') else {
        return keep_three(email);
    };
    let masked_domain = match domain.rfind('.') {
        Some(dot) => format!("***{}", &domain[dot..]),
        None => "***".to_string(),
    };
    format!("{}@{masked_domain}", keep_three(local))
}

fn take_chars(text: &str, count: usize) -> &str {
    text.char_indices()
        .nth(count)
        .map_or(text, |(i, _)| &text[..i])
}

/// Longest message (in UTF-16 units, as Kotlin counted) sanitized in full;
/// anything longer is cut first so the regexes never see unbounded input.
const MAX_MESSAGE_LENGTH: usize = 2_000;

// Java's `\s`/`\S`/`\w` are ASCII-only by default; spelled out here because
// the Rust classes are Unicode-aware.
const WS: &str = r"[ \t\n\x0B\x0C\r]";
const NON_WS: &str = r"[^ \t\n\x0B\x0C\r]";

struct CredentialPattern {
    regex: Regex,
    replacement: &'static str,
    /// Port of a Kotlin `(?<![a-z])` lookbehind (case-insensitive): the match
    /// must not directly follow an ASCII letter.
    not_after_letter: bool,
}

static CREDENTIAL_PATTERNS: LazyLock<Vec<CredentialPattern>> = LazyLock::new(|| {
    let pattern = |keyword: &str, replacement, not_after_letter| CredentialPattern {
        regex: Regex::new(&format!("(?i){keyword}[=:]{WS}*{NON_WS}+")).expect("valid regex"),
        replacement,
        not_after_letter,
    };
    vec![
        pattern("password", "password=****", false),
        pattern("passwd", "passwd=****", false),
        pattern("pass", "pass=****", true),
        pattern("pwd", "pwd=****", true),
        pattern("token", "token=****", false),
        pattern("secret", "secret=****", false),
        pattern("api[_-]?key", "api_key=****", false),
    ]
});

static PATH_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r"/Users/[A-Za-z0-9_]+/").expect("valid regex"),
            "/****/",
        ),
        (
            Regex::new(r"/home/[A-Za-z0-9_]+/").expect("valid regex"),
            "/****/",
        ),
        (
            Regex::new(r"C:\\Users\\[A-Za-z0-9_]+\\").expect("valid regex"),
            r"C:\****\",
        ),
    ]
});

static EMAIL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([a-zA-Z0-9]{1,3})[a-zA-Z0-9._+-]*@[A-Za-z0-9_.-]+\.[A-Za-z0-9_]+")
        .expect("valid regex")
});

/// Replace every match of `regex` in `input` with `replacement`, skipping
/// matches that start right after an ASCII letter and retrying one character
/// later, which is how Java evaluates a leading negative lookbehind.
pub(crate) fn replace_not_after_letter(
    regex: &Regex,
    input: &str,
    mut replacement: impl FnMut(&regex::Captures<'_>) -> String,
) -> String {
    let mut out = String::with_capacity(input.len());
    let (mut copied_to, mut search_from) = (0, 0);
    while let Some(caps) = regex.captures_at(input, search_from) {
        let whole = caps.get(0).expect("group 0 always matches");
        if input[..whole.start()]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            search_from = next_char_boundary(input, whole.start());
            continue;
        }
        out.push_str(&input[copied_to..whole.start()]);
        out.push_str(&replacement(&caps));
        copied_to = whole.end();
        search_from = if whole.is_empty() {
            next_char_boundary(input, whole.end())
        } else {
            whole.end()
        };
        if search_from > input.len() {
            break;
        }
    }
    out.push_str(&input[copied_to.min(input.len())..]);
    out
}

fn next_char_boundary(text: &str, from: usize) -> usize {
    text[from..]
        .chars()
        .next()
        .map_or(text.len() + 1, |c| from + c.len_utf8())
}

/// Cut `message` to at most [`MAX_MESSAGE_LENGTH`] UTF-16 units, on a char boundary.
fn truncate_message(message: &str) -> String {
    if message.encode_utf16().count() <= MAX_MESSAGE_LENGTH {
        return message.to_string();
    }
    let mut units = 0;
    let end = message
        .char_indices()
        .find(|(_, c)| {
            units += c.len_utf16();
            units > MAX_MESSAGE_LENGTH
        })
        .map_or(message.len(), |(i, _)| i);
    format!("{}... [truncated]", &message[..end])
}

/// Strip credentials, home paths and emails, then escape for a JSON string.
fn sanitize_error_message(message: &str) -> String {
    let mut sanitized = truncate_message(message);
    for pattern in CREDENTIAL_PATTERNS.iter() {
        sanitized = if pattern.not_after_letter {
            replace_not_after_letter(&pattern.regex, &sanitized, |_| {
                pattern.replacement.to_string()
            })
        } else {
            pattern
                .regex
                .replace_all(&sanitized, NoExpand(pattern.replacement))
                .into_owned()
        };
    }
    for (regex, replacement) in PATH_PATTERNS.iter() {
        sanitized = regex
            .replace_all(&sanitized, NoExpand(replacement))
            .into_owned();
    }
    sanitized = EMAIL_PATTERN
        .replace_all(&sanitized, |caps: &regex::Captures<'_>| {
            format!("{}***@***.***", &caps[1])
        })
        .into_owned();
    escape_json(&sanitized)
}

/// JSON string escaping, applied last so it cannot disturb the patterns above.
///
/// Newlines and tabs become spaces and CRs are dropped, as in Kotlin; other
/// control characters are `\u` escaped (Kotlin left them raw, which produced
/// invalid JSON).
fn escape_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\t' => out.push(' '),
            '\r' => {}
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .expect("text content")
    }

    fn sanitized(message: &str) -> String {
        text(&create_error_result(
            ErrorCode::ValidationError,
            message,
            None,
        ))
    }

    #[test]
    fn result_has_expected_json_shape() {
        let result = create_error_result(ErrorCode::ValidationError, "Invalid input", None);
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            text(&result),
            r#"{"success": false, "error": {"code": "VALIDATION_ERROR", "message": "Invalid input"}}"#
        );
        assert!(result.structured_content.is_none());
    }

    #[test]
    fn result_includes_details_when_provided() {
        let result = create_error_result(
            ErrorCode::ValidationError,
            "Multiple errors",
            Some("Error 1; Error 2"),
        );
        assert!(text(&result).contains(r#""details": "Error 1; Error 2""#));
        assert!(serde_json::from_str::<serde_json::Value>(&text(&result)).is_ok());
    }

    #[test]
    fn credentials_are_masked() {
        let out = sanitized("Failed with password=secret123");
        assert!(!out.contains("secret123"));
        assert!(out.contains("password=****"));
        assert!(!sanitized("password: mysecret").contains("mysecret"));
        let out = sanitized("Invalid token=abc123def456");
        assert!(!out.contains("abc123def456") && out.contains("token=****"));
        let out = sanitized("Bad api_key=sk-abc123");
        assert!(!out.contains("sk-abc123") && out.contains("api_key=****"));
        assert!(!sanitized("Bad api-key=sk-xyz789").contains("sk-xyz789"));
        assert!(!sanitized("Bad apikey=secret").contains("secret"));
    }

    #[test]
    fn pass_and_pwd_need_a_word_start() {
        assert_eq!(sanitized("pass=hunter2"), sanitized("pass=****"));
        assert!(sanitized("pwd: hunter2").contains("pwd=****"));
        // "bypass=" is preceded by a letter, so the bare `pass` rule must not fire.
        assert!(sanitized("bypass=open").contains("bypass=open"));
        assert!(sanitized("xpwd=open").contains("xpwd=open"));
        // A rejected candidate does not hide a valid one that starts inside it.
        assert!(sanitized("xpass=pass=hunter2").contains("xpass=pass=****"));
    }

    #[test]
    fn user_paths_are_masked() {
        let out = sanitized("File not found: /Users/johndoe/Documents/secret.txt");
        assert!(!out.contains("johndoe") && out.contains("/****/"));
        assert!(!sanitized("File not found: /home/developer/config.json").contains("developer"));
        assert!(!sanitized(r"File not found: C:\Users\alice\Documents\file.txt").contains("alice"));
    }

    #[test]
    fn emails_are_partially_masked() {
        let out = sanitized("Unknown user: john.doe@icloud.com");
        assert!(!out.contains("john.doe@icloud.com"));
        assert!(out.contains("joh***@***.***"));
        let out = sanitized("Conflict: alice@example.com vs bob@example.com");
        assert!(!out.contains("alice@example.com") && !out.contains("bob@example.com"));
    }

    #[test]
    fn json_special_characters_are_escaped() {
        assert!(sanitized(r#"Invalid value: "test""#).contains(r#"\"test\""#));
        assert!(sanitized(r"Path: C:\temp").contains(r"\\"));
        let out = sanitized("Line1\nLine2\nLine3");
        assert!(!out.contains('\n') && out.contains("Line1 Line2 Line3"));
        assert!(!sanitized("Line1\r\nLine2").contains('\r'));
        let out = sanitized("Col1\tCol2");
        assert!(!out.contains('\t') && out.contains("Col1 Col2"));
        let out = sanitized("nul\u{0}byte");
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
    }

    #[test]
    fn caldav_errors_are_categorized() {
        let out = text(&caldav_error(Some("HTTP 401 Unauthorized")));
        assert!(out.contains("Authentication failed") && !out.contains("401"));
        assert!(text(&caldav_error(Some("HTTP 403 Forbidden"))).contains("Access denied"));
        assert!(text(&caldav_error(Some("HTTP 404 Not Found"))).contains("not found"));
        assert!(text(&caldav_error(Some("Connection timeout after 30s"))).contains("timed out"));
        assert!(
            text(&caldav_error(Some("Unable to connect to host"))).contains("Unable to connect")
        );
        let out = text(&caldav_error(Some(
            "Some internal server error with stack trace",
        )));
        assert!(out.contains("Calendar service error") && !out.contains("stack trace"));
        assert!(text(&caldav_error(None)).contains("Calendar service error"));
    }

    #[test]
    fn internal_error_is_generic() {
        let out = text(&internal_error(
            "NullPointerException at com.example.Secret.method(Secret.kt:42)",
        ));
        assert!(out.contains("internal error"));
        assert!(
            !out.contains("NullPointerException")
                && !out.contains("Secret.kt")
                && !out.contains(":42")
        );
        let out = text(&internal_error("SecretInternalClass"));
        assert!(!out.contains("SecretInternalClass"));
    }

    #[test]
    fn validation_error_single_and_multiple() {
        let out = text(&validation_error(&["Title is required".into()]));
        assert!(out.contains("Title is required") && !out.contains("details"));
        let out = text(&validation_error(&[
            "Title is required".into(),
            "Start date is invalid".into(),
            "Calendar ID not found".into(),
        ]));
        assert!(out.contains("Multiple validation errors") && out.contains("details"));
        assert!(out.contains("Title is required") && out.contains("Start date is invalid"));
    }

    #[test]
    fn service_error_maps_status_codes() {
        let code_of = |status| {
            let out = text(&service_error(status, "x"));
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            v["error"]["code"].as_str().unwrap().to_string()
        };
        assert_eq!(code_of(400), "VALIDATION_ERROR");
        assert_eq!(code_of(401), "AUTH_ERROR");
        assert_eq!(code_of(403), "FORBIDDEN");
        assert_eq!(code_of(404), "NOT_FOUND");
        assert_eq!(code_of(409), "CONFLICT");
        assert_eq!(code_of(413), "PAYLOAD_TOO_LARGE");
        assert_eq!(code_of(429), "RATE_LIMITED");
        assert_eq!(code_of(503), "CALDAV_ERROR");
        assert_eq!(code_of(412), "CONFLICT");
        assert_eq!(code_of(0), "INTERNAL_ERROR");
    }

    #[test]
    fn rate_limit_error_reports_whole_seconds() {
        assert!(text(&rate_limit_error(500)).contains("retry in 1 seconds"));
        assert!(text(&rate_limit_error(42_000)).contains("retry in 42 seconds"));
    }

    #[test]
    fn calendar_error_properties() {
        let v = CalendarError::Validation {
            field: Some("title".into()),
            detail: "Required".into(),
        };
        assert!(!v.is_retryable());
        assert_eq!(v.code(), "VALIDATION_ERROR");
        assert_eq!(v.suggested_action(), "Check input parameters and retry");
        assert_eq!(v.safe_message(), "Required");
        assert_eq!(CalendarError::authentication().http_status(), 401);
        assert!(!CalendarError::authentication().is_retryable());
        let a = CalendarError::Authorization {
            calendar_name: Some("Work".into()),
            reason: "Access denied".into(),
        };
        assert_eq!(a.suggested_action(), "Check calendar sharing permissions");
        let n = CalendarError::NotFound {
            resource_type: "calendar".into(),
            resource_id: Some("home".into()),
        };
        assert_eq!(n.safe_message(), "calendar not found");
        assert_eq!(
            CalendarError::conflict().suggested_action(),
            "Refresh and retry with updated data"
        );
        assert!(
            CalendarError::RateLimited {
                retry_after_ms: 5000
            }
            .is_retryable()
        );
        let net = CalendarError::Network {
            reason: "Connection refused".into(),
        };
        assert!(net.is_retryable());
        assert_eq!(net.safe_message(), "Connection refused");
        assert_eq!(
            CalendarError::Timeout { timeout_ms: 30_000 }.http_status(),
            408
        );
        let s = CalendarError::ServerError {
            server_code: 503,
            reason: "Service Unavailable".into(),
        };
        assert!(s.is_retryable());
        assert_eq!(s.http_status(), 503);
        assert!(!CalendarError::internal().is_retryable());
        assert_eq!(
            CalendarError::internal().safe_message(),
            "An internal error occurred"
        );
    }

    #[test]
    fn to_error_code_mapping() {
        assert_eq!(
            CalendarError::validation().to_error_code(),
            ErrorCode::ValidationError
        );
        assert_eq!(
            CalendarError::authentication().to_error_code(),
            ErrorCode::AuthenticationError
        );
        assert_eq!(
            CalendarError::not_found().to_error_code(),
            ErrorCode::NotFound
        );
        assert_eq!(
            CalendarError::conflict().to_error_code(),
            ErrorCode::Conflict
        );
        assert_eq!(
            CalendarError::rate_limited().to_error_code(),
            ErrorCode::RateLimited
        );
        assert_eq!(
            CalendarError::network().to_error_code(),
            ErrorCode::CaldavError
        );
        assert_eq!(
            CalendarError::timeout().to_error_code(),
            ErrorCode::CaldavError
        );
        assert_eq!(
            CalendarError::server_error().to_error_code(),
            ErrorCode::CaldavError
        );
        assert_eq!(
            CalendarError::internal().to_error_code(),
            ErrorCode::InternalError
        );
    }

    #[test]
    fn every_category_has_a_suggested_action() {
        let all = [
            CalendarError::validation(),
            CalendarError::authentication(),
            CalendarError::authorization(),
            CalendarError::not_found(),
            CalendarError::conflict(),
            CalendarError::rate_limited(),
            CalendarError::network(),
            CalendarError::timeout(),
            CalendarError::server_error(),
            CalendarError::caldav(),
            CalendarError::internal(),
        ];
        assert!(all.iter().all(|e| !e.suggested_action().trim().is_empty()));
    }

    #[test]
    fn error_code_http_statuses() {
        assert_eq!(ErrorCode::ValidationError.http_status(), 400);
        assert_eq!(ErrorCode::AuthenticationError.http_status(), 401);
        assert_eq!(ErrorCode::AuthorizationError.http_status(), 403);
        assert_eq!(ErrorCode::NotFound.http_status(), 404);
        assert_eq!(ErrorCode::Conflict.http_status(), 409);
        assert_eq!(ErrorCode::RateLimited.http_status(), 429);
        assert_eq!(ErrorCode::InternalError.http_status(), 500);
        assert_eq!(ErrorCode::CaldavError.http_status(), 502);
    }

    #[test]
    fn failure_mapping() {
        let t = from_failure(&FailureKind::Timeout);
        assert!(matches!(t, CalendarError::Timeout { .. }) && t.is_retryable());
        let h = from_failure(&FailureKind::UnknownHost("caldav.icloud.com".into()));
        assert!(matches!(h, CalendarError::Network { .. }) && h.is_retryable());
        let tls = from_failure(&FailureKind::Tls);
        assert!(matches!(tls, CalendarError::Authentication { .. }) && !tls.is_retryable());
        let refused = from_failure(&FailureKind::ConnectionRefused);
        assert!(matches!(refused, CalendarError::Network { .. }) && refused.is_retryable());
    }

    #[test]
    fn http_code_mapping() {
        assert!(matches!(
            from_http_code(401, ""),
            CalendarError::Authentication { .. }
        ));
        assert!(matches!(
            from_http_code(403, ""),
            CalendarError::Authorization { .. }
        ));
        assert!(matches!(
            from_http_code(429, ""),
            CalendarError::RateLimited { .. }
        ));
        let s = from_http_code(503, "Service Unavailable");
        assert!(matches!(s, CalendarError::ServerError { .. }) && s.is_retryable());
        assert_eq!(
            from_http_code(418, "").safe_message(),
            "An internal error occurred"
        );
    }

    #[test]
    fn mask_email_variants() {
        assert_eq!(mask_email("john.doe@icloud.com"), "joh***@***.com");
        let short = mask_email("ab@x.co");
        assert!(short.contains('@') && !short.contains("ab@x.co"));
        let none = mask_email("no-at-sign");
        assert!(!none.contains("no-at-sign") && none.contains("***"));
        let sub = mask_email("user@sub.domain.com");
        assert!(sub.contains(".com") && !sub.contains("sub.domain"));
    }

    #[test]
    fn combined_sanitization() {
        let out = text(&create_error_result(
            ErrorCode::AuthenticationError,
            "Failed for user@example.com with password=secret at /Users/admin/app token=abc123",
            None,
        ));
        assert!(!out.contains("user@example.com") && !out.contains("secret"));
        assert!(!out.contains("admin") && !out.contains("abc123"));
        assert!(out.contains("***@***.***"));
        assert!(out.contains("password=****") && out.contains("token=****"));
    }

    #[test]
    fn long_messages_are_truncated() {
        let out = sanitized(&format!("Error: {}", "X".repeat(100_000)));
        assert!(out.contains("... [truncated]"));
        assert!(out.len() < 2_200);
    }

    #[test]
    fn sanitization_stress_completes() {
        let mut message = String::new();
        for i in 0..1000 {
            message.push_str(&format!(
                "password=secret{i} token=tok{i} user{i}@example.com /Users/user{i}/secret "
            ));
        }
        let out = text(&create_error_result(
            ErrorCode::InternalError,
            &message,
            None,
        ));
        assert!(!out.contains("secret0"));
    }

    #[test]
    fn unicode_is_preserved() {
        assert!(sanitized("Calendar '日历' not found").contains("日历"));
        assert!(!sanitized("Unknown user: user@例え.com").is_empty());
    }
}
