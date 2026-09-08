use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
};
use std::{
    future::{Future, poll_fn},
    io::Write,
    pin::Pin,
    task::Poll,
    time::Duration,
};

pub const CASE_LIMIT: Duration = Duration::from_secs(5);

pub fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        second,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap()
}

pub fn supervisor(capacity: usize) -> Supervisor {
    let supervisor = Supervisor::with_process_capacity(budget(), capacity).unwrap();
    assert!(supervisor.handle().mark_ready());
    supervisor
}

#[derive(Clone, Copy)]
pub struct Case {
    pub seed: u64,
    pub workers: usize,
    pub index: usize,
    pub family: &'static str,
}

impl Case {
    pub fn event(&self, action: &'static str) {
        eprintln!(
            "SCHEDULE v1 workers={} seed={} case={} family={} action={action}",
            self.workers, self.seed, self.index, self.family
        );
        std::io::stderr().flush().unwrap();
    }

    pub async fn bounded<T>(&self, future: impl Future<Output = T>) -> T {
        self.event("begin");
        let result = tokio::time::timeout(CASE_LIMIT, future).await;
        assert!(
            result.is_ok(),
            "case deadline exceeded; replay preceding SCHEDULE record"
        );
        self.event("end");
        result.ok().unwrap()
    }
}

/// Test-local SplitMix64 stream; wrapping arithmetic is part of schedule version 1.
pub struct Choices(u64);

impl Choices {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }
}

pub async fn yields(count: u64) {
    for _ in 0..count % 8 {
        tokio::task::yield_now().await;
    }
}

pub async fn poll_pending<T>(mut future: Pin<&mut impl Future<Output = T>>) {
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}
