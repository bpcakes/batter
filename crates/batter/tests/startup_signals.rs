use batter::{
    RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{ManagedComponent, ManagedSettlement, ShutdownBudget, Supervisor},
    operation::OperationContext,
    startup::{InitializationError, Startup, StartupCause, StartupError},
};
use std::{
    future::pending,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::process::ExitStatusExt,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const CHILD_MODE: &str = "BATTER_PROTECTED_SIGNAL_CHILD";

struct NativeReport;

impl ManagedSettlement for NativeReport {
    fn is_success(&self) -> bool {
        true
    }

    fn allows_dependency_cleanup(&self) -> bool {
        true
    }
}

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap()
}

fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            cleanup_budget(),
        )
        .unwrap(),
    )
}

fn failure(
    result: Result<
        batter::lifecycle::RunningSupervisor,
        StartupError<InitializationError<RegistrationError>>,
    >,
) -> Arc<batter::startup::StartupFailure<InitializationError<RegistrationError>>> {
    match result {
        Err(StartupError::Failed(report)) => report,
        _ => panic!("expected protected startup failure"),
    }
}

#[tokio::test]
async fn invalid_occupied_and_repeated_policies_skip_the_initializer_and_clean_up() {
    for case in ["invalid", "occupied", "repeated"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let cleaned = Arc::new(AtomicUsize::new(0));
        let cleanup = cleaned.clone();
        let mut process = supervisor();
        process
            .reserve_cleanup("resource")
            .unwrap()
            .register(move || async move {
                cleanup.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
        if case == "occupied" {
            process
                .register("signals", |shutdown| async move {
                    shutdown.mark_started();
                    shutdown.draining().await;
                    Ok(())
                })
                .unwrap();
        }
        let specification = Startup::scoped(
            process,
            OperationContext::new(Duration::from_secs(1)).unwrap(),
            cleanup_budget(),
            move |_scope| {
                called.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok::<_, RegistrationError>(()) })
            },
        );
        let specification = match case {
            "invalid" => specification.with_unix_signals(""),
            "occupied" => specification.with_unix_signals("signals"),
            "repeated" => specification
                .with_unix_signals("signals")
                .with_unix_signals("other-signals"),
            _ => unreachable!(),
        };
        let mut starting = specification.start();
        let report = failure(starting.wait().await);
        match case {
            "repeated" => assert!(matches!(
                report.cause,
                StartupCause::Failed(InitializationError::SignalPolicyAlreadySelected)
            )),
            "invalid" => assert!(matches!(
                report.cause,
                StartupCause::Failed(InitializationError::Signals(
                    batter::lifecycle::SignalRegistrationError::Registration(
                        RegistrationError::InvalidName
                    )
                ))
            )),
            "occupied" => assert!(matches!(
                report.cause,
                StartupCause::Failed(InitializationError::Signals(
                    batter::lifecycle::SignalRegistrationError::Registration(
                        RegistrationError::Duplicate("signals")
                    )
                ))
            )),
            _ => unreachable!(),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
        assert!(report.cleanup.is_success());
    }
}

#[tokio::test]
async fn repeated_policy_precedes_preexisting_cancellation() {
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    context.cancel();
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let mut starting = Startup::scoped(supervisor(), context, cleanup_budget(), move |_scope| {
        called.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok::<_, RegistrationError>(()) })
    })
    .with_unix_signals("signals")
    .with_unix_signals("signals")
    .start();
    let report = failure(starting.wait().await);
    assert!(matches!(
        report.cause,
        StartupCause::Failed(InitializationError::SignalPolicyAlreadySelected)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn configured_signals_cover_start_return_running_and_explicit_opt_out() {
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        run_child(&mode);
        return;
    }

    for signal in ["TERM", "INT"] {
        let output = run_signaled_child("startup", signal, true);
        assert!(output.contains("startup-cleanup"), "{output:?}");
        assert!(output.contains("startup-draining"), "{output:?}");

        let output = run_signaled_child("held", signal, true);
        assert!(output.contains("held-cleanup"), "{output:?}");
        assert!(output.contains("held-draining"), "{output:?}");

        let output = run_signaled_child("waiter", signal, true);
        assert!(output.contains("waiter-draining"), "{output:?}");

        let output = run_signaled_child("running", signal, true);
        assert!(output.contains("running-complete"), "{output:?}");

        let output = run_signaled_child("unapproved", signal, true);
        assert!(output.contains("unapproved-complete"), "{output:?}");

        run_signaled_child("configured-unstarted", signal, false);
        run_signaled_child("started-unconfigured", signal, false);
    }

    let output = run_child_to_completion("reserved-name");
    assert!(output.contains("reserved-name-rejected"), "{output:?}");
    let output = run_child_to_completion("reserved-managed-name");
    assert!(output.contains("reserved-name-rejected"), "{output:?}");
    let output = run_child_to_completion("owner-loss");
    assert!(output.contains("owner-loss-cleanup"), "{output:?}");
}

#[test]
fn signal_child_capture_drains_stderr_and_reaps_a_pre_marker_stall() {
    let output = run_child_to_completion_with_limit("stderr-flood", Duration::from_secs(2));
    assert!(output.contains("stderr-flood-ready"), "{output:?}");

    let started = Instant::now();
    assert!(
        std::panic::catch_unwind(|| {
            run_child_to_completion_with_limit("stall-before-marker", Duration::from_millis(50));
        })
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

fn run_child(mode: &str) {
    match mode {
        "startup" => run_startup_child(),
        "held" => run_held_child(),
        "waiter" => run_waiter_child(),
        "running" => run_running_child(),
        "unapproved" => run_unapproved_child(),
        "configured-unstarted" => run_default_termination_child(true),
        "started-unconfigured" => run_default_termination_child(false),
        "reserved-name" => run_reserved_name_child(false),
        "reserved-managed-name" => run_reserved_name_child(true),
        "owner-loss" => run_owner_loss_child(),
        "stderr-flood" => {
            std::io::stderr()
                .write_all(&vec![b'x'; 256 * 1024])
                .unwrap();
            println!("stderr-flood-ready");
        }
        "stall-before-marker" => runtime().block_on(std::future::pending()),
        _ => panic!("unknown signal child mode"),
    }
}

fn run_held_child() {
    runtime().block_on(async {
        let process = supervisor();
        let handle = process.handle();
        let mut starting = Startup::scoped(
            process,
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            |scope| {
                Box::pin(async move {
                    scope.reserve_cleanup("resource")?.register(|| async {
                        println!("held-cleanup");
                        Ok(())
                    });
                    println!("held-ready");
                    std::io::stdout().flush().unwrap();
                    std::future::pending::<Result<(), RegistrationError>>().await
                })
            },
        )
        .with_unix_signals("signals")
        .start();
        let report = failure(starting.wait().await);
        assert!(matches!(report.cause, StartupCause::Draining));
        assert_eq!(handle.readiness(), batter::lifecycle::Readiness::Draining);
        assert!(report.cleanup.is_success());
        println!("held-draining");
    });
}

fn run_waiter_child() {
    runtime().block_on(async {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let mut starting = Startup::scoped(
            supervisor(),
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            move |scope| {
                Box::pin(async move {
                    scope
                        .reserve_cleanup("resource")?
                        .register(|| async { Ok(()) });
                    entered_tx.send(()).unwrap();
                    std::future::pending::<Result<(), RegistrationError>>().await
                })
            },
        )
        .with_unix_signals("signals")
        .start();
        entered_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), starting.wait())
                .await
                .is_err()
        );
        println!("waiter-cancelled");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        let report = failure(starting.wait().await);
        assert!(matches!(report.cause, StartupCause::Draining));
        assert!(report.cleanup.is_success());
        println!("waiter-draining");
    });
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn wait_for_parent() {
    let mut release = [0_u8; 1];
    std::io::stdin().read_exact(&mut release).unwrap();
}

fn run_startup_child() {
    runtime().block_on(async {
        let mut process = supervisor();
        process
            .reserve_cleanup("signals")
            .unwrap()
            .register(|| async {
                println!("startup-cleanup");
                Ok(())
            });
        let mut starting = Startup::scoped(
            process,
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            |_scope| Box::pin(std::future::pending::<Result<(), RegistrationError>>()),
        )
        .with_unix_signals("signals")
        .start();
        println!("start-returned");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        let report = failure(starting.wait().await);
        assert!(matches!(report.cause, StartupCause::Draining));
        assert!(report.cleanup.is_success());
        println!("startup-draining");
    });
}

fn run_reserved_name_child(managed: bool) {
    runtime().block_on(async {
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let mut process = supervisor();
        process
            .reserve_cleanup("signals")
            .unwrap()
            .register(|| async { Ok(()) });
        let mut starting = Startup::scoped(
            process,
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            move |scope| {
                Box::pin(async move {
                    if managed {
                        scope.registration().register_managed(
                            "signals",
                            OperationContext::new(Duration::from_secs(1)).unwrap(),
                            move |_shutdown| {
                                called.fetch_add(1, Ordering::SeqCst);
                                Ok(ManagedComponent::new(
                                    async { Ok(()) },
                                    pending(),
                                    |started| started,
                                    async { NativeReport },
                                ))
                            },
                        )?;
                    } else {
                        scope.registration().register("signals", move |_shutdown| {
                            called.fetch_add(1, Ordering::SeqCst);
                            async { Ok(()) }
                        })?;
                    }
                    Ok::<_, RegistrationError>(())
                })
            },
        )
        .with_unix_signals("signals")
        .start();
        let report = failure(starting.wait().await);
        assert!(matches!(
            report.cause,
            StartupCause::Failed(InitializationError::Application(
                RegistrationError::Duplicate("signals")
            ))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(report.cleanup.is_success());
        println!("reserved-name-rejected");
    });
}

fn run_running_child() {
    runtime().block_on(async {
        let events = Arc::new(Mutex::new(Vec::new()));
        let stopped = events.clone();
        let cleaned = events.clone();
        let mut starting = Startup::scoped(
            supervisor(),
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            move |scope| {
                Box::pin(async move {
                    scope
                        .reserve_cleanup("resource")?
                        .register(move || async move {
                            cleaned.lock().unwrap().push("cleanup");
                            Ok(())
                        });
                    scope
                        .registration()
                        .register("worker", move |shutdown| async move {
                            shutdown.mark_started();
                            shutdown.draining().await;
                            stopped.lock().unwrap().push("worker");
                            Ok(())
                        })?;
                    Ok::<_, RegistrationError>(())
                })
            },
        )
        .with_unix_signals("signals")
        .start();
        let running = starting.wait().await.unwrap();
        running.handle().wait_ready().await.unwrap();
        println!("running-ready");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        let report = running.wait().await.unwrap();
        assert!(report.is_success(), "{report}");
        assert_eq!(*events.lock().unwrap(), ["worker", "cleanup"]);
        println!("running-complete");
    });
}

fn run_unapproved_child() {
    runtime().block_on(async {
        let process = supervisor();
        let handle = process.handle();
        let mut starting = Startup::scoped(
            process,
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            |scope| {
                Box::pin(async move {
                    scope
                        .registration()
                        .register("worker", |shutdown| async move {
                            shutdown.mark_started();
                            shutdown.draining().await;
                            Ok(())
                        })?;
                    Ok::<_, RegistrationError>(())
                })
            },
        )
        .without_readiness_approval()
        .with_unix_signals("signals")
        .start();
        let running = starting.wait().await.unwrap();
        assert_eq!(handle.readiness(), batter::lifecycle::Readiness::Starting);
        println!("unapproved-running");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        let report = running.wait().await.unwrap();
        assert!(report.is_success(), "{report}");
        println!("unapproved-complete");
    });
}

fn run_owner_loss_child() {
    runtime().block_on(async {
        let closed = Arc::new(AtomicUsize::new(0));
        let closing = closed.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let starting = Startup::scoped(
            supervisor(),
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            move |scope| {
                Box::pin(async move {
                    scope
                        .reserve_cleanup("resource")?
                        .register(move || async move {
                            closing.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        });
                    entered_tx.send(()).unwrap();
                    std::future::pending::<Result<(), RegistrationError>>().await
                })
            },
        )
        .with_unix_signals("signals")
        .start();
        let observer = starting.observer();
        entered_rx.await.unwrap();
        drop(starting);
        let batter::startup::StartupOutcome::Failed(StartupError::Failed(report)) =
            observer.wait().await
        else {
            panic!("missing owner-loss report")
        };
        assert!(matches!(report.cause, StartupCause::Draining));
        assert!(report.cleanup.is_success());
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        println!("owner-loss-cleanup");
    });
}

fn run_default_termination_child(configured: bool) {
    runtime().block_on(async {
        let specification = Startup::scoped(
            supervisor(),
            OperationContext::new(Duration::from_secs(5)).unwrap(),
            cleanup_budget(),
            |_scope| Box::pin(std::future::pending::<Result<(), RegistrationError>>()),
        );
        if configured {
            let configured = specification.with_unix_signals("signals");
            drop(configured);
        } else {
            let _starting = specification.start();
        }
        println!("default-ready");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        std::future::pending::<()>().await;
    });
}

fn run_signaled_child(mode: &str, signal: &str, survives: bool) -> String {
    let mut child = CapturedChild::spawn(mode, true);
    let marker_deadline = Instant::now() + Duration::from_secs(5);
    let expected = match mode {
        "running" => "running-ready",
        "unapproved" => "unapproved-running",
        "held" => "held-ready",
        "waiter" => "waiter-cancelled",
        _ if survives => "start-returned",
        _ => "default-ready",
    };
    loop {
        let remaining = marker_deadline.saturating_duration_since(Instant::now());
        match child.lines.recv_timeout(remaining) {
            Ok(line) if line.contains(expected) => break,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                let output = child.finish(Instant::now());
                panic!(
                    "child did not publish {expected}; status={}; timed_out={}; stdout={:?}; stderr={:?}",
                    output.status, output.timed_out, output.stdout, output.stderr
                );
            }
        }
    }
    assert!(
        Command::new("/bin/kill")
            .args([format!("-{signal}"), child.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let release_error = survives
        .then(|| child.child.stdin.take().unwrap().write_all(b"x").err())
        .flatten();
    let output = child.finish(Instant::now() + Duration::from_secs(5));
    assert!(
        !output.timed_out,
        "signal child timed out: mode={mode} signal={signal}"
    );
    if survives {
        assert!(
            output.status.success(),
            "{}; release={release_error:?}; stdout={:?}; stderr={:?}",
            output.status,
            output.stdout,
            output.stderr
        );
    } else {
        let expected_signal = if signal == "TERM" { 15 } else { 2 };
        assert_eq!(
            output.status.signal(),
            Some(expected_signal),
            "{}; {:?}; {:?}",
            output.status,
            output.stdout,
            output.stderr
        );
    }
    output.stdout
}

fn run_child_to_completion(mode: &str) -> String {
    run_child_to_completion_with_limit(mode, Duration::from_secs(5))
}

fn run_child_to_completion_with_limit(mode: &str, limit: Duration) -> String {
    let output = CapturedChild::spawn(mode, false).finish(Instant::now() + limit);
    assert!(!output.timed_out, "child timed out: mode={mode}");
    assert!(
        output.status.success(),
        "{}; stdout={:?}; stderr={:?}",
        output.status,
        output.stdout,
        output.stderr
    );
    output.stdout
}

struct CapturedChild {
    child: Child,
    lines: Receiver<String>,
    stdout: JoinHandle<io::Result<String>>,
    stderr: JoinHandle<io::Result<String>>,
}

struct CapturedOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl CapturedChild {
    fn spawn(mode: &str, piped_stdin: bool) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "configured_signals_cover_start_return_running_and_explicit_opt_out",
                "--nocapture",
            ])
            .env(CHILD_MODE, mode)
            .stdin(if piped_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (line_tx, lines) = mpsc::channel();
        let stdout = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut output = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line)? == 0 {
                    break;
                }
                output.push_str(&line);
                let _ = line_tx.send(line);
            }
            Ok(output)
        });
        let stderr = std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut output = String::new();
            reader.read_to_string(&mut output)?;
            Ok(output)
        });
        Self {
            child,
            lines,
            stdout,
            stderr,
        }
    }

    fn finish(mut self, deadline: Instant) -> CapturedOutput {
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = self.child.kill();
                break self.child.wait().unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = join_capture("stdout", self.stdout);
        let stderr = join_capture("stderr", self.stderr);
        CapturedOutput {
            status,
            stdout,
            stderr,
            timed_out,
        }
    }
}

fn join_capture(name: &str, capture: JoinHandle<io::Result<String>>) -> String {
    match capture.join() {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => format!("<{name} capture failed: {error}>"),
        Err(_) => format!("<{name} capture panicked>"),
    }
}
