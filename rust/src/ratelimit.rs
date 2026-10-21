//! Thread-safe rate limiter with separate read and write budgets.
//!
//! A fixed window: both counters reset together once the window has elapsed.
//! The MCP server uses 60 reads and 20 writes per minute.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

/// Outcome of one acquire attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitResult {
    pub allowed: bool,
    pub remaining: u32,
    /// Time until the window resets; 0 when the request was allowed.
    pub retry_after_ms: u64,
}

/// Snapshot of the limiter's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitStatus {
    pub read_count: u32,
    pub write_count: u32,
    pub read_remaining: u32,
    pub write_remaining: u32,
    pub window_reset_ms: u64,
}

pub struct RateLimiter {
    read_limit: u32,
    write_limit: u32,
    window_ms: u64,
    read_count: AtomicU32,
    write_count: AtomicU32,
    /// Window start, in millis since `epoch`.
    window_start: AtomicU64,
    epoch: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(60, 20, 60_000)
    }
}

impl RateLimiter {
    pub fn new(read_limit: u32, write_limit: u32, window_ms: u64) -> Self {
        Self {
            read_limit,
            write_limit,
            window_ms,
            read_count: AtomicU32::new(0),
            write_count: AtomicU32::new(0),
            window_start: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }

    pub fn try_acquire_read(&self) -> bool {
        self.check_and_reset_window();
        acquire(&self.read_count, self.read_limit)
    }

    pub fn try_acquire_write(&self) -> bool {
        self.check_and_reset_window();
        acquire(&self.write_count, self.write_limit)
    }

    pub fn acquire_read_with_status(&self) -> RateLimitResult {
        let allowed = self.try_acquire_read();
        self.result(allowed, &self.read_count, self.read_limit)
    }

    pub fn acquire_write_with_status(&self) -> RateLimitResult {
        let allowed = self.try_acquire_write();
        self.result(allowed, &self.write_count, self.write_limit)
    }

    pub fn status(&self) -> RateLimitStatus {
        self.check_and_reset_window();
        let (reads, writes) = (self.read_count.load(Ordering::SeqCst), self.write_count.load(Ordering::SeqCst));
        RateLimitStatus {
            read_count: reads,
            write_count: writes,
            read_remaining: self.read_limit.saturating_sub(reads),
            write_remaining: self.write_limit.saturating_sub(writes),
            window_reset_ms: self.time_left_in_window(),
        }
    }

    /// Clear both counters and start a new window now.
    pub fn reset(&self) {
        self.read_count.store(0, Ordering::SeqCst);
        self.write_count.store(0, Ordering::SeqCst);
        self.window_start.store(self.now_ms(), Ordering::SeqCst);
    }

    fn result(&self, allowed: bool, counter: &AtomicU32, limit: u32) -> RateLimitResult {
        RateLimitResult {
            allowed,
            remaining: limit.saturating_sub(counter.load(Ordering::SeqCst)),
            retry_after_ms: if allowed { 0 } else { self.time_left_in_window() },
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn time_left_in_window(&self) -> u64 {
        let elapsed = self.now_ms().saturating_sub(self.window_start.load(Ordering::SeqCst));
        self.window_ms.saturating_sub(elapsed)
    }

    fn check_and_reset_window(&self) {
        let now = self.now_ms();
        let start = self.window_start.load(Ordering::SeqCst);
        // Only the thread that wins the swap resets, so a burst at the boundary
        // cannot reset the counters twice.
        if now.saturating_sub(start) >= self.window_ms
            && self.window_start.compare_exchange(start, now, Ordering::SeqCst, Ordering::SeqCst).is_ok()
        {
            self.read_count.store(0, Ordering::SeqCst);
            self.write_count.store(0, Ordering::SeqCst);
        }
    }
}

/// Take one token if any remain. A compare-and-swap loop, so concurrent callers
/// can never push the count past `limit`.
///
/// Written out by hand: `fetch_update` is deprecated on current stable, and its
/// replacement `try_update` is newer than the crate's `rust-version`.
fn acquire(counter: &AtomicU32, limit: u32) -> bool {
    let mut count = counter.load(Ordering::SeqCst);
    while count < limit {
        match counter.compare_exchange_weak(count, count + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(actual) => count = actual,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn limiter() -> RateLimiter {
        RateLimiter::new(60, 20, 60_000)
    }

    #[test]
    fn reads_allowed_up_to_limit_then_denied() {
        let limiter = limiter();
        for _ in 0..60 {
            assert!(limiter.try_acquire_read());
        }
        assert!(!limiter.try_acquire_read());
    }

    #[test]
    fn writes_allowed_up_to_limit_then_denied() {
        let limiter = limiter();
        for _ in 0..20 {
            assert!(limiter.try_acquire_write());
        }
        assert!(!limiter.try_acquire_write());
    }

    #[test]
    fn status_results_report_remaining() {
        let limiter = limiter();
        let read = limiter.acquire_read_with_status();
        assert!(read.allowed);
        assert_eq!(read.remaining, 59);
        let write = limiter.acquire_write_with_status();
        assert!(write.allowed);
        assert_eq!(write.remaining, 19);
    }

    #[test]
    fn reset_clears_counts() {
        let limiter = limiter();
        (0..60).for_each(|_| {
            limiter.try_acquire_read();
        });
        (0..20).for_each(|_| {
            limiter.try_acquire_write();
        });
        assert!(!limiter.try_acquire_read());
        assert!(!limiter.try_acquire_write());
        limiter.reset();
        assert!(limiter.try_acquire_read());
        assert!(limiter.try_acquire_write());
    }

    #[test]
    fn window_expires_after_time_passes() {
        let limiter = RateLimiter::new(5, 2, 100);
        for _ in 0..5 {
            assert!(limiter.try_acquire_read());
        }
        assert!(!limiter.try_acquire_read());
        std::thread::sleep(Duration::from_millis(150));
        assert!(limiter.try_acquire_read());
    }

    #[test]
    fn status_shows_current_counts() {
        let limiter = limiter();
        (0..10).for_each(|_| {
            limiter.try_acquire_read();
        });
        (0..5).for_each(|_| {
            limiter.try_acquire_write();
        });
        let status = limiter.status();
        assert_eq!((status.read_count, status.write_count), (10, 5));
        assert_eq!((status.read_remaining, status.write_remaining), (50, 15));
    }

    #[test]
    fn denied_result_carries_retry_after() {
        let limiter = limiter();
        (0..60).for_each(|_| {
            limiter.try_acquire_read();
        });
        let result = limiter.acquire_read_with_status();
        assert!(!result.allowed);
        assert!(result.retry_after_ms > 0 && result.retry_after_ms <= 60_000);
    }

    fn hammer(limiter: &Arc<RateLimiter>, threads: usize, write: bool) {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let limiter = Arc::clone(limiter);
                std::thread::spawn(move || if write { limiter.try_acquire_write() } else { limiter.try_acquire_read() })
            })
            .collect();
        handles.into_iter().for_each(|h| {
            h.join().unwrap();
        });
    }

    #[test]
    fn concurrent_reads_respect_limit() {
        let limiter = Arc::new(limiter());
        hammer(&limiter, 100, false);
        assert_eq!(limiter.status().read_count, 60);
    }

    #[test]
    fn concurrent_writes_respect_limit() {
        let limiter = Arc::new(limiter());
        hammer(&limiter, 50, true);
        assert_eq!(limiter.status().write_count, 20);
    }

    #[test]
    fn zero_limit_always_denies() {
        let limiter = RateLimiter::new(0, 0, 60_000);
        assert!(!limiter.try_acquire_read());
        assert!(!limiter.try_acquire_write());
    }

    #[test]
    fn high_limit_works() {
        let limiter = RateLimiter::new(10_000, 1_000, 60_000);
        assert!((0..1000).all(|_| limiter.try_acquire_read()));
        assert!((0..500).all(|_| limiter.try_acquire_write()));
    }

    #[test]
    fn read_and_write_limits_are_independent() {
        let limiter = limiter();
        (0..60).for_each(|_| {
            limiter.try_acquire_read();
        });
        assert!(!limiter.try_acquire_read());
        assert!(limiter.try_acquire_write());
    }
}
