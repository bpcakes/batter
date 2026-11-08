//! Unix signal registration contracts, including the child processes that
//! receive real signals.

use super::*;
use crate::{
    cleanup::CleanupBudget,
    lifecycle::ShutdownBudget,
    startup::{Startup, StartupCause, StartupError},
};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SIGNAL_CHILD: &str = "BATTER_INSTALLED_SIGNAL_CHILD";
const PARTIAL_SIGNAL_CHILD: &str = "BATTER_PARTIAL_SIGNAL_CHILD";

#[test]
fn partial_reserved_install_retains_the_second_native_error() {
    if std::env::var_os(PARTIAL_SIGNAL_CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut calls = 0;
            let result = install_reserved_with("signals", |kind| {
                calls += 1;
                if calls == 1 {
                    signal(kind)
                } else {
                    Err(io::Error::other("second-listener-marker"))
                }
            });
            let Err(error) = result else {
                panic!("partial installation unexpectedly succeeded")
            };
            let SignalRegistrationError::Install(error) = error else {
                panic!("wrong partial-install error")
            };
            assert_eq!(calls, 2);
            assert_eq!(error.to_string(), "second-listener-marker");
        });
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "lifecycle::unix::tests::partial_reserved_install_retains_the_second_native_error",
            "--nocapture",
        ])
        .env(PARTIAL_SIGNAL_CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}; stdout={:?}; stderr={:?}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn installed_sources_receive_before_driver_registration() {
    if std::env::var_os(SIGNAL_CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let second = Duration::from_secs(1);
            let cleanup = CleanupBudget::new(second, second, second).unwrap();
            let mut supervisor =
                Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
            let mut signals = install_signals(&supervisor, "signals").unwrap();
            println!("installed-signal-child-ready");
            std::io::stdout().flush().unwrap();
            tokio::time::timeout(Duration::from_secs(2), signals.received())
                .await
                .expect("installed source receives SIGTERM before registration");
            signals.register(&mut supervisor).unwrap();
            assert_eq!(
                supervisor.status().readiness(),
                super::super::Readiness::Draining
            );
        });
        return;
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "lifecycle::unix::tests::installed_sources_receive_before_driver_registration",
            "--nocapture",
        ])
        .env(SIGNAL_CHILD, "1")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut output = BufReader::new(output);
        let mut captured = String::new();
        loop {
            let mut line = String::new();
            match output.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    captured.push_str(&line);
                    if line.contains("installed-signal-child-ready") {
                        let _ = ready_sender.try_send(());
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok::<_, std::io::Error>(captured)
    });
    if ready_receiver.recv_timeout(Duration::from_secs(3)).is_err() {
        child.kill().unwrap();
        let _ = child.wait();
        let captured = reader.join().unwrap().unwrap();
        panic!("signal child exited or stalled before readiness: {captured:?}");
    }
    let signaled = Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signaled.success());

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let captured = reader.join().unwrap().unwrap();
            assert!(
                status.success(),
                "signal child failed: {status}; output: {captured:?}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("signal child did not observe SIGTERM");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test]
async fn installation_failure_is_startup_failure_and_runs_prior_cleanup() {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    let supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    let starting = Startup::new(
        supervisor,
        crate::operation::OperationOwner::new(second)
            .unwrap()
            .into_context(),
        cleanup,
        |scope| {
            Box::pin(async move {
                scope
                    .supervisor()
                    .on_cleanup("resource", || async { Ok(()) })?;
                let _installed = install_with(scope.supervisor(), "signals", || {
                    Err(io::Error::other("installation-original"))
                })?;
                Ok(())
            })
        },
    )
    .start();
    let Err(StartupError::Failed(report)) = starting.finish().await else {
        panic!("expected failure")
    };
    let StartupCause::Failed(SignalRegistrationError::Install(error)) = &report.cause else {
        panic!("lost OS cause")
    };
    assert_eq!(error.to_string(), "installation-original");
    assert!(report.cleanup.is_success());
    assert_eq!(report.cleanup.records.len(), 1);
    assert!(!format!("{report:?}").contains("installation-original"));
}
