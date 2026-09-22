use std::time::{Duration, Instant};

/// Supplies monotonic elapsed time to a [`MemoryStore`](crate::MemoryStore).
///
/// A clock implementation must never intentionally move backwards. The store
/// nevertheless clamps values to the greatest time observed by each touched
/// shard, which also makes deterministic test clocks straightforward.
pub trait Clock: Send + Sync + 'static {
    /// Returns monotonic time elapsed from an arbitrary origin.
    fn now(&self) -> Duration;
}

/// A monotonic clock backed by [`Instant`].
#[derive(Clone, Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// Creates a clock whose origin is now.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}
