//! Clock abstraction for the service layer.
//!
//! Production uses [`SystemClock`] (wall-clock `Utc::now()`). Tests inject
//! [`FixedClock`] to make timestamp-dependent behaviour deterministic without
//! sleeping — e.g. hook-event timestamps are persisted at one-second
//! resolution, so a test that needs two events to land in distinct seconds
//! advances the clock instead of `tokio::time::sleep`. See `docs/conventions.md`
//! ("No `tokio::time::sleep` in tests").

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

/// Source of "now" for the service layer.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> DateTime<Utc>;
}

/// Real wall-clock time. The production default.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A manually-advanceable clock for tests. Starts at a fixed instant and only
/// moves when [`advance`](Self::advance) is called — never via wall-clock, so
/// timestamp ordering is fully deterministic.
#[derive(Clone)]
pub struct FixedClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl FixedClock {
    /// Create a clock fixed at `start`.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(start)),
        }
    }

    /// Advance the clock by `delta`.
    pub fn advance(&self, delta: Duration) {
        let mut guard = self.now.lock().unwrap_or_else(|e| e.into_inner());
        *guard += delta;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}
