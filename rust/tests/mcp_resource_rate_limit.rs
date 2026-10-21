//! Rate limiting of the calendar resource. Port of `ResourceRateLimitTest.kt`,
//! plus the same limit observed through the server's resource read.

mod support;

use std::sync::Arc;

use icloud_calendar_mcp::logging::{McpLogger, NullSink};
use icloud_calendar_mcp::ratelimit::RateLimiter;
use icloud_calendar_mcp::server::{CALENDARS_URI, CalendarServer};
use rmcp::model::ErrorCode;
use support::*;

#[test]
fn allows_requests_within_limit() {
    let limiter = RateLimiter::new(5, 2, 60_000);
    for i in 0..5 {
        assert!(limiter.try_acquire_read(), "read {i} should be allowed");
    }
}

#[test]
fn blocks_requests_exceeding_limit() {
    let limiter = RateLimiter::new(3, 2, 60_000);
    for _ in 0..3 {
        limiter.try_acquire_read();
    }
    assert!(!limiter.try_acquire_read(), "4th read should be blocked");
}

#[test]
fn returns_retry_info_when_blocked() {
    let limiter = RateLimiter::new(1, 1, 60_000);
    limiter.try_acquire_read();
    let result = limiter.acquire_read_with_status();
    assert!(!result.allowed);
    assert!(result.retry_after_ms > 0);
    assert_eq!(result.remaining, 0);
}

#[tokio::test]
async fn resource_reads_share_the_read_budget_with_tools() {
    let mock = MockCalDavClient::with_calendars(vec![calendar("home", "Home")]);
    let server = CalendarServer::with_parts(
        Some(service(&mock)),
        Arc::new(RateLimiter::new(2, 1, 60_000)),
        Arc::new(McpLogger::new(Arc::new(NullSink))),
    );
    server.read_resource_uri(CALENDARS_URI).await.unwrap();
    let tool = server.call_tool_by_name("list_calendars", None).await.unwrap();
    assert_ne!(tool.is_error, Some(true));
    let error = server.read_resource_uri(CALENDARS_URI).await.unwrap_err();
    assert_eq!(error.code, ErrorCode(-32000));
    let blocked_tool = server.call_tool_by_name("list_calendars", None).await.unwrap();
    assert!(result_text(&blocked_tool).contains("RATE_LIMITED"));
}
