//! Periodic report retention through the existing OS-process watchdog.

use super::fixture::{TaskLifetime, block_forever, budget, emit};
use batter_core::{
    cleanup::SkipReason,
    lifecycle::{Fatal, ProcessCapacity, ShutdownCause, ShutdownReport, Supervisor, TaskOutcome},
    periodic::{
        PeriodicCompletion, PeriodicFailure, PeriodicPolicy, PeriodicReader, PeriodicShutdown,
        PeriodicStartup, register_periodic_in,
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
#[error("retained maintenance cause")]
struct MaintenanceFailure(u32);

pub(super) fn run(scenario: &str) {
    let finite_namesake = scenario == "periodic-finite-namesake";
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    let mut supervisor =
        Supervisor::with_process_capacity(budget(), ProcessCapacity::new(1).unwrap());
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    let live = Arc::new(AtomicBool::new(false));
    let cleanup_called = Arc::new(AtomicBool::new(false));
    let cleanup = cleanup_called.clone();
    supervisor
        .on_cleanup("dependency", move || {
            cleanup.store(true, Ordering::SeqCst);
            emit("cleanup-invoked");
            async { Ok(()) }
        })
        .unwrap();
    let (entered, entry) = std::sync::mpsc::channel();
    let reader = register(
        &mut supervisor,
        finite_namesake,
        entered.clone(),
        live.clone(),
    );
    // Wake the coordinator from outside the worker about to park. This avoids
    // stranding it in that worker's non-stealable Tokio LIFO slot.
    let request = std::thread::spawn(move || {
        entry.recv().unwrap();
        handle.request();
        emit("drain-requested");
    });
    let report = runtime.block_on(async {
        let running = supervisor.start();
        if finite_namesake {
            running.status().wait_ready().await.unwrap();
            while reader.snapshot().recoverable_failures == 0 {
                tokio::task::yield_now().await;
            }
            let active = live.clone();
            let receipt = process
                .try_spawn("maintenance", move |_| async move {
                    let _lifetime = TaskLifetime(active.clone());
                    active.store(true, Ordering::SeqCst);
                    emit("task-entered");
                    entered.send(()).unwrap();
                    block_forever();
                    #[allow(unreachable_code)]
                    Ok::<_, Fatal<std::io::Error>>(())
                })
                .unwrap();
            drop(receipt);
        }
        running.wait().await.unwrap()
    });
    request.join().unwrap();
    assert!(live.load(Ordering::SeqCst));
    assert!(!cleanup_called.load(Ordering::SeqCst));
    assert_unjoined_shutdown(&report);
    assert_periodic_history(&report, finite_namesake);
    emit("report-unjoined-cleanup-skipped");
    emit("runtime-drop-started");
    drop(runtime);
    emit("runtime-dropped");
}

fn register(
    supervisor: &mut Supervisor,
    finite_namesake: bool,
    notify: std::sync::mpsc::Sender<()>,
    active: Arc<AtomicBool>,
) -> PeriodicReader {
    let mut invocation = 0;
    register_periodic_in(
        supervisor,
        "maintenance",
        PeriodicPolicy::new(
            Duration::from_millis(20),
            Duration::from_secs(1),
            PeriodicStartup::immediate(),
            if finite_namesake {
                PeriodicShutdown::StopAtDrain
            } else {
                PeriodicShutdown::SupportThroughDrain
            },
        )
        .unwrap(),
        move |_| {
            invocation += 1;
            let attempt = invocation;
            let (notify, active) = (notify.clone(), active.clone());
            async move {
                if !finite_namesake && attempt == 2 {
                    let _lifetime = TaskLifetime(active.clone());
                    active.store(true, Ordering::SeqCst);
                    emit("task-entered");
                    notify.send(()).unwrap();
                    block_forever();
                }
                Err(PeriodicFailure::Recoverable(MaintenanceFailure(7)))
            }
        },
    )
    .unwrap()
}

fn assert_unjoined_shutdown(report: &ShutdownReport) {
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert!(report.forced_cancellation);
    assert!(!report.is_success());
    assert_eq!(report.abort_requested, ["maintenance"]);
    assert_eq!(report.unjoined, ["maintenance"]);
    assert!(report.cleanup.records.is_empty());
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
}

fn assert_periodic_history(report: &ShutdownReport, finite_namesake: bool) {
    assert_eq!(report.periodic.len(), 1);
    let summary = &report.periodic[0].summary;
    let failure = summary.first_failure.as_ref().unwrap();
    assert_eq!(failure.invocation, 1);
    assert_eq!(
        failure
            .error
            .downcast_ref::<MaintenanceFailure>()
            .unwrap()
            .0,
        7
    );
    if finite_namesake {
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].name, "maintenance");
        assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
        assert_eq!(summary.completion, PeriodicCompletion::Stopped);
        emit("periodic-history-stopped");
    } else {
        assert!(report.tasks.is_empty());
        assert_eq!(summary.invocations, 2);
        assert_eq!(summary.recoverable_failures, 1);
        assert_eq!(summary.completion, PeriodicCompletion::Pending);
        emit("periodic-history-pending");
    }
}
