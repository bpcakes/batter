//! External-style completion controls shared with the temporary facade consumer.
#![deny(unused_must_use)]

use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{
        Readiness, RunningSupervisor, ShutdownBudget, ShutdownFailure, ShutdownSuccess, Supervisor,
        SupervisorObserver,
    },
};
use std::{
    future::pending,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[allow(dead_code)]
fn direct_success(_: batter_core::lifecycle::ShutdownSuccess) {}
const _: fn(ShutdownSuccess) = direct_success;

fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    ShutdownBudget::new(second, second, second, cleanup).unwrap()
}

fn component(supervisor: &mut Supervisor) {
    supervisor
        .register("component", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
}

async fn ready_owner(fail_cleanup: bool) -> RunningSupervisor {
    let mut supervisor = Supervisor::new(budget());
    component(&mut supervisor);
    if fail_cleanup {
        supervisor
            .on_cleanup("resource", || async {
                Err(std::io::Error::other("private-cleanup-marker").into())
            })
            .unwrap();
    } else {
        supervisor
            .on_cleanup("resource", || async { Ok(()) })
            .unwrap();
    }
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    running
}

async fn boxed_boundary(running: &RunningSupervisor) -> Result<(), BoxError> {
    running.wait_checked().await?;
    Ok(())
}

async fn anyhow_boundary(running: &RunningSupervisor) -> anyhow::Result<()> {
    running.wait_checked().await?;
    Ok(())
}

fn process_exit(result: anyhow::Result<()>) -> ExitCode {
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[tokio::test]
async fn ordinary_boundaries_and_exit_code_preserve_failed_cleanup() {
    let running = ready_owner(false).await;
    running.handle().request();
    assert_eq!(
        process_exit(anyhow_boundary(&running).await),
        ExitCode::SUCCESS
    );
    boxed_boundary(&running).await.unwrap();

    let running = ready_owner(true).await;
    running.handle().request();
    let error = anyhow_boundary(&running).await.unwrap_err();
    let ShutdownFailure::Report(report) = error.downcast_ref::<ShutdownFailure>().unwrap() else {
        panic!("driven cleanup failure must retain a report")
    };
    let repeated = running.wait_checked().await.unwrap_err();
    let ShutdownFailure::Report(repeated) = repeated else {
        panic!("repeat observation must retain report failure")
    };
    assert!(std::ptr::eq(&**report, &*repeated));
    let boxed_error = boxed_boundary(&running).await.unwrap_err();
    let ShutdownFailure::Report(boxed_report) =
        boxed_error.downcast_ref::<ShutdownFailure>().unwrap()
    else {
        panic!("boxed boundary must preserve the failed report")
    };
    assert!(std::ptr::eq(&**report, &**boxed_report));
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "private-cleanup-marker"
    );
    assert_eq!(process_exit(Err(error)), ExitCode::FAILURE);
}

struct PanickingDrop;
impl Drop for PanickingDrop {
    fn drop(&mut self) {
        panic!("coordinator-capture-marker")
    }
}

#[tokio::test(start_paused = true)]
async fn coordinator_failure_reaches_anyhow_with_original_join_error() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", |_| pending::<Result<_, BoxError>>())
        .unwrap();
    let capture = PanickingDrop;
    supervisor
        .on_cleanup("resource", move || async move {
            drop(capture);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    let observer = running.observer();
    drop(running);
    let mut summaries = Vec::new();
    let error = summarize_then_propagate(&observer, &mut summaries)
        .await
        .unwrap_err();
    assert!(summaries.is_empty());
    let failure = error.downcast_ref::<ShutdownFailure>().unwrap();
    let ShutdownFailure::Coordinator(join) = failure else {
        panic!("coordinator panic must retain JoinError")
    };
    assert!(join.is_panic());
    assert!(!format!("{failure:?}").contains("coordinator-capture-marker"));
    assert!(!format!("{failure}").contains("coordinator-capture-marker"));
    // The native source is intentionally retained. An anyhow renderer that
    // traverses sources can print the original panic payload.
    assert!(format!("{error:?}").contains("coordinator-capture-marker"));
    assert!(format!("{error:#}").contains("coordinator-capture-marker"));
    let ShutdownFailure::Coordinator(repeated) = observer.wait_checked().await.unwrap_err() else {
        panic!("repeat observation must retain coordinator failure")
    };
    assert!(Arc::ptr_eq(join, &repeated));
    assert_eq!(process_exit(Err(error)), ExitCode::FAILURE);
}

fn report_summary(report: &batter::lifecycle::SharedShutdownReport) -> String {
    let failed = report
        .cleanup
        .records
        .iter()
        .filter(|record| record.outcome != CleanupOutcome::Succeeded)
        .count();
    format!("tasks={} failed_cleanup={failed}", report.tasks.len())
}

async fn summarize_then_propagate(
    observer: &SupervisorObserver,
    summaries: &mut Vec<String>,
) -> anyhow::Result<()> {
    let completion = observer.wait_checked().await;
    record_completion_summary(&completion, summaries);
    completion?;
    Ok(())
}

fn record_completion_summary(
    completion: &Result<ShutdownSuccess, ShutdownFailure>,
    summaries: &mut Vec<String>,
) {
    match &completion {
        Ok(success) => summaries.push(report_summary(success.report())),
        Err(ShutdownFailure::Report(report)) => summaries.push(report_summary(report)),
        Err(ShutdownFailure::Coordinator(_)) => {}
    }
}

#[tokio::test]
async fn one_checked_observation_summarizes_success_and_failure_before_propagation() {
    for fail_cleanup in [false, true] {
        let running = ready_owner(fail_cleanup).await;
        let observer = running.observer();
        running.handle().request();
        let mut summaries = Vec::new();
        let outcome = summarize_then_propagate(&observer, &mut summaries).await;
        assert_eq!(
            summaries,
            [format!("tasks=1 failed_cleanup={}", u8::from(fail_cleanup))]
        );
        assert!(!summaries[0].contains("private-cleanup-marker"));
        if fail_cleanup {
            assert!(
                outcome
                    .unwrap_err()
                    .downcast_ref::<ShutdownFailure>()
                    .is_some()
            );
        } else {
            outcome.unwrap();
        }
    }
}

enum WorkerFailure {
    Shutdown(ShutdownFailure),
    Rejected {
        state: Readiness,
        shutdown: Option<ShutdownFailure>,
    },
    Deadline {
        shutdown: Option<ShutdownFailure>,
    },
}

async fn worker_completion(
    running: &RunningSupervisor,
    readiness_deadline: Duration,
    summaries: &mut Vec<String>,
) -> Result<(), WorkerFailure> {
    match tokio::time::timeout(readiness_deadline, running.status().wait_ready()).await {
        Ok(Ok(())) => {
            let completion = running.wait_checked().await;
            record_completion_summary(&completion, summaries);
            completion.map(|_| ()).map_err(WorkerFailure::Shutdown)
        }
        Ok(Err(state)) => {
            let shutdown = running.wait_checked().await;
            record_completion_summary(&shutdown, summaries);
            Err(WorkerFailure::Rejected {
                state,
                shutdown: shutdown.err(),
            })
        }
        Err(_) => {
            let shutdown = running.shutdown_checked().await;
            record_completion_summary(&shutdown, summaries);
            Err(WorkerFailure::Deadline {
                shutdown: shutdown.err(),
            })
        }
    }
}

#[tokio::test]
async fn worker_ready_completion_awaits_cleanup() {
    let mut supervisor = Supervisor::new(budget());
    component(&mut supervisor);
    let cleaned = Arc::new(AtomicBool::new(false));
    let flag = cleaned.clone();
    supervisor
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let worker = tokio::spawn({
        let running = running.clone();
        async move {
            let mut summaries = Vec::new();
            let result = worker_completion(&running, Duration::from_secs(1), &mut summaries).await;
            (result, summaries)
        }
    });
    tokio::task::yield_now().await;
    running.handle().request();
    let (result, summaries) = worker.await.unwrap();
    assert!(result.is_ok());
    assert_eq!(summaries, ["tasks=1 failed_cleanup=0"]);
    assert!(cleaned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn worker_readiness_rejection_retains_shutdown_failure() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", |_startup| async {
            Err(std::io::Error::other("component-marker").into())
        })
        .unwrap();
    let cleaned = Arc::new(AtomicBool::new(false));
    let flag = cleaned.clone();
    supervisor
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Err(std::io::Error::other("rejection-cleanup-marker").into())
        })
        .unwrap();
    let running = supervisor.start();
    let mut summaries = Vec::new();
    let failure = worker_completion(&running, Duration::from_secs(1), &mut summaries)
        .await
        .unwrap_err();
    assert!(cleaned.load(Ordering::SeqCst));
    assert_eq!(summaries, ["tasks=1 failed_cleanup=1"]);
    assert!(!summaries[0].contains("rejection-cleanup-marker"));
    let WorkerFailure::Rejected {
        state: Readiness::Draining | Readiness::Stopped,
        shutdown: Some(ShutdownFailure::Report(report)),
    } = failure
    else {
        panic!("readiness rejection and failed shutdown must both remain")
    };
    assert_eq!(
        report.tasks[0].error.as_ref().unwrap().to_string(),
        "component-marker"
    );
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "rejection-cleanup-marker"
    );
}

#[tokio::test]
async fn worker_readiness_rejection_survives_clean_shutdown() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", |startup| async move {
            startup.shutdown().draining().await;
            Ok(startup.abandon())
        })
        .unwrap();
    let cleaned = Arc::new(AtomicBool::new(false));
    let flag = cleaned.clone();
    supervisor
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().request();
    let mut summaries = Vec::new();
    let failure = worker_completion(&running, Duration::from_secs(1), &mut summaries)
        .await
        .unwrap_err();
    assert!(cleaned.load(Ordering::SeqCst));
    assert_eq!(summaries, ["tasks=1 failed_cleanup=0"]);
    assert!(matches!(
        failure,
        WorkerFailure::Rejected {
            state: Readiness::Draining | Readiness::Stopped,
            shutdown: None,
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn worker_readiness_deadline_requests_shutdown_and_retains_outcomes() {
    for fail_cleanup in [false, true] {
        let mut supervisor = Supervisor::new(budget());
        supervisor
            .register("component", |startup| async move {
                let shutdown = startup.shutdown().clone();
                shutdown.draining().await;
                Ok(startup.abandon())
            })
            .unwrap();
        let finalized = Arc::new(AtomicBool::new(false));
        let flag = finalized.clone();
        if fail_cleanup {
            supervisor
                .on_cleanup("resource", move || async move {
                    flag.store(true, Ordering::SeqCst);
                    Err(std::io::Error::other("deadline-cleanup-marker").into())
                })
                .unwrap();
        } else {
            supervisor
                .on_cleanup("resource", move || async move {
                    flag.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
        }
        let running = supervisor.start();
        let mut summaries = Vec::new();
        let failure = worker_completion(&running, Duration::from_millis(10), &mut summaries)
            .await
            .unwrap_err();
        assert_eq!(
            summaries,
            [format!("tasks=1 failed_cleanup={}", u8::from(fail_cleanup))]
        );
        assert!(!summaries[0].contains("deadline-cleanup-marker"));
        assert!(finalized.load(Ordering::SeqCst));
        assert_eq!(running.status().readiness(), Readiness::Stopped);
        match (fail_cleanup, failure) {
            (false, WorkerFailure::Deadline { shutdown: None }) => {}
            (
                true,
                WorkerFailure::Deadline {
                    shutdown: Some(ShutdownFailure::Report(report)),
                },
            ) => assert_eq!(
                report.cleanup.records[0]
                    .error
                    .as_ref()
                    .unwrap()
                    .to_string(),
                "deadline-cleanup-marker"
            ),
            _ => panic!("deadline and shutdown outcomes must remain distinct"),
        }
    }
}

#[tokio::test]
async fn worker_ready_path_preserves_failed_cleanup() {
    let running = ready_owner(true).await;
    let worker = tokio::spawn({
        let running = running.clone();
        async move {
            let mut summaries = Vec::new();
            let result = worker_completion(&running, Duration::from_secs(1), &mut summaries).await;
            (result, summaries)
        }
    });
    tokio::task::yield_now().await;
    running.handle().request();
    let (result, summaries) = worker.await.unwrap();
    assert_eq!(summaries, ["tasks=1 failed_cleanup=1"]);
    let WorkerFailure::Shutdown(ShutdownFailure::Report(report)) = result.unwrap_err() else {
        panic!("ready worker must propagate failed cleanup")
    };
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
}
