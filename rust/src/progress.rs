//! Progress reporting for long-running operations.
//!
//! Tracks progress internally, forwards each update to an optional callback,
//! and, when the request carried a `progressToken`, mirrors updates to the MCP
//! log so a client can follow along.

#![allow(deprecated)] // rmcp's LoggingLevel; see logging.rs.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rmcp::model::LoggingLevel;
use serde_json::{Map, Value, json};

use crate::logging::McpLogger;

/// Snapshot of a reporter's latest update.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressState {
    pub progress: f64,
    pub message: Option<String>,
    pub timestamp: SystemTime,
}

impl ProgressState {
    fn new(progress: f64, message: Option<String>) -> Self {
        Self {
            progress,
            message,
            timestamp: SystemTime::now(),
        }
    }
}

/// Receives `(progress, total, message)` on every update.
pub type ProgressCallback = Arc<dyn Fn(f64, Option<f64>, Option<&str>) + Send + Sync>;

pub struct ProgressReporter {
    progress_token: Option<String>,
    total: Option<f64>,
    logger: Option<Arc<McpLogger>>,
    operation_name: String,
    state: Mutex<ProgressState>,
    callback: Mutex<Option<ProgressCallback>>,
}

impl ProgressReporter {
    pub fn new(
        progress_token: Option<String>,
        total: Option<f64>,
        logger: Option<Arc<McpLogger>>,
        operation_name: &str,
    ) -> Self {
        Self {
            progress_token,
            total,
            logger,
            operation_name: operation_name.to_string(),
            state: Mutex::new(ProgressState::new(0.0, None)),
            callback: Mutex::new(None),
        }
    }

    /// A reporter that tracks state but never logs.
    pub fn noop() -> Self {
        Self::new(None, None, None, "operation")
    }

    /// A reporter for the `progressToken` in a request's `_meta`, if present.
    /// A non-string token is used as its JSON text; null means no token.
    pub fn from_meta(
        meta: Option<&Map<String, Value>>,
        logger: Option<Arc<McpLogger>>,
        operation_name: &str,
        total: Option<f64>,
    ) -> Self {
        let token = meta
            .and_then(|m| m.get("progressToken"))
            .and_then(|token| match token {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            });
        Self::new(token, total, logger, operation_name)
    }

    /// Record an update. Values are stored as given; validating them is the
    /// caller's job.
    pub fn report(&self, progress: f64, message: Option<&str>) {
        *self.state.lock().expect("progress lock") =
            ProgressState::new(progress, message.map(str::to_string));
        self.invoke_callback(progress, message);

        let (Some(logger), Some(token)) = (&self.logger, &self.progress_token) else {
            return;
        };
        // Float-to-int `as` saturates and maps NaN to 0, matching Kotlin's toInt().
        let percent = self
            .total
            .map(|total| format!(" ({}%)", (progress / total * 100.0) as i64))
            .unwrap_or_default();
        logger.log(
            LoggingLevel::Debug,
            &format!(
                "Progress{percent}: {}",
                message.unwrap_or(&self.operation_name)
            ),
            &[
                ("progressToken", json!(token)),
                ("progress", json!(progress)),
                ("total", json!(self.total)),
                ("operation", json!(self.operation_name)),
            ],
        );
    }

    /// Report completion at `total` (or the last progress when there is no total).
    pub fn complete(&self, message: Option<&str>) {
        let final_progress = self.total.unwrap_or_else(|| self.state().progress);
        self.report(final_progress, Some(message.unwrap_or("Complete")));
        if let (Some(logger), Some(token)) = (&self.logger, &self.progress_token) {
            logger.log(
                LoggingLevel::Info,
                &format!(
                    "Operation complete: {}",
                    message.unwrap_or(&self.operation_name)
                ),
                &[
                    ("progressToken", json!(token)),
                    ("operation", json!(self.operation_name)),
                ],
            );
        }
    }

    /// Record an error that stopped progress, keeping the last progress value.
    pub fn error(&self, error: &str) {
        let last_progress = {
            let mut state = self.state.lock().expect("progress lock");
            *state = ProgressState::new(state.progress, Some(error.to_string()));
            state.progress
        };
        if let (Some(logger), Some(token)) = (&self.logger, &self.progress_token) {
            logger.log(
                LoggingLevel::Error,
                &format!("Operation failed: {error}"),
                &[
                    ("progressToken", json!(token)),
                    ("operation", json!(self.operation_name)),
                    ("lastProgress", json!(last_progress)),
                ],
            );
        }
    }

    pub fn state(&self) -> ProgressState {
        self.state.lock().expect("progress lock").clone()
    }

    /// Register the callback invoked on every update (replacing any previous one).
    pub fn on_progress(
        &self,
        callback: impl Fn(f64, Option<f64>, Option<&str>) + Send + Sync + 'static,
    ) {
        *self.callback.lock().expect("progress lock") = Some(Arc::new(callback));
    }

    pub fn has_token(&self) -> bool {
        self.progress_token.is_some()
    }

    pub fn token(&self) -> Option<&str> {
        self.progress_token.as_deref()
    }

    /// Run the callback outside the lock; a panicking callback is contained so
    /// it cannot break the reporter or the operation it tracks.
    fn invoke_callback(&self, progress: f64, message: Option<&str>) {
        let callback = self.callback.lock().expect("progress lock").clone();
        if let Some(callback) = callback {
            let _ = catch_unwind(AssertUnwindSafe(|| callback(progress, self.total, message)));
        }
    }
}

impl McpLogger {
    /// A reporter that mirrors its updates to this logger.
    pub fn progress_reporter(
        self: &Arc<Self>,
        progress_token: Option<String>,
        total: Option<f64>,
        operation_name: &str,
    ) -> ProgressReporter {
        ProgressReporter::new(
            progress_token,
            total,
            Some(Arc::clone(self)),
            operation_name,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::MemorySink;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn reporter(token: &str, total: Option<f64>) -> ProgressReporter {
        ProgressReporter::new(Some(token.to_string()), total, None, "operation")
    }

    #[test]
    fn report_updates_state() {
        let r = reporter("test-token", Some(100.0));
        r.report(25.0, Some("Step 1"));
        assert_eq!(r.state().progress, 25.0);
        assert_eq!(r.state().message.as_deref(), Some("Step 1"));
        r.report(75.0, Some("Step 2"));
        assert_eq!(r.state().progress, 75.0);
        assert_eq!(r.state().message.as_deref(), Some("Step 2"));
    }

    #[test]
    fn complete_sets_total() {
        let r = reporter("test-token", Some(100.0));
        r.report(50.0, None);
        r.complete(Some("Done"));
        assert_eq!(r.state().progress, 100.0);
        assert_eq!(r.state().message.as_deref(), Some("Done"));
    }

    #[test]
    fn token_accessors() {
        assert!(reporter("token-123", None).has_token());
        assert_eq!(reporter("my-token", None).token(), Some("my-token"));
        let none = ProgressReporter::noop();
        assert!(!none.has_token());
        assert_eq!(none.token(), None);
        let empty = reporter("", None);
        assert!(empty.has_token());
        assert_eq!(empty.token(), Some(""));
        assert_eq!(reporter("   ", None).token(), Some("   "));
    }

    #[test]
    fn callback_receives_values() {
        let r = reporter("values-test", Some(200.0));
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        r.on_progress(move |p, t, m| *sink.lock().unwrap() = Some((p, t, m.map(str::to_string))));
        r.report(100.0, Some("Halfway"));
        assert_eq!(
            *seen.lock().unwrap(),
            Some((100.0, Some(200.0), Some("Halfway".to_string())))
        );
    }

    #[test]
    fn noop_reporter_never_fails() {
        let r = ProgressReporter::noop();
        r.report(50.0, Some("Test"));
        r.complete(Some("Done"));
        r.error("Error");
        assert!(!r.has_token());
    }

    #[test]
    fn from_meta_extracts_token() {
        let meta: Map<String, Value> =
            serde_json::from_value(json!({"progressToken": "extracted-token"})).unwrap();
        let r = ProgressReporter::from_meta(Some(&meta), None, "test-op", None);
        assert_eq!(r.token(), Some("extracted-token"));
        assert!(!ProgressReporter::from_meta(None, None, "test-op", None).has_token());

        let malicious: Map<String, Value> = serde_json::from_value(json!({
            "progressToken": "<script>alert('xss')</script>", "extra": "ignored", "__proto__": {"admin": true}
        }))
        .unwrap();
        assert_eq!(
            ProgressReporter::from_meta(Some(&malicious), None, "op", None).token(),
            Some("<script>alert('xss')</script>")
        );
        let number: Map<String, Value> =
            serde_json::from_value(json!({"progressToken": 12345})).unwrap();
        assert_eq!(
            ProgressReporter::from_meta(Some(&number), None, "op", None).token(),
            Some("12345")
        );
        let null: Map<String, Value> =
            serde_json::from_value(json!({"progressToken": null})).unwrap();
        assert!(!ProgressReporter::from_meta(Some(&null), None, "op", None).has_token());
    }

    #[test]
    fn logger_extension_and_logging() {
        let sink = Arc::new(MemorySink::new());
        let logger = Arc::new(McpLogger::with_limits(
            sink.clone(),
            "test-logger",
            10,
            50,
            1000,
        ));
        logger.set_level(LoggingLevel::Debug);
        let r = logger.progress_reporter(Some("ext-token".into()), Some(50.0), "extension-test");
        assert!(r.has_token());
        assert_eq!(r.token(), Some("ext-token"));
        r.report(25.0, Some("Working"));
        let record = &sink.records()[0];
        assert_eq!(record.data["message"], json!("Progress (50%): Working"));
        assert_eq!(record.data["progressToken"], json!("ext-token"));
        r.complete(None);
        r.error("boom");
        assert_eq!(sink.count(), 4);
    }

    #[test]
    fn messages_are_stored_verbatim() {
        let r = reporter("test", None);
        for payload in [
            "<script>alert('xss')</script>",
            "'; DROP TABLE progress; --",
            r#"{"admin": true, "bypass": "security"}"#,
            "Progress\u{0}\u{1}\u{7}\u{1B}[31mRED",
            "Line 1\nLine 2\r\nLine 3\rLine 4",
        ] {
            r.report(50.0, Some(payload));
            assert_eq!(r.state().message.as_deref(), Some(payload));
        }
        let long = "X".repeat(100_000);
        r.report(50.0, Some(&long));
        assert_eq!(r.state().message.as_deref(), Some(long.as_str()));
    }

    #[test]
    fn abnormal_values_are_stored() {
        let r = reporter("t", Some(100.0));
        r.report(-50.0, None);
        assert_eq!(r.state().progress, -50.0);
        r.report(150.0, None);
        assert_eq!(r.state().progress, 150.0);
        r.report(f64::NAN, None);
        assert!(r.state().progress.is_nan());
        r.report(f64::INFINITY, None);
        assert_eq!(r.state().progress, f64::INFINITY);
        r.report(f64::NEG_INFINITY, None);
        assert_eq!(r.state().progress, f64::NEG_INFINITY);
        r.complete(Some("Done"));
        assert_eq!(r.state().progress, 100.0);
    }

    #[test]
    fn error_keeps_progress_and_sets_message() {
        let r = reporter("error-test", None);
        r.report(50.0, Some("Working"));
        r.error("Something went wrong");
        assert_eq!(r.state().progress, 50.0);
        assert_eq!(r.state().message.as_deref(), Some("Something went wrong"));
    }

    #[test]
    fn panicking_callback_is_contained() {
        let r = reporter("exception-test", None);
        r.on_progress(|_, _, _| panic!("Callback error"));
        r.report(50.0, Some("Test"));
        assert_eq!(r.state().progress, 50.0);
    }

    #[test]
    fn rapid_updates_and_callbacks() {
        let r = reporter("rapid-test", Some(10_000.0));
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        r.on_progress(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        (0..10_000).for_each(|i| r.report(f64::from(i), Some("Update")));
        assert_eq!(r.state().progress, 9_999.0);
        assert_eq!(calls.load(Ordering::SeqCst), 10_000);
    }

    #[test]
    fn reporters_are_independent() {
        let a = reporter("same-token", None);
        let b = reporter("same-token", None);
        a.report(25.0, Some("Reporter 1"));
        b.report(75.0, Some("Reporter 2"));
        assert_eq!((a.state().progress, b.state().progress), (25.0, 75.0));
    }

    #[test]
    fn concurrent_updates_and_callbacks() {
        let r = Arc::new(reporter("concurrent", None));
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        r.on_progress(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let handles: Vec<_> = (0..10)
            .map(|t| {
                let r = Arc::clone(&r);
                std::thread::spawn(move || {
                    (0..100).for_each(|i| r.report(f64::from(t * 100 + i), Some("update")))
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1_000);
    }
}
