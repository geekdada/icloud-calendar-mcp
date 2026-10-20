//! Cancellation of in-progress operations (`notifications/cancelled`).
//!
//! Tracks active operations by request id and hands each a token the work can
//! poll. Completed ids are remembered for a while so a cancellation that
//! arrives after completion is recognized and ignored.

#![allow(deprecated)] // rmcp's LoggingLevel; see logging.rs.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{CancelledNotificationParam, LoggingLevel};
use serde_json::{Value, json};

use crate::logging::McpLogger;

/// How long a completed request id is remembered.
const COMPLETED_RETENTION: Duration = Duration::from_secs(5 * 60);

/// A cancelled operation, raised by [`CancellationToken::check_cancelled`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationCancelledError {
    pub request_id: String,
    pub message: String,
}

impl OperationCancelledError {
    pub fn new(request_id: &str) -> Self {
        Self {
            request_id: request_id.to_string(),
            message: format!("Operation cancelled: {request_id}"),
        }
    }
}

impl fmt::Display for OperationCancelledError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OperationCancelledError {}

/// Cancellation flag for one operation. Thread-safe.
#[derive(Debug)]
pub struct CancellationToken {
    pub request_id: String,
    cancelled: AtomicBool,
    created_at: Instant,
}

impl CancellationToken {
    fn new(request_id: &str) -> Self {
        Self {
            request_id: request_id.to_string(),
            cancelled: AtomicBool::new(false),
            created_at: Instant::now(),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Time since the operation started, in milliseconds.
    pub fn age_ms(&self) -> u64 {
        u64::try_from(self.created_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// `Err` once the operation has been cancelled, for `?` in processing loops.
    pub fn check_cancelled(&self) -> Result<(), OperationCancelledError> {
        if self.is_cancelled() {
            Err(OperationCancelledError::new(&self.request_id))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
pub struct CancellationManager {
    logger: Option<Arc<McpLogger>>,
    active: Mutex<HashMap<String, Arc<CancellationToken>>>,
    completed: Mutex<HashMap<String, Instant>>,
}

impl CancellationManager {
    pub fn new(logger: Option<Arc<McpLogger>>) -> Self {
        Self {
            logger,
            ..Self::default()
        }
    }

    /// Start tracking `request_id` and return its token.
    pub fn start_operation(&self, request_id: &str) -> Arc<CancellationToken> {
        let token = Arc::new(CancellationToken::new(request_id));
        self.active
            .lock()
            .expect("cancellation lock")
            .insert(request_id.to_string(), Arc::clone(&token));
        self.log(
            LoggingLevel::Debug,
            &format!("Started tracking operation: {request_id}"),
            request_id,
            None,
        );
        token
    }

    /// Cancel `request_id` if it is active; unknown and completed ids are ignored.
    pub fn handle_cancellation(&self, request_id: &str, reason: Option<&str>) {
        if self
            .completed
            .lock()
            .expect("cancellation lock")
            .contains_key(request_id)
        {
            self.log(
                LoggingLevel::Debug,
                &format!("Ignoring cancellation for completed operation: {request_id}"),
                request_id,
                Some(reason),
            );
            return;
        }
        let token = self
            .active
            .lock()
            .expect("cancellation lock")
            .get(request_id)
            .cloned();
        let Some(token) = token else {
            self.log(
                LoggingLevel::Debug,
                &format!("Ignoring cancellation for unknown operation: {request_id}"),
                request_id,
                Some(reason),
            );
            return;
        };
        token.cancel();
        self.log(
            LoggingLevel::Info,
            &format!("Operation cancelled: {request_id}"),
            request_id,
            Some(reason),
        );
    }

    /// Handle an MCP `notifications/cancelled` payload.
    pub fn handle_notification(&self, notification: &CancelledNotificationParam) {
        let Some(request_id) = &notification.request_id else {
            return;
        };
        self.handle_cancellation(&request_id.to_string(), notification.reason.as_deref());
    }

    /// Stop tracking `request_id` (success or failure) and remember it as completed.
    pub fn complete_operation(&self, request_id: &str) {
        self.active
            .lock()
            .expect("cancellation lock")
            .remove(request_id);
        {
            let mut completed = self.completed.lock().expect("cancellation lock");
            completed.insert(request_id.to_string(), Instant::now());
            completed.retain(|_, at| at.elapsed() <= COMPLETED_RETENTION);
        }
        self.log(
            LoggingLevel::Debug,
            &format!("Operation completed: {request_id}"),
            request_id,
            None,
        );
    }

    pub fn is_cancelled(&self, request_id: &str) -> bool {
        self.active
            .lock()
            .expect("cancellation lock")
            .get(request_id)
            .is_some_and(|t| t.is_cancelled())
    }

    pub fn token(&self, request_id: &str) -> Option<Arc<CancellationToken>> {
        self.active
            .lock()
            .expect("cancellation lock")
            .get(request_id)
            .cloned()
    }

    pub fn active_count(&self) -> usize {
        self.active.lock().expect("cancellation lock").len()
    }

    /// Forget every tracked operation.
    pub fn clear(&self) {
        self.active.lock().expect("cancellation lock").clear();
        self.completed.lock().expect("cancellation lock").clear();
    }

    /// Log with `requestId` (and `reason` when the event has one).
    fn log(
        &self,
        level: LoggingLevel,
        message: &str,
        request_id: &str,
        reason: Option<Option<&str>>,
    ) {
        let Some(logger) = &self.logger else {
            return;
        };
        let mut data: Vec<(&str, Value)> = vec![("requestId", json!(request_id))];
        if let Some(reason) = reason {
            data.push(("reason", json!(reason)));
        }
        logger.log(level, message, &data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::MemorySink;
    use rmcp::model::NumberOrString;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn start_operation_returns_token() {
        let manager = CancellationManager::default();
        let token = manager.start_operation("request-1");
        assert_eq!(token.request_id, "request-1");
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancellation_of_active_operation() {
        let manager = CancellationManager::default();
        manager.start_operation("request-1");
        assert!(!manager.is_cancelled("request-1"));
        manager.handle_cancellation("request-1", Some("User requested"));
        assert!(manager.is_cancelled("request-1"));
    }

    #[test]
    fn unknown_and_completed_operations_are_ignored() {
        let manager = CancellationManager::default();
        manager.handle_cancellation("unknown-request", Some("Test"));
        manager.handle_cancellation("", Some("Empty ID cancel"));
        manager.handle_cancellation("null", None);
        assert!(!manager.is_cancelled("unknown-request"));
        assert_eq!(manager.active_count(), 0);

        manager.start_operation("request-1");
        manager.complete_operation("request-1");
        manager.handle_cancellation("request-1", Some("Late cancellation"));
        assert!(!manager.is_cancelled("request-1"));
    }

    #[test]
    fn complete_removes_from_active() {
        let manager = CancellationManager::default();
        manager.start_operation("request-1");
        assert_eq!(manager.active_count(), 1);
        manager.complete_operation("request-1");
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn token_lookup() {
        let manager = CancellationManager::default();
        manager.start_operation("request-1");
        assert_eq!(manager.token("request-1").unwrap().request_id, "request-1");
        assert!(manager.token("unknown").is_none());
    }

    #[test]
    fn check_cancelled_errors_after_cancellation() {
        let manager = CancellationManager::default();
        let token = manager.start_operation("check-test");
        assert!(token.check_cancelled().is_ok());
        manager.handle_cancellation("check-test", Some("Cancel"));
        let err = token.check_cancelled().unwrap_err();
        assert_eq!(err.request_id, "check-test");
        assert!(err.to_string().contains("check-test"));
    }

    #[test]
    fn token_age_increases() {
        let manager = CancellationManager::default();
        let token = manager.start_operation("age-test");
        let first = token.age_ms();
        thread::sleep(Duration::from_millis(15));
        let second = token.age_ms();
        assert!(second >= 10 && second - first >= 10);
    }

    #[test]
    fn clear_removes_everything_and_is_per_manager() {
        let a = CancellationManager::default();
        let b = CancellationManager::default();
        a.start_operation("op-1");
        a.start_operation("op-2");
        b.start_operation("op-3");
        a.clear();
        assert_eq!(a.active_count(), 0);
        assert_eq!(b.active_count(), 1);
    }

    #[test]
    fn operations_are_independent_and_managers_isolated() {
        let manager = CancellationManager::default();
        for id in ["request-1", "request-2", "request-3"] {
            manager.start_operation(id);
        }
        manager.handle_cancellation("request-2", Some("Cancel 2"));
        assert!(!manager.is_cancelled("request-1"));
        assert!(manager.is_cancelled("request-2"));
        assert!(!manager.is_cancelled("request-3"));

        let other = CancellationManager::default();
        other.start_operation("request-2");
        assert!(!other.is_cancelled("request-2"));
    }

    #[test]
    fn replayed_and_hostile_cancellations_are_harmless() {
        let manager = CancellationManager::default();
        let token = manager.start_operation("request-1");
        let long_reason = "A".repeat(10_000_000);
        let multiline = format!("{}multiline\ninjection", "\n".repeat(1000));
        let reasons = [
            "<script>alert('xss')</script>",
            "'; DROP TABLE operations; --",
            "\u{0}\u{1}\u{2}null bytes",
            long_reason.as_str(),
            multiline.as_str(),
            r#"{"admin":true,"bypass":"auth"}"#,
        ];
        for reason in reasons {
            manager.handle_cancellation("request-1", Some(reason));
        }
        assert!(token.is_cancelled());
        assert!(token.is_cancelled());
    }

    #[test]
    fn special_and_long_ids() {
        let manager = CancellationManager::default();
        let long = "a".repeat(10_000);
        let ids = [
            "request/with/slashes",
            "request:with:colons",
            "request with spaces",
            "request\twith\ttabs",
            "request\nwith\nnewlines",
            "550e8400-e29b-41d4-a716-446655440000",
            long.as_str(),
        ];
        for id in ids {
            manager.start_operation(id);
            manager.handle_cancellation(id, Some("Cancel"));
            assert!(manager.is_cancelled(id), "{id:?}");
            manager.complete_operation(id);
        }
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn churn_leaves_nothing_active() {
        let manager = CancellationManager::default();
        for i in 0..10_000 {
            let id = format!("rapid-{i}");
            manager.start_operation(&id);
            if i % 2 == 0 {
                manager.handle_cancellation(&id, Some("Cancel"));
            }
            manager.complete_operation(&id);
        }
        assert_eq!(manager.active_count(), 0);

        (0..10_000).for_each(|i| {
            manager.start_operation(&format!("request-{i}"));
        });
        assert_eq!(manager.active_count(), 10_000);
        manager.clear();
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn races_do_not_panic() {
        let manager = Arc::new(CancellationManager::default());
        for i in 0..200 {
            let id = format!("race-{i}");
            manager.start_operation(&id);
            let barrier = Arc::new(Barrier::new(2));
            let spawn = |complete: bool| {
                let (manager, barrier, id) =
                    (Arc::clone(&manager), Arc::clone(&barrier), id.clone());
                thread::spawn(move || {
                    barrier.wait();
                    if complete {
                        manager.complete_operation(&id);
                    } else {
                        manager.handle_cancellation(&id, Some("Cancel"));
                    }
                })
            };
            let (a, b) = (spawn(false), spawn(true));
            a.join().unwrap();
            b.join().unwrap();
        }
        assert_eq!(manager.active_count(), 0);

        manager.start_operation("multi-cancel");
        let handles: Vec<_> = (0..100)
            .map(|i| {
                let manager = Arc::clone(&manager);
                thread::spawn(move || {
                    manager.handle_cancellation("multi-cancel", Some(&format!("Reason {i}")))
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert!(manager.is_cancelled("multi-cancel"));
    }

    #[test]
    fn concurrent_creation_from_many_threads() {
        let manager = Arc::new(CancellationManager::default());
        let handles: Vec<_> = (0..20)
            .map(|t| {
                let manager = Arc::clone(&manager);
                thread::spawn(move || {
                    for i in 0..100 {
                        let id = format!("thread-{t}-op-{i}");
                        manager.start_operation(&id);
                        manager.complete_operation(&id);
                    }
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn notification_and_logging() {
        let sink = Arc::new(MemorySink::new());
        let logger = Arc::new(McpLogger::with_limits(sink.clone(), "test", 10, 50, 1000));
        let manager = CancellationManager::new(Some(logger));
        manager.start_operation("42");
        manager.handle_notification(&CancelledNotificationParam::new(
            Some(NumberOrString::Number(42)),
            Some("user".into()),
        ));
        assert!(manager.is_cancelled("42"));
        manager.handle_notification(&CancelledNotificationParam::new(None, None));
        let info: Vec<_> = sink
            .records()
            .into_iter()
            .filter(|r| r.data["message"] == json!("Operation cancelled: 42"))
            .collect();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].data["reason"], json!("user"));
    }
}
