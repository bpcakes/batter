//! Real SIGTERM and SIGINT against finite commands, each case in a fresh child
//! process launched through the shared fixture protocol. The parent sends a
//! signal only after the child printed the marker that makes it meaningful.

#[allow(dead_code)]
#[path = "../../../test-support/process/capture.rs"]
mod capture;
#[path = "command_signals/child.rs"]
mod child;
#[allow(dead_code)]
#[path = "../../../test-support/process/evidence.rs"]
mod evidence;
#[path = "command_signals/fixture.rs"]
mod fixture;
#[path = "../../../test-support/process/launch.rs"]
mod launch;

use fixture::{Exit, SignalledChild, StopSignal};

const SIGNALS: [StopSignal; 2] = [StopSignal::Term, StopSignal::Int];

#[test]
fn child_fixture() {
    if let Some(scenario) = launch::scenario() {
        child::run(&scenario);
    }
}

#[test]
fn signal_visible_before_the_first_coordinator_poll_prevents_the_factory() {
    for signal in SIGNALS {
        let mut child = SignalledChild::spawn("before-work");
        child.await_event("before-work-ready");
        child.signal(signal);
        child.expect_exit(
            Exit::Success,
            &["before-work-checked"],
            &["factory-invoked"],
        );
    }
}

fn signal_during_work(scenario: &'static str) {
    for signal in SIGNALS {
        let mut child = SignalledChild::spawn(scenario);
        child.await_event("work-started");
        child.signal(signal);
        child.expect_exit(
            Exit::Success,
            &["resource-released", "during-work-checked"],
            &[],
        );
    }
}

#[test]
fn signal_during_work_cancels_it_and_still_runs_lifo_cleanup() {
    signal_during_work("during-work");
}

#[test]
fn signal_during_work_on_a_multi_thread_runtime_cancels_it_and_runs_cleanup() {
    signal_during_work("during-work-multi-thread");
}

#[test]
fn signal_during_cleanup_neither_interrupts_nor_fails_a_successful_command() {
    for signal in SIGNALS {
        let mut child = SignalledChild::spawn("during-cleanup");
        child.await_event("cleanup-started");
        child.signal(signal);
        child.expect_exit(
            Exit::Success,
            &["cleanup-saw-signal", "during-cleanup-checked"],
            &[],
        );
    }
}

#[test]
fn repeated_signals_cancel_work_once_and_leave_running_cleanup_alone() {
    for (first, second) in [
        (StopSignal::Term, StopSignal::Int),
        (StopSignal::Int, StopSignal::Term),
    ] {
        let mut child = SignalledChild::spawn("repeated");
        child.await_event("work-started");
        child.signal(first);
        child.await_event("cleanup-started");
        child.signal(second);
        child.expect_exit(
            Exit::Success,
            &["cleanup-saw-signal", "repeated-checked"],
            &[],
        );
    }
}

/// Negative controls: the inert and unsignalled paths leave each signal's
/// default disposition, which terminates the child. A shell background job
/// that inherited an ignored SIGINT would make the INT case time out instead.
#[test]
fn unsignalled_paths_keep_the_default_signal_disposition() {
    for (scenario, ready) in [
        ("never-started", "never-started-ready"),
        ("without-signals", "work-started"),
    ] {
        for signal in SIGNALS {
            let mut child = SignalledChild::spawn(scenario);
            child.await_event(ready);
            child.signal(signal);
            child.expect_exit(Exit::KilledBy(signal), &[], &[]);
        }
    }
}
