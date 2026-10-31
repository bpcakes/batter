//! Fresh processes keep sibling tests from pre-registering cleanup callsites.
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
mod timing;
#[allow(dead_code)]
#[path = "../../../../test-support/process/watchdog.rs"]
mod watchdog;

fn isolated(scenario: &str) {
    watchdog::ChildRun::run_with_policy(
        scenario,
        timing::WaitPolicy::ExitAfter(std::time::Duration::from_secs(5)),
    )
    .validate(
        watchdog::ExpectedExit::Success,
        &["cleanup-ordering-preserved"],
        &[],
    )
    .unwrap();
}

#[test]
fn unscoped_first_skip_preserves_warning_metric_and_destruction_order() {
    isolated("skip");
}

#[test]
fn unscoped_first_close_preserves_info_before_metric() {
    isolated("close");
}

pub(super) fn child_fixture() {
    if let Some(scenario) = launch::scenario() {
        match scenario.as_str() {
            // The ordinary test additionally covers a panicking destructor;
            // the process watchdog deliberately rejects any panic output.
            "skip" => {
                super::ordering::assert_skip_ordering(super::ordering::CaptureDestruction::Normal)
            }
            "close" => super::ordering::observed_cleanup_traces_before_recording(),
            _ => panic!("unknown cleanup case"),
        }
        use std::io::Write;
        writeln!(
            std::io::stdout().lock(),
            "\nbatter-fixture:cleanup-ordering-preserved"
        )
        .unwrap();
    }
}
