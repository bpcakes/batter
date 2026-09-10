//! Bounded process-contained exploration; seeds do not control Tokio scheduling.

#[path = "scheduling/admission.rs"]
mod admission;
#[path = "scheduling/closure_races.rs"]
mod closure_races;
#[path = "scheduling/escalation.rs"]
mod escalation;
#[path = "scheduling/failures.rs"]
mod failures;
#[path = "scheduling/families.rs"]
mod families;
#[path = "scheduling/launch.rs"]
mod launch;
#[path = "scheduling/ledger.rs"]
mod ledger;
#[path = "scheduling/operations.rs"]
mod operations;
#[path = "scheduling/ownership.rs"]
mod ownership;
#[path = "scheduling/profile.rs"]
mod profile;
#[path = "scheduling/readiness.rs"]
mod readiness;
#[path = "scheduling/support.rs"]
mod support;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "scheduling/transitions.rs"]
mod transitions;
#[path = "scheduling/workload.rs"]
mod workload;

use std::process::{Command, Output};

const FIXTURE_WATCHDOG_SECONDS: u64 = 3;
// Matches stress_scheduling.py's bounded reap/output-EOF allowance.
const REAP_ALLOWANCE_SECONDS: u64 = 5;
const PROCESS_STARTUP_SLACK_SECONDS: u64 = 4;

fn fixture_watchdog(schedule: &str) -> Option<std::time::Duration> {
    let observation = std::time::Duration::from_secs(FIXTURE_WATCHDOG_SECONDS);
    match schedule {
        "stuck" => Some(observation),
        "unjoined" | "unjoined-delayed-start" => {
            // This fixture must finish its entire bounded case before runtime
            // destruction becomes the hang that the external owner observes.
            Some(
                std::time::Duration::from_secs(PROCESS_STARTUP_SLACK_SECONDS)
                    + support::CASE_LIMIT
                    + observation,
            )
        }
        _ => None,
    }
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_watchdog_elapsed(elapsed: std::time::Duration, watchdog_seconds: u64, text: &str) {
    assert!(
        elapsed >= std::time::Duration::from_secs(watchdog_seconds),
        "premature watchdog kill: {text}"
    );
    assert!(
        elapsed
            < std::time::Duration::from_secs(
                watchdog_seconds + REAP_ALLOWANCE_SECONDS + PROCESS_STARTUP_SLACK_SECONDS,
            ),
        "late watchdog kill/reap after {elapsed:?}: {text}"
    );
}

fn run(workers: usize, schedule: Option<&str>) -> Output {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/stress_scheduling.py"
    );
    let mut command = Command::new("python3");
    command.args([script, "--binary"]);
    command.arg(std::env::current_exe().unwrap());
    command.args(["--workers", &workers.to_string()]);
    if let Some(schedule) = schedule {
        command.args(["--seed", "17", "--schedule", schedule]);
        if let Some(watchdog) = fixture_watchdog(schedule) {
            command.args(["--watchdog", &watchdog.as_secs_f64().to_string()]);
        }
    }
    command
        .output()
        .expect("scheduling tests require Python 3 on PATH")
}

#[test]
fn two_worker_scheduling_corpus() {
    let output = run(2, None);
    assert!(output.status.success(), "{}", diagnostics(&output));
}

#[test]
fn four_worker_scheduling_corpus() {
    let output = run(4, None);
    assert!(output.status.success(), "{}", diagnostics(&output));
}

fn escalation_case(mode: usize) -> support::Case {
    support::Case {
        seed: 17,
        workers: 1,
        index: mode,
        family: "controlled-escalation",
    }
}

#[tokio::test(start_paused = true)]
async fn cooperative_escalation_ignores_real_clock_stalls() {
    for mode in 0..2 {
        let case = escalation_case(mode);
        let report = case
            .bounded(escalation::outcomes(case, mode, async {
                // Model an OS pause after the signal. The current-thread
                // runtime's paused clock must not consume a phase allowance.
                std::thread::sleep(escalation::PHASE_ALLOWANCE * 4);
            }))
            .await;
        assert!(report.is_success());
        assert_eq!(report.completed_process_tasks, 2);
        assert_eq!(report.forced_cancellation, mode == 1);
        assert!(report.abort_requested.is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn cooperative_escalation_after_deadline_retains_abort_evidence() {
    for mode in 0..2 {
        let case = escalation_case(mode);
        let report = case
            .bounded(escalation::outcomes(case, mode, async {
                // Advancing Tokio time past both allowances is a legitimate
                // unfinished-task abort, even for otherwise cooperative work.
                tokio::time::sleep(escalation::PHASE_ALLOWANCE * 4).await;
            }))
            .await;
        assert!(!report.is_success());
        assert!(report.forced_cancellation);
        assert_eq!(report.abort_requested, ["affected-task"]);
        assert_eq!(report.completed_process_tasks, 1);
        assert_eq!(report.cleanup.skipped.len(), 1);
    }
}

#[test]
fn controlled_capacity_schedules_replay() {
    for schedule in ["capacity-before-drain", "capacity-after-drain"] {
        let output = run(2, Some(schedule));
        assert!(output.status.success(), "{}", diagnostics(&output));
    }
}

#[tokio::test(start_paused = true)]
async fn descendant_closure_before_deadline_requires_completion() {
    for failure in [false, true] {
        let case = support::Case {
            seed: 17,
            workers: 1,
            index: usize::from(failure),
            family: "controlled-closure",
        };
        let report = case
            .bounded(closure_races::descendant_closure(
                case,
                failure,
                0,
                std::future::ready(()),
            ))
            .await;
        assert_eq!(report.is_success(), !failure);
        assert!(report.abort_requested.is_empty());
        assert!(report.completed_process_tasks >= 1);
    }
}

#[tokio::test(start_paused = true)]
async fn descendant_closure_after_deadline_reconciles_termination() {
    for failure in [false, true] {
        let case = support::Case {
            seed: 17,
            workers: 1,
            index: usize::from(failure),
            family: "controlled-closure",
        };
        let report = case
            .bounded(closure_races::descendant_closure(case, failure, 0, async {
                tokio::time::sleep(std::time::Duration::from_millis(1250)).await;
            }))
            .await;
        assert!(!report.is_success());
        assert!(report.forced_cancellation);
        assert!(report.abort_requested.contains(&"active-ancestor"));
        assert!(report.unjoined.is_empty());
        assert_eq!(report.cleanup.skipped.len(), 1);
    }
}

#[test]
fn external_watchdog_rejects_a_runtime_that_cannot_poll_timers() {
    let started = std::time::Instant::now();
    let output = run(2, Some("stuck"));
    let elapsed = started.elapsed();
    let text = diagnostics(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("action=timer-armed"), "{text}");
    assert!(text.contains("action=runtime-blocked"), "{text}");
    assert!(text.contains("\"status\": -9"), "{text}");
    assert!(text.contains("\"watchdog\": true"), "{text}");
    assert!(!text.contains("PROFILE_OK"), "{text}");
    assert!(!text.contains("action=timer-finished"), "{text}");
    assert_watchdog_elapsed(elapsed, FIXTURE_WATCHDOG_SECONDS, &text);
}

#[test]
fn watchdog_rejects_success_without_a_profile_oracle() {
    let output = run(2, Some("early-exit"));
    let text = diagnostics(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("\"status\": 0"), "{text}");
    assert!(!text.contains("PROFILE_OK"), "{text}");
}

#[test]
fn unjoined_finite_work_keeps_its_receipt_pending_and_skips_cleanup() {
    let started = std::time::Instant::now();
    let output = run(2, Some("unjoined"));
    assert_unjoined_watchdog(&output, started.elapsed());
}

#[test]
fn delayed_unjoined_start_preserves_report_evidence() {
    let started = std::time::Instant::now();
    let output = run(2, Some("unjoined-delayed-start"));
    let elapsed = started.elapsed();
    let text = diagnostics(&output);
    assert!(text.contains("family=startup action=delay-end"), "{text}");
    assert_unjoined_watchdog(&output, elapsed);
}

fn assert_unjoined_watchdog(output: &Output, elapsed: std::time::Duration) {
    let text = diagnostics(output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("action=unjoined-report-and-pending-receipt-reconciled"),
        "{text}"
    );
    assert!(text.contains("\"status\": -9"), "{text}");
    assert!(text.contains("\"watchdog\": true"), "{text}");
    assert!(!text.contains("PROFILE_OK"), "{text}");
    // Independent of fixture_watchdog(): omitting its override must not let
    // the 140-second default satisfy this fixture's 12-second contract.
    assert_watchdog_elapsed(elapsed, 12, &text);
}

#[test]
#[should_panic(expected = "late watchdog kill/reap")]
fn unjoined_elapsed_oracle_rejects_the_default_profile_deadline() {
    assert_watchdog_elapsed(std::time::Duration::from_secs(140), 12, "late fixture");
}

#[test]
fn watchdog_rejects_overflow_even_with_a_success_marker() {
    let output = run(2, Some("overflow"));
    let text = diagnostics(&output);
    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("PROFILE_OK fixture-overflow"));
    assert!(text.contains("\"status\": 0"));
    assert!(text.contains("\"overflow\": true"));
    assert!(text.contains("\"watchdog\": false"));
    assert!(text.len() < 1_049_600, "capture exceeded declared bound");
}

#[test]
fn scheduling_child() {
    if let Some(config) = launch::receive() {
        profile::child(config);
    }
}

#[test]
fn scheduling_tool_process_controls() {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/test_scheduling_process.py"
        ))
        .arg("--binary")
        .arg(std::env::current_exe().unwrap())
        .output()
        .expect("scheduling tests require Python 3 on PATH");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
