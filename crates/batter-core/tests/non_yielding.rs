//! OS-process containment for futures whose poll never returns to Tokio.

#[path = "non_yielding/capture.rs"]
mod capture;
#[path = "non_yielding/evidence.rs"]
mod evidence;
#[path = "non_yielding/fixture.rs"]
mod fixture;
#[path = "non_yielding/launch.rs"]
mod launch;
#[path = "non_yielding/timing.rs"]
mod timing;
#[path = "non_yielding/watchdog.rs"]
mod watchdog;

use watchdog::{ChildRun, ExpectedExit};

const BLOCKED_RUNTIME_EVIDENCE: &[&str] = &[
    "timer-armed",
    "task-entered",
    "drain-requested",
    "blocked-observation-elapsed",
];

const NON_YIELDING_EVIDENCE: &[&str] = &[
    "task-entered",
    "drain-requested",
    "report-unjoined-cleanup-skipped",
    "runtime-drop-started",
];

#[test]
fn cooperative_child_joins_and_cleans_up_before_normal_exit() {
    ChildRun::run("cooperative")
        .validate(
            ExpectedExit::Success,
            &[
                "task-entered",
                "drain-requested",
                "cancellation-observed",
                "task-dropped",
                "cleanup-invoked",
                "report-clean",
                "runtime-drop-started",
                "runtime-dropped",
            ],
            &[],
        )
        .unwrap();
}

#[test]
fn non_yielding_child_reports_unjoined_work_and_cannot_drop_runtime() {
    ChildRun::run("non-yielding")
        .validate(
            ExpectedExit::WatchdogKill,
            NON_YIELDING_EVIDENCE,
            &["task-dropped", "cleanup-invoked", "runtime-dropped"],
        )
        .unwrap();
}

#[test]
fn blocked_current_thread_cannot_drive_shutdown_or_its_timeout() {
    ChildRun::run("blocked-current-thread")
        .validate(
            ExpectedExit::WatchdogKill,
            BLOCKED_RUNTIME_EVIDENCE,
            &[
                "timer-completed",
                "report-unjoined-cleanup-skipped",
                "task-dropped",
                "cleanup-invoked",
                "runtime-drop-started",
                "runtime-dropped",
            ],
        )
        .unwrap();
}

#[test]
fn watchdog_rejects_a_stuck_child_without_shutdown_evidence() {
    let run = ChildRun::run("stuck-before-runtime");
    assert!(run.kill_requested(), "{run:?}");
    let failure = run
        .validate(ExpectedExit::WatchdogKill, NON_YIELDING_EVIDENCE, &[])
        .unwrap_err();
    assert!(failure.contains("missing event task-entered"), "{failure}");
    assert!(failure.contains("fixture-entered"), "{failure}");
}

#[test]
fn watchdog_preserves_child_failure_diagnostics() {
    let run = ChildRun::run("fixture-failure");
    assert!(!run.kill_requested(), "{run:?}");
    assert!(!run.status.success(), "{run:?}");
    let failure = run.validate(ExpectedExit::Success, &[], &[]).unwrap_err();
    assert!(failure.contains("deliberate fixture failure"), "{failure}");
}

#[test]
fn watchdog_rejects_child_panic_even_when_expected_stall_milestones_exist() {
    let run = ChildRun::run("blocked-request-failure");
    assert!(run.kill_requested(), "{run:?}");
    let failure = run
        .validate(ExpectedExit::WatchdogKill, BLOCKED_RUNTIME_EVIDENCE, &[])
        .unwrap_err();
    assert!(failure.contains("child panicked"), "{failure}");
    assert!(failure.contains("deliberate request failure"), "{failure}");
}

// Launch is a private stdin protocol tied to this process's actual PID.
#[test]
fn child_fixture() {
    if let Some(scenario) = launch::scenario() {
        fixture::run(&scenario);
    }
}
