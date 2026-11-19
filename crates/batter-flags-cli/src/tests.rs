//! Exercise the real command owner with an outstanding blocking task, like
//! Tokio's resolver. No DNS timing or external database is needed. The shared
//! Unix watchdog bounds failures even if runtime destruction blocks forever.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(
    clippy::expect_used,
    reason = "shared fixture launch fails by panicking"
)]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![allow(clippy::unreachable, reason = "only the parent selects fixture names")]
#![allow(
    clippy::panic,
    reason = "the panic fixture must unwind through the runtime owner"
)]
#![allow(
    dead_code,
    reason = "shared process support; this suite uses part of it"
)]

#[path = "../../../test-support/process/capture.rs"]
mod capture;
#[path = "../../../test-support/process/evidence.rs"]
mod evidence;
#[path = "../../../test-support/process/launch.rs"]
mod launch;
#[path = "../../../test-support/process/timing.rs"]
mod timing;
#[path = "../../../test-support/process/watchdog.rs"]
mod watchdog;

use super::{Endpoint, Output, commands::Command, on_runtime, plan, run_prepared};
use std::{io::Write, net::TcpListener, sync::mpsc, time::Duration};
use timing::WaitPolicy;
use watchdog::{ChildRun, ExpectedExit};

enum End {
    Deadline,
    Signal(&'static str),
    Panic,
}

#[test]
fn command_deadline_returns_its_report_and_exits_with_blocking_work_outstanding() {
    check("deadline");
}

#[test]
fn command_signals_return_their_report_and_exit_with_blocking_work_outstanding() {
    for name in ["interrupt", "terminate"] {
        check(name);
    }
}

#[test]
fn a_panicking_command_exits_with_blocking_work_outstanding() {
    let run = ChildRun::run_with_policy("panic", WaitPolicy::ExitAfter(Duration::from_secs(8)));
    run.validate(
        ExpectedExit::Code(101),
        &[
            "blocking-started",
            "about-to-panic",
            "original-panic-resumed",
        ],
        &[
            "blocking-finished",
            "report-validated",
            "runtime-returned",
            "unexpected-panic",
        ],
    )
    .unwrap();
}

fn check(name: &str) {
    let run = ChildRun::run_with_policy(name, WaitPolicy::ExitAfter(Duration::from_secs(8)));
    run.validate(
        ExpectedExit::Code(1),
        &[
            "blocking-started",
            "preflight-connected",
            "report-validated",
        ],
        &["blocking-finished"],
    )
    .unwrap();
}

pub(super) fn fixture() {
    let Some(name) = launch::scenario() else {
        return;
    };
    let end = match name.as_str() {
        "deadline" => End::Deadline,
        "interrupt" => End::Signal("-INT"),
        "terminate" => End::Signal("-TERM"),
        "panic" => End::Panic,
        _ => unreachable!("unknown fixture"),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (started, running) = mpsc::channel();
    let (_hold, held) = mpsc::channel::<()>();
    runtime.spawn_blocking(move || {
        event("blocking-started");
        started.send(()).unwrap();
        let _ = held.recv();
        event("blocking-finished");
    });
    running.recv_timeout(Duration::from_secs(2)).unwrap();

    if let End::Panic = end {
        std::mem::forget(_hold);
        panicking(runtime);
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Endpoint::parse(&format!(
        "postgres://ops@{}/flags?sslmode=disable",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let timeout = match end {
        End::Deadline => Duration::from_secs(1),
        End::Signal(_) => Duration::from_secs(60),
        End::Panic => unreachable!("the panic case already unwound"),
    };
    let (connected, connection) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        event("preflight-connected");
        // Connection establishment happens after the command installs handlers.
        if let End::Signal(signal) = end {
            assert!(
                std::process::Command::new("kill")
                    .args([signal, &std::process::id().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        connected.send(stream).unwrap();
    });
    let plan = plan::plan(Command::Show {
        key: "value".into(),
    })
    .unwrap();
    let report = run_prepared(runtime, plan, endpoint, timeout);
    server.join().unwrap();
    let _stream = connection.recv_timeout(Duration::from_secs(2)).unwrap();
    let rendered = report.render(Output::Json);
    let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(json["command"], "show");
    assert_eq!(json["exit"], 1);
    assert_eq!(
        json["outcome"],
        if name == "deadline" {
            "failed"
        } else {
            "interrupted"
        }
    );
    if name == "deadline" {
        assert_eq!(
            json["message"],
            "the timeout passed before COMMIT; nothing written"
        );
    }
    println!("{rendered}");
    event("report-validated");
    // Keep the blocking task held through exit, exactly the root's boundary.
    std::process::exit(i32::from(report.exit));
}

fn panicking(runtime: tokio::runtime::Runtime) -> ! {
    // Only this isolated, intentionally panicking child suppresses its hook:
    // the shared watchdog rejects panic text even for an expected exit 101.
    // Libtest still observes the resumed panic and supplies the exit status.
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        on_runtime(runtime, async {
            event("about-to-panic");
            panic!("fixed fixture panic");
        })
    }));
    match outcome {
        Err(payload) if payload.downcast_ref::<&str>() == Some(&"fixed fixture panic") => {
            event("original-panic-resumed");
            std::panic::resume_unwind(payload);
        }
        Err(_) => {
            event("unexpected-panic");
            std::process::exit(2);
        }
        Ok(_) => {
            event("runtime-returned");
            std::process::exit(1);
        }
    }
}

fn event(name: &str) {
    let mut output = std::io::stdout().lock();
    writeln!(output, "\nbatter-fixture:{name}").unwrap();
    output.flush().unwrap();
}
