//! Runs that decline their work: counted as skips, never as successes.

use super::support::*;
use batter_core::{
    lifecycle::Readiness,
    periodic::{PeriodicFailure, PeriodicOutcome, PeriodicShutdown, register_skippable_in},
};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::sleep;

/// A skip does no work, so it cannot stand in for the first success that
/// first-success startup waits for: readiness comes only with a run that ran.
#[tokio::test(start_paused = true)]
async fn a_skip_never_acknowledges_first_success_startup() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_skippable_in(
        &mut supervisor,
        "storage.gated",
        first_success(SECOND, SECOND, SECOND * 10, PeriodicShutdown::StopAtDrain),
        move |_| {
            // Three skips, then work.
            let attempt = counted.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok::<_, PeriodicFailure<Infallible>>(if attempt < 3 {
                    PeriodicOutcome::Skipped
                } else {
                    PeriodicOutcome::Ran
                })
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    sleep(Duration::from_millis(2_500)).await;
    let summary = reader.snapshot();
    assert_eq!((summary.invocations, summary.skipped), (3, 3));
    assert_eq!(summary.succeeded, 0);
    assert!(!summary.acknowledged, "a skip acknowledged startup");
    assert_eq!(running.status().readiness(), Readiness::Starting);

    running.status().wait_ready().await.unwrap();
    let summary = reader.snapshot();
    assert!(summary.acknowledged);
    assert_eq!((summary.succeeded, summary.skipped), (1, 3));
    let success = running.shutdown_checked().await.unwrap();
    assert_eq!(success.report().periodic[0].summary.skipped, 3);
}
