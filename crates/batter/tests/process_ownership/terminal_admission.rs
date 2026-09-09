use super::finite_supervisor;
use batter::lifecycle::{ProcessAdmissionError, ProcessHandle, Readiness};
use std::{
    convert::Infallible,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn assert_admission_closed(process: &ProcessHandle) {
    let called = Arc::new(AtomicBool::new(false));
    let in_factory = called.clone();
    let error = process
        .try_spawn("after-closure", move |_| {
            in_factory.store(true, Ordering::SeqCst);
            async { Ok::<_, Infallible>(()) }
        })
        .err()
        .expect("terminal admission must reject work");
    assert!(matches!(error, ProcessAdmissionError::Closed), "{error:?}");
    assert!(!called.load(Ordering::SeqCst));
}

#[test]
fn admission_is_closed_when_shutdown_precedes_startup() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    handle.request();
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(!handle.operation_token().is_cancelled());
    assert_admission_closed(&process);
}

#[test]
fn admission_is_closed_after_unpolled_driver_drop() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    drop(supervisor.run_until(pending()));
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(handle.operation_token().is_cancelled());
    assert_admission_closed(&process);
}

#[test]
fn admission_is_closed_after_unstarted_supervisor_drop() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    drop(supervisor);
    assert_admission_closed(&process);
}

#[tokio::test]
async fn admission_is_closed_after_driver_abort_before_first_poll() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    // This current-thread runtime cannot poll the driver before the abort.
    let task = tokio::spawn(supervisor.run_until(pending()));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(handle.operation_token().is_cancelled());
    assert_admission_closed(&process);
}

#[tokio::test]
async fn admission_is_closed_after_driver_abort_after_startup() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    let task = tokio::spawn(supervisor.run_until(pending()));
    handle.wait_ready().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_admission_closed(&process);
}

#[tokio::test]
async fn admission_is_closed_after_completed_shutdown() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    let running = supervisor.start();
    handle.wait_ready().await.unwrap();
    assert!(running.shutdown().await.unwrap().is_success());
    assert_eq!(handle.readiness(), Readiness::Stopped);
    assert_admission_closed(&process);
}
