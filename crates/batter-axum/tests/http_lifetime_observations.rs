//! Real HTTP/1.1 observations, each contained by an independent Unix watchdog.
// Shared capture also exposes filter constructors used by other targets.
#[allow(dead_code)]
#[path = "support/capture.rs"]
mod capture;
#[path = "support/http_graceful.rs"]
mod http_graceful;
mod http_lifetime_observations {
    pub mod cases;
    pub mod client;
    pub mod driver;
    pub mod fixture;
    pub mod limits;
    pub mod resource;
    pub mod transport;
}

#[path = "../../../test-support/events.rs"]
mod events;
#[path = "support/http_process.rs"]
mod http_process;
use http_process::{ExpectedExit, run};

macro_rules! cases {
    ($($name:ident),+ $(,)?) => {$(
        #[test]
        fn $name() {
            let name = stringify!($name);
            http_process::success(&run(name), name).unwrap();
        }
    )+};
}
cases!(
    idle_keep_alive_drain,
    existing_connection_drain,
    admission_with_graceful_withheld,
    admitted_handler_drain,
    incomplete_upload_deadline,
    streaming_cooperative_drain,
    forced_handler_cancellation,
    blocked_body_wrapper_abort,
    disconnect_pending_handler,
    disconnect_streaming_body,
    write_half_close_pending_handler,
    delayed_shutdown_report,
);

#[test]
fn missing_reconciliation_event_retains_report_and_drop_evidence() {
    for name in [
        "missing_reconciliation_event",
        "delayed_missing_reconciliation_event",
    ] {
        let run = run(name);
        assert!(!run.kill_requested(), "{run:?}");
        assert_eq!(run.status.code(), Some(101));
        let text = http_process::success(&run, "missing_reconciliation_event").unwrap_err();
        for expected in [
            "exercise=Some(Ok(Ok(())))",
            "report=Ok(Ok(SharedShutdownReport",
            "reconciliation=Some(Ok(Err(Elapsed(()))))",
            "interrupted event: never-reconciled",
            "reconciliation-resource-drop",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        let report = text.find("lifetime: terminal-report").unwrap();
        let dropped = text.find("lifetime: reconciliation-resource-drop").unwrap();
        assert!(report < dropped, "{text}");
    }
}

#[test]
fn exercise_deadline_retains_missing_event_and_wire_bytes() {
    for (name, detail) in [
        ("missing_exercise_event", "event: never-exercised"),
        ("missing_wire_marker", "wire marker:"),
    ] {
        let run = run(name);
        assert!(!run.kill_requested(), "{run:?}");
        assert_eq!(run.status.code(), Some(101));
        let text = http_process::success(&run, name).unwrap_err();
        for expected in [
            "exercise=Some(Ok(Err(Elapsed(()))))",
            "report=Ok(Ok(SharedShutdownReport",
            "reconciliation=Some(Ok(Ok(())))",
            "events: [",
            "trace:",
            "HTTP response boundary finished",
            detail,
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        if name == "missing_wire_marker" {
            let wire = text.split_once("interrupted wire marker:").unwrap().1;
            assert!(
                wire.contains("never-on-wire")
                    && wire.contains("HTTP/1.1 200 OK")
                    && wire.contains("ok"),
                "{text}"
            );
        }
    }
}

#[test]
fn disconnect_deadline_retains_missing_event() {
    let run = run("missing_disconnect_event");
    assert!(!run.kill_requested(), "{run:?}");
    assert_eq!(run.status.code(), Some(101));
    let text = http_process::success(&run, "missing_disconnect_event").unwrap_err();
    for expected in [
        "exercise=Some(Err",
        "interrupted event: never-disconnected",
        "report=Ok(Ok(SharedShutdownReport",
        "reconciliation=Some(Ok(Ok(())))",
        "handler-drop",
        "server-socket-drop",
        "HTTP response boundary finished",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn terminal_report_rechecks_handler_entry_count() {
    let run = run("late_handler_entry");
    assert!(!run.kill_requested(), "{run:?}");
    assert_eq!(run.status.code(), Some(101));
    let text = http_process::success(&run, "late_handler_entry").unwrap_err();
    for expected in [
        "exercise=Some(Ok(Ok(())))",
        "report=Ok(Ok(SharedShutdownReport",
        "reconciliation=Some(Err",
        "terminal handler entry count",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn watchdog_rejects_stalled_runtime_and_reaps_child() {
    let run = run("stall");
    assert!(run.kill_requested());
    run.validate(ExpectedExit::WatchdogKill, &["http-runtime-stalled"], &[])
        .unwrap();
    assert!(http_process::success(&run, "stall").is_err());
}

#[test]
fn exercise_and_teardown_failures_are_both_retained() {
    let run = run("dual_failure");
    assert!(!run.kill_requested());
    assert_eq!(run.status.code(), Some(101));
    let text = http_process::success(&run, "dual_failure").unwrap_err();
    for expected in [
        "exercise=Some(Err",
        "exercise-failure-control",
        "reconciliation=Some(Err",
        "cleanup-failure-control",
    ] {
        assert!(text.contains(expected), "{text}");
    }
}

#[test]
fn slow_exercise_and_teardown_retain_both_timeouts_and_report() {
    let run = run("slow_failure");
    assert!(!run.kill_requested(), "{run:?}");
    assert_eq!(run.status.code(), Some(101));
    let text = http_process::success(&run, "slow_failure").unwrap_err();
    for expected in [
        "exercise=Some(Ok(Err(Elapsed(()))))",
        "report=Ok(Ok(SharedShutdownReport",
        "reconciliation=Some(Ok(Err(Elapsed(()))))",
        "exercise-resource-drop",
        "teardown-resource-drop",
        "finalizer",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let exercise_drop = text.find("lifetime: exercise-resource-drop").unwrap();
    let cleanup = text.find("lifetime: finalizer").unwrap();
    let teardown_drop = text.find("lifetime: teardown-resource-drop").unwrap();
    assert!(exercise_drop < cleanup && cleanup < teardown_drop, "{text}");
}

#[test]
fn readiness_timeout_preserves_owned_teardown() {
    let run = run("startup_timeout");
    assert!(!run.kill_requested(), "{run:?}");
    assert_eq!(run.status.code(), Some(101));
    let text = http_process::success(&run, "startup_timeout").unwrap_err();
    for expected in [
        "startup=Err(Elapsed(()))",
        "exercise=None",
        "report=Ok(Ok(SharedShutdownReport",
        "reconciliation=Some(Ok(Ok(())))",
        "server-ok",
        "finalizer",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("lifetime: handler-entry"), "{text}");
}

#[test]
fn child_fixture() {
    http_process::fixture(|case| {
        let capture = std::sync::Arc::new(capture::Capture::with_filter(http_graceful::FILTER));
        capture.block_on(async {
            if case == "stall" {
                use std::io::Write;
                writeln!(
                    std::io::stdout().lock(),
                    "\nbatter-fixture:http-runtime-stalled"
                )
                .unwrap();
                loop {
                    std::thread::park();
                }
            }
            http_lifetime_observations::driver::run(case, capture.clone()).await;
        });
    });
}
