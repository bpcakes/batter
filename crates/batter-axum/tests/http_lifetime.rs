//! Real HTTP/1.1 ownership cases, each bounded outside its Tokio runtime.

#[path = "../../../test-support/events.rs"]
mod events;
#[path = "support/http_process.rs"]
mod http_process;

#[path = "http_lifetime/client.rs"]
mod client;
#[path = "http_lifetime/limits.rs"]
mod limits;
#[path = "http_lifetime/scenarios.rs"]
mod scenarios;
#[path = "http_lifetime/server.rs"]
mod server;
#[path = "http_lifetime/state.rs"]
mod state;
#[allow(dead_code)]
#[path = "support/capture.rs"]
mod trace_capture;

#[path = "support/http_graceful.rs"]
mod http_graceful;

use http_process::{ChildRun, ExpectedExit};

fn run(name: &str) -> ChildRun {
    limits::validate();
    http_process::run(name)
}

fn case(name: &str) {
    let run = run(name);
    http_process::success(&run, name).unwrap();
    if name == "http-reject" {
        let branch = if run
            .validate(ExpectedExit::Success, &["drain-request-rejected"], &[])
            .is_ok()
        {
            "received-503"
        } else {
            run.validate(
                ExpectedExit::Success,
                &["drain-request-transport-closed"],
                &[],
            )
            .unwrap();
            "transport-closed"
        };
        eprintln!("ordinary drain outcome: {branch}");
    }
}

#[test]
fn delayed_shutdown_report_uses_teardown_allowance() {
    case("http-delayed-report");
}
#[test]
fn idle_keep_alive_closes_on_drain() {
    case("http-idle");
}
#[test]
fn admitted_handler_completes_during_drain() {
    case("http-handler");
}
#[test]
fn established_connection_cannot_admit_after_drain() {
    case("http-reject");
}
#[test]
fn synchronized_admission_rejects_before_graceful_notification() {
    case("http-admission");
}
#[test]
fn incomplete_upload_is_bounded_by_response_deadline() {
    case("http-upload");
}
#[test]
fn response_context_completion_does_not_finish_streaming_body() {
    case("http-stream");
}
#[test]
fn forced_cancellation_drops_handler_and_can_join_cleanly() {
    case("http-cancel");
}
#[test]
fn aborted_server_wrapper_leaves_body_pending_and_skips_cleanup() {
    case("http-abort");
}
#[test]
fn full_disconnect_before_response_drops_handler() {
    case("http-disconnect-handler");
}
#[test]
fn full_disconnect_while_streaming_drops_body() {
    case("http-disconnect-body");
}

#[test]
fn runtime_stall_is_reaped_and_rejected_as_lifetime_success() {
    let run = run("http-stall");
    assert!(run.kill_requested());
    run.validate(ExpectedExit::WatchdogKill, &["http-runtime-stalled"], &[])
        .unwrap();
    assert!(http_process::success(&run, "http-stall").is_err());
}

#[test]
fn missing_event_retains_progress_and_teardown_diagnostics() {
    let run = run("http-missing-event");
    assert!(!run.kill_requested());
    assert_eq!(run.status.code(), Some(101));
    let error = http_process::success(&run, "http-missing-event").unwrap_err();
    for evidence in [
        "batter-http-wait:never-emitted",
        "Elapsed(())",
        "teardown: Ok(Ok(SharedShutdownReport",
        "abort_requested: []",
        "trace:",
    ] {
        assert!(error.contains(evidence), "missing {evidence}: {error}");
    }
    let snapshot = error
        .split_once("events: [")
        .unwrap()
        .1
        .split_once(']')
        .unwrap()
        .0;
    assert!(snapshot.contains("cleanup"), "{error}");
}

#[test]
fn timed_out_exercise_retains_forced_teardown_report() {
    let run = run("http-teardown-stuck");
    assert!(!run.kill_requested());
    assert_eq!(run.status.code(), Some(101));
    let error = http_process::success(&run, "http-teardown-stuck").unwrap_err();
    let report = error
        .split_once("teardown: ")
        .unwrap()
        .1
        .split_once("; events:")
        .unwrap()
        .0;
    for evidence in [
        "Ok(Ok(SharedShutdownReport",
        "Aborted",
        "UnsafeTaskExit",
        "forced_cancellation: true",
    ] {
        assert!(report.contains(evidence), "missing {evidence}: {error}");
    }
    assert!(
        error.contains("exercise: Some(Ok(Err(Elapsed(()))))"),
        "{error}"
    );
}

#[test]
fn readiness_timeout_retains_owned_server_teardown() {
    let run = run("http-startup-stuck");
    assert!(!run.kill_requested());
    assert_eq!(run.status.code(), Some(101));
    let error = http_process::success(&run, "http-startup-stuck").unwrap_err();
    for evidence in [
        "startup: Err(Elapsed(()))",
        "exercise: None",
        "teardown: Ok(Ok(SharedShutdownReport",
        "Aborted",
        "UnsafeTaskExit",
        "trace:",
    ] {
        assert!(error.contains(evidence), "missing {evidence}: {error}");
    }
}

#[test]
fn child_fixture() {
    http_process::fixture(scenarios::run);
}

#[test]
fn terminal_report_rechecks_handler_entry_count() {
    let run = run("http-late-handler");
    assert!(!run.kill_requested(), "{run:?}");
    assert_eq!(run.status.code(), Some(101));
    let text = http_process::success(&run, "http-late-handler").unwrap_err();
    assert!(text.contains("terminal handler entry count"), "{text}");
}
