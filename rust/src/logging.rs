//! MCP logging: messages sent to the connected client as
//! `notifications/message`, used for the audit trail of create/update/delete.
//!
//! Every message and string value is scrubbed of credentials and email
//! addresses before it is sent, and sending is rate limited so a flood of
//! events cannot flood the client.

// rmcp marks LoggingLevel deprecated (SEP-2577) but it is still the wire type
// for `notifications/message` and `logging/setLevel`.
#![allow(deprecated)]

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use regex::Regex;
use rmcp::model::LoggingLevel;
use serde_json::{Map, Value};

use crate::error::replace_not_after_letter;

/// Where log notifications go. The server implements this over its MCP peer;
/// tests use [`MemorySink`].
pub trait LogSink: Send + Sync {
    fn send(&self, level: LoggingLevel, logger: &str, data: Value);
}

/// Discards every message.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl LogSink for NullSink {
    fn send(&self, _level: LoggingLevel, _logger: &str, _data: Value) {}
}

/// One message captured by [`MemorySink`].
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    pub level: LoggingLevel,
    pub logger: String,
    pub data: Value,
}

/// Records every message, for assertions in tests.
#[derive(Debug, Default)]
pub struct MemorySink {
    records: Mutex<Vec<LogRecord>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<LogRecord> {
        self.records.lock().expect("sink lock").clone()
    }

    pub fn count(&self) -> usize {
        self.records.lock().expect("sink lock").len()
    }

    pub fn clear(&self) {
        self.records.lock().expect("sink lock").clear();
    }
}

impl LogSink for MemorySink {
    fn send(&self, level: LoggingLevel, logger: &str, data: Value) {
        self.records.lock().expect("sink lock").push(LogRecord {
            level,
            logger: logger.to_string(),
            data,
        });
    }
}

/// Severity rank (RFC 5424 order, debug lowest), for level filtering.
fn rank(level: LoggingLevel) -> u8 {
    match level {
        LoggingLevel::Debug => 0,
        LoggingLevel::Info => 1,
        LoggingLevel::Notice => 2,
        LoggingLevel::Warning => 3,
        LoggingLevel::Error => 4,
        LoggingLevel::Critical => 5,
        LoggingLevel::Alert => 6,
        LoggingLevel::Emergency => 7,
    }
}

const LEVELS: [LoggingLevel; 8] = [
    LoggingLevel::Debug,
    LoggingLevel::Info,
    LoggingLevel::Notice,
    LoggingLevel::Warning,
    LoggingLevel::Error,
    LoggingLevel::Critical,
    LoggingLevel::Alert,
    LoggingLevel::Emergency,
];

/// The level named by `level` (case-insensitive; `warn` is accepted), if any.
pub fn parse_level(level: &str) -> Option<LoggingLevel> {
    match level.to_lowercase().as_str() {
        "debug" => Some(LoggingLevel::Debug),
        "info" => Some(LoggingLevel::Info),
        "notice" => Some(LoggingLevel::Notice),
        "warning" | "warn" => Some(LoggingLevel::Warning),
        "error" => Some(LoggingLevel::Error),
        "critical" => Some(LoggingLevel::Critical),
        "alert" => Some(LoggingLevel::Alert),
        "emergency" => Some(LoggingLevel::Emergency),
        _ => None,
    }
}

struct KeyValuePattern {
    name: &'static str,
    regex: Regex,
    /// Port of a Kotlin `(?<![a-z])` lookbehind: no ASCII letter right before.
    not_after_letter: bool,
}

// Java's `\s` is ASCII-only; spelled out because Rust's `\s` is Unicode-aware.
const WS: &str = r"[ \t\n\x0B\x0C\r]";
const VALUE: &str = r#"[^"' \t\n\x0B\x0C\r,}]"#;

static KEY_VALUE_PATTERNS: LazyLock<Vec<KeyValuePattern>> = LazyLock::new(|| {
    let simple = |keyword: &str| format!(r#"(?i){keyword}["']?{WS}*[:=]{WS}*["']?({VALUE}+)["']?"#);
    let pattern = |name, source: String, not_after_letter| KeyValuePattern {
        name,
        regex: Regex::new(&source).expect("valid regex"),
        not_after_letter,
    };
    vec![
        pattern("password", simple("password"), false),
        pattern("passwd", simple("passwd"), false),
        pattern("pass", simple("pass"), true),
        pattern("pwd", simple("pwd"), true),
        pattern("token", simple("token"), false),
        pattern("api_key", simple("api[_-]?key"), false),
        pattern("secret", simple("secret"), false),
        pattern(
            "authorization",
            format!(
                r#"(?i)authorization["']?{WS}*[:=]{WS}*["']?({VALUE}+(?:{WS}+{VALUE}+)*)["']?"#
            ),
            false,
        ),
    ]
});

static EMAIL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").expect("valid regex")
});

const REDACTED: &str = "[REDACTED]";

/// Redact the captured value inside the matched text, or fall back to
/// `name:[REDACTED]` when nothing was captured.
fn redact_match(name: &str, caps: &regex::Captures<'_>) -> String {
    let full = &caps[0];
    match caps.get(1).map(|m| m.as_str()).filter(|v| !v.is_empty()) {
        Some(value) => full.replace(value, REDACTED),
        None => format!("{name}:{REDACTED}"),
    }
}

/// Remove credential values and email addresses from `input`.
pub fn sanitize(input: &str) -> String {
    let mut result = input.to_string();
    for pattern in KEY_VALUE_PATTERNS.iter() {
        let redact = |caps: &regex::Captures<'_>| redact_match(pattern.name, caps);
        result = if pattern.not_after_letter {
            replace_not_after_letter(&pattern.regex, &result, redact)
        } else {
            pattern.regex.replace_all(&result, redact).into_owned()
        };
    }
    EMAIL_PATTERN.replace_all(&result, REDACTED).into_owned()
}

/// Sends sanitized, rate-limited log messages to the MCP client.
///
/// Data entries are `serde_json::Value`s: strings are sanitized, numbers,
/// booleans and null pass through, and arrays/objects are sanitized as their
/// JSON text (matching Kotlin's `toString()` fallback).
pub struct McpLogger {
    sink: Arc<dyn LogSink>,
    logger_name: String,
    rate_limit: u32,
    burst_limit: u32,
    window_ms: u64,
    min_level: AtomicU8,
    log_count: AtomicU32,
    burst_count: AtomicU32,
    /// Window start, in millis since `epoch`.
    window_start: AtomicU64,
    epoch: Instant,
}

impl McpLogger {
    /// A logger named `icloud-calendar-mcp` with the default limits (10/s, burst 50).
    pub fn new(sink: Arc<dyn LogSink>) -> Self {
        Self::with_limits(sink, "icloud-calendar-mcp", 10, 50, 1000)
    }

    pub fn with_limits(
        sink: Arc<dyn LogSink>,
        logger_name: &str,
        rate_limit: u32,
        burst_limit: u32,
        window_ms: u64,
    ) -> Self {
        Self {
            sink,
            logger_name: logger_name.to_string(),
            rate_limit,
            burst_limit,
            window_ms,
            min_level: AtomicU8::new(rank(LoggingLevel::Info)),
            log_count: AtomicU32::new(0),
            burst_count: AtomicU32::new(0),
            window_start: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }

    /// Drop messages below `level` from now on (`logging/setLevel`).
    pub fn set_level(&self, level: LoggingLevel) {
        self.min_level.store(rank(level), Ordering::SeqCst);
    }

    pub fn level(&self) -> LoggingLevel {
        LEVELS[usize::from(self.min_level.load(Ordering::SeqCst))]
    }

    pub fn sanitize(&self, input: &str) -> String {
        sanitize(input)
    }

    pub fn debug(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Debug, message, data);
    }

    pub fn info(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Info, message, data);
    }

    pub fn notice(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Notice, message, data);
    }

    pub fn warning(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Warning, message, data);
    }

    pub fn error(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Error, message, data);
    }

    pub fn critical(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Critical, message, data);
    }

    pub fn alert(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Alert, message, data);
    }

    pub fn emergency(&self, message: &str, data: &[(&str, Value)]) {
        self.log(LoggingLevel::Emergency, message, data);
    }

    /// Send `message` with `data` at `level`, unless filtered or rate limited.
    pub fn log(&self, level: LoggingLevel, message: &str, data: &[(&str, Value)]) {
        if rank(level) < self.min_level.load(Ordering::SeqCst) || !self.acquire_log_token() {
            return;
        }
        let mut payload = Map::new();
        payload.insert("message".into(), Value::String(sanitize(message)));
        for (key, value) in data {
            payload.insert((*key).to_string(), sanitize_value(value));
        }
        self.sink
            .send(level, &self.logger_name, Value::Object(payload));
    }

    /// Clear the rate-limit counters and start a new window.
    pub fn reset_rate_limit(&self) {
        self.log_count.store(0, Ordering::SeqCst);
        self.burst_count.store(0, Ordering::SeqCst);
        self.window_start.store(self.now_ms(), Ordering::SeqCst);
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Same admission rule as the Kotlin build: the burst counter is the
    /// effective cap within a window (the per-window rate check can never be the
    /// deciding factor once the burst check has passed).
    fn acquire_log_token(&self) -> bool {
        let now = self.now_ms();
        let start = self.window_start.load(Ordering::SeqCst);
        if now.saturating_sub(start) >= self.window_ms
            && self
                .window_start
                .compare_exchange(start, now, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            self.log_count.store(0, Ordering::SeqCst);
            self.burst_count.store(0, Ordering::SeqCst);
        }
        let current_burst = self
            .burst_count
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        if current_burst > self.burst_limit {
            return false;
        }
        let current_rate = self
            .log_count
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        current_rate <= self.rate_limit || current_burst <= self.burst_limit
    }
}

fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(sanitize(s)),
        Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
        other => Value::String(sanitize(&other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn logger_with(sink: &Arc<MemorySink>, rate: u32, burst: u32) -> McpLogger {
        McpLogger::with_limits(sink.clone(), "test-logger", rate, burst, 1000)
    }

    #[test]
    fn sanitize_removes_credentials_and_emails() {
        let out = sanitize(r#"{"password": "secret123", "user": "test"}"#);
        assert!(!out.contains("secret123") && out.contains(REDACTED));
        let out = sanitize("api_key: my-secret-key-12345");
        assert!(!out.contains("my-secret-key") && out.contains(REDACTED));
        let out = sanitize("User email: user@example.com logged in");
        assert!(!out.contains("user@example.com") && out.contains(REDACTED));
        let out = sanitize("Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
        assert!(!out.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9") && out.contains(REDACTED));
    }

    #[test]
    fn set_level_changes_minimum() {
        let logger = McpLogger::new(Arc::new(NullSink));
        assert_eq!(logger.level(), LoggingLevel::Info);
        logger.set_level(LoggingLevel::Warning);
        assert_eq!(logger.level(), LoggingLevel::Warning);
        logger.set_level(LoggingLevel::Debug);
        assert_eq!(logger.level(), LoggingLevel::Debug);
    }

    #[test]
    fn parse_level_names() {
        assert_eq!(parse_level("debug"), Some(LoggingLevel::Debug));
        assert_eq!(parse_level("info"), Some(LoggingLevel::Info));
        assert_eq!(parse_level("notice"), Some(LoggingLevel::Notice));
        assert_eq!(parse_level("warning"), Some(LoggingLevel::Warning));
        assert_eq!(parse_level("warn"), Some(LoggingLevel::Warning));
        assert_eq!(parse_level("error"), Some(LoggingLevel::Error));
        assert_eq!(parse_level("critical"), Some(LoggingLevel::Critical));
        assert_eq!(parse_level("alert"), Some(LoggingLevel::Alert));
        assert_eq!(parse_level("emergency"), Some(LoggingLevel::Emergency));
        assert_eq!(parse_level("invalid"), None);
        assert_eq!(parse_level("DEBUG"), Some(LoggingLevel::Debug));
        assert_eq!(parse_level("Error"), Some(LoggingLevel::Error));
    }

    #[test]
    fn messages_within_limit_are_sent() {
        let sink = Arc::new(MemorySink::new());
        let logger = logger_with(&sink, 5, 10);
        (0..5).for_each(|i| logger.info(&format!("Message {i}"), &[]));
        assert_eq!(sink.count(), 5);
    }

    #[test]
    fn reset_rate_limit_allows_more() {
        let sink = Arc::new(MemorySink::new());
        let logger = logger_with(&sink, 2, 2);
        (0..10).for_each(|_| logger.info("Message", &[]));
        logger.reset_rate_limit();
        sink.clear();
        logger.info("After reset", &[]);
        assert!(sink.count() >= 1);
    }

    #[test]
    fn flood_is_capped_by_burst_limit() {
        let sink = Arc::new(MemorySink::new());
        let logger = logger_with(&sink, 5, 10);
        (0..1000).for_each(|i| logger.info(&format!("Flood message {i}"), &[]));
        assert!(sink.count() <= 20);
    }

    #[test]
    fn below_min_level_is_dropped() {
        let sink = Arc::new(MemorySink::new());
        let logger = logger_with(&sink, 10, 50);
        logger.debug("hidden", &[]);
        assert_eq!(sink.count(), 0);
        logger.set_level(LoggingLevel::Debug);
        logger.debug("shown", &[]);
        assert_eq!(sink.count(), 1);
    }

    #[test]
    fn payload_has_message_first_then_sanitized_data() {
        let sink = Arc::new(MemorySink::new());
        let logger = logger_with(&sink, 10, 50);
        logger.info(
            "Event created for bob@example.com",
            &[
                ("tool", json!("create_event")),
                ("count", json!(3)),
                ("ok", json!(true)),
                ("none", Value::Null),
                ("who", json!("alice@example.com")),
                ("nested", json!({"password": "hunter2"})),
            ],
        );
        let record = &sink.records()[0];
        assert_eq!(record.level, LoggingLevel::Info);
        assert_eq!(record.logger, "test-logger");
        let object = record.data.as_object().unwrap();
        assert_eq!(object.keys().next().map(String::as_str), Some("message"));
        assert_eq!(
            record.data["message"],
            json!("Event created for [REDACTED]")
        );
        assert_eq!(record.data["tool"], json!("create_event"));
        assert_eq!(record.data["count"], json!(3));
        assert_eq!(record.data["ok"], json!(true));
        assert_eq!(record.data["none"], Value::Null);
        assert_eq!(record.data["who"], json!("[REDACTED]"));
        assert!(!record.data["nested"].as_str().unwrap().contains("hunter2"));
    }

    #[test]
    fn injection_and_format_strings_are_literal() {
        assert!(sanitize("Normal log\n[CRITICAL] Fake critical message").contains("Normal log"));
        assert!(sanitize("Request completed\r\n\r\nHTTP/1.1 200 OK").contains("Request completed"));
        assert_eq!(sanitize("%s%s%s%n%n%n"), "%s%s%s%n%n%n");
        assert!(sanitize(r#"{"level":"emergency","message":"FAKE ALERT"}"#).contains("level"));
        assert!(sanitize("Normal\\x1b[31mRED TEXT\\x1b[0m").contains("Normal"));
    }

    #[test]
    fn password_variants_are_redacted() {
        for input in ["passwd=secret123", "password=secret123"] {
            assert!(!sanitize(input).contains("secret123"), "{input}");
        }
        for input in [
            r#"password="secret123""#,
            "password='secret123'",
            "password=`secret123`",
            "password:secret123",
            "password = secret123",
            "password  =  secret123",
            "password\t=\tsecret123",
            "password:  secret123",
        ] {
            assert!(!sanitize(input).contains("secret123"), "{input}");
        }
    }

    #[test]
    fn pass_and_pwd_need_a_word_start() {
        assert_eq!(sanitize("pass=hunter2"), "pass=[REDACTED]");
        assert_eq!(sanitize("pwd: hunter2"), "pwd: [REDACTED]");
        assert_eq!(sanitize("bypass=open"), "bypass=open");
    }

    #[test]
    fn multiple_secrets_in_one_message() {
        let out = sanitize(
            "config = {\n    password: \"secret1\",\n    api_key: \"key123\",\n    token: \"tok456\",\n    email: \"user@example.com\"\n}",
        );
        for secret in ["secret1", "key123", "tok456", "user@example.com"] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
    }

    #[test]
    fn secret_and_bearer_tokens() {
        assert!(sanitize("secret=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY").contains(REDACTED));
        let bearer = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert!(!sanitize(bearer).contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
    }

    #[test]
    fn email_detection() {
        assert!(!sanitize("Email: user@example.com").contains("user@example.com"));
        assert!(sanitize("Email: user@xn--e1afmkfd.xn--p1ai").contains(REDACTED));
    }

    #[test]
    fn hostile_input_does_not_panic_or_hang() {
        let inputs = [
            "Normal\u{0}NULL\u{7}BELL\u{8}BACKSPACE\u{1B}ESCAPE".to_string(),
            "\u{1B}[2J\u{1B}[H".to_string(),
            "pаssword=secret123".to_string(),
            "pass\u{200B}word\u{200C}=\u{200D}secret123".to_string(),
            "\u{202E}password=secret123".to_string(),
            "user@ехample.com user＠example.com".to_string(),
            "A".repeat(10_000),
            format!("{}!", "a".repeat(100)),
            format!("Email: {}@{}.com!", "a".repeat(50), "b".repeat(50)),
            format!("password=={}!", "a".repeat(50)),
            format!("Email: a@{}com", "b.".repeat(100)),
            "p%61ssword=secret123 \\u0070assword=x \\x70assword=y".to_string(),
        ];
        for input in &inputs {
            let _ = sanitize(input);
        }
        assert!(sanitize("Sensitive data here\rPublic message only").contains("Public"));
    }

    #[test]
    fn many_patterns_are_all_redacted() {
        let many: String = (0..10_000)
            .map(|i| format!("password{i}=value{i} "))
            .collect();
        let _ = sanitize(&many);
        let alternating = "password=x token=y api_key=z secret=w ".repeat(5000);
        assert!(!sanitize(&alternating).contains("password=x"));
    }

    #[test]
    fn many_data_entries_are_handled() {
        let sink = Arc::new(MemorySink::new());
        let logger = logger_with(&sink, 10, 50);
        let keys: Vec<String> = (1..=10_000).map(|i| format!("key{i}")).collect();
        let entries: Vec<(&str, Value)> = keys
            .iter()
            .map(|k| (k.as_str(), json!(format!("value-{k}"))))
            .collect();
        logger.info("Test message", &entries);
        assert_eq!(sink.records()[0].data.as_object().unwrap().len(), 10_001);
    }
}
