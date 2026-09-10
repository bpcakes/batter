use batter::{cleanup::CleanupBudget, lifecycle::ShutdownBudget};
use std::time::Duration;

pub use crate::http_process::{EXERCISE, STARTUP, TEARDOWN};
pub const MISSING_EVENT: Duration = Duration::from_millis(500);
const DRAIN: Duration = Duration::from_secs(2);
const FORCE_DRAIN: Duration = Duration::from_millis(100);
const CANCEL: Duration = Duration::from_millis(300);
const ABORT_REAP: Duration = Duration::from_millis(300);
const CLEANUP: Duration = Duration::from_millis(500);
const CLEANUP_REAP: Duration = Duration::from_millis(100);

pub fn validate() {
    assert!(shutdown_budget(false).total_allowance() < TEARDOWN);
    assert!(shutdown_budget(true).total_allowance() < TEARDOWN);
    assert!(FORCE_DRAIN < DRAIN);
    assert!(MISSING_EVENT < EXERCISE);
}

pub fn shutdown_budget(force: bool) -> ShutdownBudget {
    ShutdownBudget::new(
        if force { FORCE_DRAIN } else { DRAIN },
        CANCEL,
        ABORT_REAP,
        CleanupBudget::new(CLEANUP, CLEANUP, CLEANUP_REAP).unwrap(),
    )
    .unwrap()
}
