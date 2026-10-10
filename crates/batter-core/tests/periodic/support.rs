//! Shared fixtures for the periodic-component contracts.

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ManagedSettlement, ShutdownBudget, Supervisor},
    periodic::{PeriodicPolicy, PeriodicRun, PeriodicShutdown, PeriodicStartup},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

pub const SECOND: Duration = Duration::from_secs(1);

/// A generic retained application failure with an inspectable identity.
#[derive(Debug, thiserror::Error)]
#[error("maintenance attempt failed")]
pub struct Attempt(pub u32);

/// Every fixture run uses one concrete application error type, exactly as a
/// consumer would, so `?` propagation stays unambiguous.
pub type Run = PeriodicRun<Attempt>;

pub fn succeeded() -> Run {
    Ok(())
}

pub fn failed(attempt: u32) -> Run {
    Err(Attempt(attempt))?
}

pub fn escalated(attempt: u32) -> Run {
    Err(batter_core::periodic::PeriodicFailure::Fatal(Attempt(
        attempt,
    )))
}

pub fn supervisor() -> Supervisor {
    supervisor_with(SECOND)
}

/// A supervisor whose drain allowance is explicitly selected, so support work
/// admitted during drain can be bounded by a known graceful interval.
pub fn supervisor_with(drain: Duration) -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            drain,
            SECOND,
            SECOND,
            CleanupBudget::new(SECOND, SECOND, SECOND).unwrap(),
        )
        .unwrap(),
    )
}

pub fn immediate(
    interval: Duration,
    budget: Duration,
    shutdown: PeriodicShutdown,
) -> PeriodicPolicy {
    PeriodicPolicy::new(interval, budget, PeriodicStartup::immediate(), shutdown).unwrap()
}

pub fn first_success(
    interval: Duration,
    budget: Duration,
    allowance: Duration,
    shutdown: PeriodicShutdown,
) -> PeriodicPolicy {
    PeriodicPolicy::new(
        interval,
        budget,
        PeriodicStartup::after_first_success(allowance).unwrap(),
        shutdown,
    )
    .unwrap()
}

/// Records every invocation's start offset and proves runs never overlap.
#[derive(Clone, Default)]
pub struct Schedule {
    inner: Arc<ScheduleState>,
}

#[derive(Default)]
struct ScheduleState {
    starts: Mutex<Vec<Duration>>,
    active: AtomicUsize,
    overlapped: AtomicUsize,
}

/// Releases its active slot even when its run is destroyed mid-flight.
pub struct Active {
    inner: Arc<ScheduleState>,
}

impl Drop for Active {
    fn drop(&mut self) {
        self.inner.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Schedule {
    pub fn started(&self, origin: Instant) -> Active {
        self.inner
            .starts
            .lock()
            .unwrap()
            .push(Instant::now().duration_since(origin));
        if self.inner.active.fetch_add(1, Ordering::SeqCst) != 0 {
            self.inner.overlapped.fetch_add(1, Ordering::SeqCst);
        }
        Active {
            inner: self.inner.clone(),
        }
    }

    pub fn starts(&self) -> Vec<Duration> {
        self.inner.starts.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.inner.starts.lock().unwrap().len()
    }

    pub fn overlapped(&self) -> usize {
        self.inner.overlapped.load(Ordering::SeqCst)
    }
}

/// A native settlement report whose cooperation is selected by the test.
pub struct NativeReport {
    pub joined: bool,
}

impl ManagedSettlement for NativeReport {
    fn is_success(&self) -> bool {
        self.joined
    }
    fn allows_dependency_cleanup(&self) -> bool {
        self.joined
    }
}

/// Pushes one label when the value it is captured by is destroyed.
pub struct Destroyed(pub Order, pub &'static str);

impl Drop for Destroyed {
    fn drop(&mut self) {
        self.0.push(self.1);
    }
}

/// Records the order in which lifecycle phases were observed.
#[derive(Clone, Default)]
pub struct Order(Arc<Mutex<Vec<&'static str>>>);

impl Order {
    pub fn push(&self, label: &'static str) {
        self.0.lock().unwrap().push(label);
    }

    pub fn observed(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}
