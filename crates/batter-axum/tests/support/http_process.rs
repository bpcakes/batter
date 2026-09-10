//! Both HTTP targets use the same launch authority and success evidence.
#[allow(dead_code)]
#[path = "../../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../../../../test-support/process/evidence.rs"]
mod evidence;
#[path = "../../../../test-support/process/launch.rs"]
mod launch;
#[allow(dead_code)]
#[path = "../../../../test-support/process/timing.rs"]
pub mod timing;
#[allow(dead_code)]
#[path = "http_process_watchdog.rs"]
mod watchdog;

pub use watchdog::{ChildRun, ExpectedExit};

// Complete diagnostic phases, shared by both fixture drivers. Individual waits
// may be tighter; they must never grant a fresh phase allowance.
pub const STARTUP: std::time::Duration = std::time::Duration::from_secs(1);
pub const EXERCISE: std::time::Duration = std::time::Duration::from_secs(2);
pub const TEARDOWN: std::time::Duration = std::time::Duration::from_millis(3500);
const MARGIN: std::time::Duration = std::time::Duration::from_secs(1);
pub const PARENT: std::time::Duration = std::time::Duration::from_secs(8);
pub const REPORT_DELAY: std::time::Duration = std::time::Duration::from_millis(2200);

// A real finalizer completes after the exercise allowance, within teardown.
// This policy is only for the delayed-report success controls.
pub fn delayed_report_budget() -> batter::lifecycle::ShutdownBudget {
    use std::time::Duration;
    let short = Duration::from_millis(100);
    let cleanup = Duration::from_millis(2700);
    let budget = batter::lifecycle::ShutdownBudget::new(
        Duration::from_millis(400),
        short,
        short,
        batter::cleanup::CleanupBudget::new(cleanup, cleanup, short).unwrap(),
    )
    .unwrap();
    assert!(EXERCISE < REPORT_DELAY && REPORT_DELAY < cleanup);
    assert!(budget.total_allowance() < TEARDOWN);
    budget
}

pub fn run(name: &str) -> ChildRun {
    assert!(STARTUP + EXERCISE + TEARDOWN + MARGIN < PARENT);
    assert!(PARENT < launch::EMERGENCY_LIMIT);
    ChildRun::run_with_policy(name, timing::WaitPolicy::ExitAfter(PARENT))
}

pub fn success(run: &ChildRun, name: &str) -> Result<(), String> {
    run.validate(
        ExpectedExit::Success,
        &[&format!("http-case-complete:{name}")],
        &[],
    )
}

fn complete(name: &str) {
    use std::io::Write;
    let mut output = std::io::stdout().lock();
    writeln!(output, "\nbatter-fixture:http-case-complete:{name}").unwrap();
    output.flush().unwrap();
}

pub fn fixture(exercise: impl FnOnce(&str)) {
    if let Some(name) = launch::scenario() {
        match name.as_str() {
            "control-no-completion" => std::process::exit(0),
            "control-wrong-completion" => complete("different-case"),
            _ => {
                exercise(&name);
                complete(&name);
            }
        }
    }
}

#[test]
fn successful_exit_without_matching_completion_is_rejected() {
    for name in ["control-no-completion", "control-wrong-completion"] {
        let run = run(name);
        assert!(run.status.success(), "{run:?}");
        assert!(success(&run, name).unwrap_err().contains("missing event"));
    }
}
