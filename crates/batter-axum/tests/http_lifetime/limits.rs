use batter::{cleanup::CleanupBudget, lifecycle::ShutdownBudget};
use std::time::Duration;

pub const EXERCISE: Duration = Duration::from_secs(2);
pub const STARTUP: Duration = Duration::from_secs(1);
pub const TEARDOWN: Duration = Duration::from_millis(3500);
pub const MISSING_EVENT: Duration = Duration::from_millis(500);
pub const PARENT: Duration = Duration::from_secs(8);
const DRAIN: Duration = Duration::from_secs(2);
const FORCE_DRAIN: Duration = Duration::from_millis(100);
const CANCEL: Duration = Duration::from_millis(300);
const ABORT_REAP: Duration = Duration::from_millis(300);
const CLEANUP: Duration = Duration::from_millis(500);
const CLEANUP_REAP: Duration = Duration::from_millis(100);

pub fn validate() {
    // Reserve startup/unwind margin separately from the two diagnostic phases.
    assert!(STARTUP + EXERCISE + TEARDOWN + Duration::from_secs(1) < PARENT);
    assert!(shutdown_budget(false).total_allowance() < TEARDOWN);
    assert!(shutdown_budget(true).total_allowance() < TEARDOWN);
    assert!(FORCE_DRAIN < DRAIN);
    assert!(MISSING_EVENT < EXERCISE);
    assert!(PARENT < super::launch::EMERGENCY_LIMIT);
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
