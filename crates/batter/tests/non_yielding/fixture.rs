use batter::{
    cleanup::{CleanupBudget, CleanupOutcome, SkipReason},
    lifecycle::{
        Readiness, ShutdownBudget, ShutdownCause, ShutdownReport, Supervisor, TaskOutcome,
    },
};
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const BLOCKED_OBSERVATION: Duration = Duration::from_secs(2);

fn emit(event: &str) {
    let mut output = std::io::stdout().lock();
    // libtest can print its test-name prefix without a trailing newline.
    writeln!(output, "\nbatter-fixture:{event}").unwrap();
    output.flush().unwrap();
}

fn block_forever() -> ! {
    // OS parking releases the CPU, but this Tokio task's poll never returns.
    // Repeat on spurious unparks; no self-release can rescue a broken watchdog.
    loop {
        std::thread::park();
    }
}

struct TaskLifetime(Arc<AtomicBool>);

impl Drop for TaskLifetime {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
        emit("task-dropped");
    }
}

fn budget() -> ShutdownBudget {
    ShutdownBudget::new(
        Duration::from_millis(100),
        Duration::from_millis(250),
        Duration::from_millis(250),
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(100),
        )
        .unwrap(),
    )
    .unwrap()
}

fn supervisor(
    cooperative: bool,
    live: Arc<AtomicBool>,
    cleanup_called: Arc<AtomicBool>,
    entered: std::sync::mpsc::Sender<()>,
) -> Supervisor {
    let mut supervisor = Supervisor::new(budget());
    let handle = supervisor.handle();
    let active = live.clone();
    supervisor
        .register("worker", move |signal| async move {
            let lifetime = TaskLifetime(active.clone());
            active.store(true, Ordering::SeqCst);
            assert!(signal.mark_started());
            assert_eq!(handle.readiness(), Readiness::Ready);
            emit("task-entered");
            entered.send(()).unwrap();
            if !cooperative {
                block_forever();
            }
            signal.cancelled().await;
            assert!(signal.is_draining());
            emit("cancellation-observed");
            drop(lifetime);
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("dependency", move || {
            // Observe factory invocation, including a factory whose future
            // might never be polled. Skipping must not call this at all.
            cleanup_called.store(true, Ordering::SeqCst);
            emit("cleanup-invoked");
            async move {
                assert!(!live.load(Ordering::SeqCst));
                Ok(())
            }
        })
        .unwrap();
    supervisor.handle().mark_ready();
    supervisor
}

fn assert_clean(report: &ShutdownReport) {
    assert!(report.is_success());
    assert!(report.all_direct_tasks_joined());
    assert!(report.abort_requested.is_empty());
    assert!(report.unjoined.is_empty());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].name, "worker");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert!(report.tasks[0].error.is_none());
    assert!(report.cleanup.skipped.is_empty());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "dependency");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
}

fn assert_unjoined(report: &ShutdownReport) {
    assert!(!report.is_success());
    assert!(!report.all_direct_tasks_joined());
    assert_eq!(report.abort_requested, ["worker"]);
    assert_eq!(report.unjoined, ["worker"]);
    // Abort was requested, but no termination was observed. An invented
    // TaskOutcome::Aborted here would incorrectly claim a joined direct future.
    assert!(report.tasks.is_empty());
    assert!(!report.cleanup.is_success());
    assert!(report.cleanup.records.is_empty());
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].name, "dependency");
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
}

fn invalid_output(scenario: &str) {
    emit("evidence-ready");
    if scenario == "event-overflow" {
        for id in 0..128 {
            emit(&format!("event-{id}"));
        }
    }
    if scenario != "unexpected-cleanup" {
        // More than the diagnostic preview and typical pipe capacity.
        // The forbidden event comes AFTER the preview, then exit 0.
        std::io::stdout().write_all(&[b'x'; 128 * 1024]).unwrap();
        println!();
    }
    emit("cleanup-invoked");
}

pub fn run(scenario: &str) {
    emit("fixture-entered");
    match scenario {
        "stuck-before-runtime" => block_forever(),
        "fixture-failure" => panic!("deliberate fixture failure"),
        "exit-success" => return,
        "exit-failure" => std::process::exit(7),
        "unexpected-cleanup" | "output-overflow" | "event-overflow" => {
            invalid_output(scenario);
            return;
        }
        "cooperative" | "non-yielding" | "blocked-current-thread" | "blocked-request-failure" => {}
        _ => panic!("unknown child scenario"),
    }
    let cooperative = scenario == "cooperative";
    let current_thread = matches!(
        scenario,
        "blocked-current-thread" | "blocked-request-failure"
    );
    let request_failure = scenario == "blocked-request-failure";
    let runtime = if current_thread {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
    }
    .unwrap();
    let live = Arc::new(AtomicBool::new(false));
    let cleanup_called = Arc::new(AtomicBool::new(false));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let supervisor = supervisor(
        cooperative,
        live.clone(),
        cleanup_called.clone(),
        entered_tx,
    );
    let handle = supervisor.handle();
    let requesting = handle.clone();
    // Wake the driver externally after task entry. Waking it from the worker
    // about to block can strand it in Tokio's non-stealable LIFO slot, even
    // when a second worker is available. This thread uses no Tokio scheduling.
    let request = std::thread::spawn(move || {
        entered_rx.recv().unwrap();
        requesting.request();
        emit("drain-requested");
        if current_thread {
            // Prove time really passed after task entry, even with slow child
            // startup. This exceeds both the armed timer and all phase budgets.
            assert!(BLOCKED_OBSERVATION > budget().total_allowance());
            std::thread::sleep(BLOCKED_OBSERVATION);
            emit("blocked-observation-elapsed");
        }
        assert!(!request_failure, "deliberate request failure");
    });
    let report = runtime.block_on(async {
        if current_thread {
            // Poll the timer once BEFORE starting the driver, so its failure to
            // complete cannot be explained by a timer that was never armed.
            let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async {
                let timer = tokio::time::sleep(Duration::from_secs(1));
                tokio::pin!(timer);
                std::future::poll_fn(|cx| {
                    assert!(timer.as_mut().poll(cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
                emit("timer-armed");
                armed_tx.send(()).unwrap();
                timer.await;
                emit("timer-completed");
            });
            armed_rx.await.unwrap();
        }
        supervisor.start().wait().await.unwrap()
    });
    request.join().unwrap();
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert!(report.forced_cancellation);
    assert_eq!(report.completed_process_tasks, 0);
    assert_eq!(handle.readiness(), Readiness::Stopped);
    if cooperative {
        assert_clean(&report);
        assert!(!live.load(Ordering::SeqCst));
        assert!(cleanup_called.load(Ordering::SeqCst));
        emit("report-clean");
    } else {
        assert_unjoined(&report);
        assert!(live.load(Ordering::SeqCst));
        assert!(!cleanup_called.load(Ordering::SeqCst));
        emit("report-unjoined-cleanup-skipped");
    }
    emit("runtime-drop-started");
    // Intentionally do not leak the runtime or exit(0). A non-yielding task
    // prevents native runtime destruction even after the report is available.
    drop(runtime);
    emit("runtime-dropped");
}
