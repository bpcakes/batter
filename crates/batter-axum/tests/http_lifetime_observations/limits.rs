use batter::{cleanup::CleanupBudget, lifecycle::ShutdownBudget};
use std::time::Duration;

pub const DISCONNECT: Duration = Duration::from_secs(1);
pub use crate::http_process::REPORT_DELAY;

pub fn shutdown(case: &str) -> ShutdownBudget {
    if matches!(
        case,
        "delayed_shutdown_report" | "delayed_missing_reconciliation_event"
    ) {
        return crate::http_process::delayed_report_budget();
    }
    let force = matches!(
        case,
        "forced_handler_cancellation" | "blocked_body_wrapper_abort"
    );
    let budget = ShutdownBudget::new(
        Duration::from_millis(if force { 100 } else { 2000 }),
        Duration::from_millis(if case == "blocked_body_wrapper_abort" {
            100
        } else {
            300
        }),
        Duration::from_millis(300),
        CleanupBudget::new(
            Duration::from_millis(500),
            Duration::from_millis(500),
            Duration::from_millis(100),
        )
        .unwrap(),
    )
    .unwrap();
    // Include cleanup and abort observation through the actual library allowance.
    assert!(budget.total_allowance() < crate::http_process::TEARDOWN);
    budget
}
