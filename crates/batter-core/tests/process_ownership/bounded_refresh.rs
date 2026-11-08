//! The process-task migration must retain fatal work at the task boundary.
use super::*;
use batter_core::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity, BulkheadError},
    operation::{Interruption, OperationError, OperationOwner},
};

type RefreshTaskResult = Result<Result<(), BulkheadError<Infallible>>, Fatal<std::io::Error>>;

#[tokio::test]
async fn unobserved_task_failure_initiates_drain_and_retains_original_source() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let receipt = process
        .try_spawn("failed-task", |_| async {
            Err::<(), _>(Fatal(std::io::Error::other("private failure cause")))
        })
        .unwrap();
    drop(receipt);
    let report = running.wait().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("failed-task"));
    assert!(!report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    let source = report.tasks[0]
        .error
        .as_ref()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(source.to_string(), "private failure cause");
    assert!(matches!(
        process.try_spawn("after-failure", |_| async {
            Ok::<_, Fatal<Infallible>>(())
        }),
        Err(ProcessAdmissionError::Closed)
    ));
}

// Same exhaustive mapping as the CHANGELOG's process-task migration. Skips are
// application-selected nonfatal values, while retained fatal work stays fatal.
fn refresh_task_result(
    result: Result<(), BulkheadError<Fatal<std::io::Error>>>,
) -> RefreshTaskResult {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(BulkheadError::Operation(OperationError::Failed(fatal))) => Err(fatal),
        Err(BulkheadError::NotAdmitted(reason)) => {
            Ok(Err(BulkheadError::<Infallible>::NotAdmitted(reason)))
        }
        Err(BulkheadError::Operation(OperationError::Interrupted(reason))) => Ok(Err(
            BulkheadError::Operation(OperationError::Interrupted(reason)),
        )),
    }
}

#[tokio::test(start_paused = true)]
async fn bounded_fatal_refresh_drains_without_a_receipt_and_retains_its_source() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let receipt = process
        .try_spawn("refresh", move |_| async move {
            refresh_task_result(
                bulkhead
                    .run(owner.context(), Admission::Reject, "refresh", |_| async {
                        Err::<(), _>(Fatal(std::io::Error::other("private refresh failure")))
                    })
                    .await,
            )
        })
        .unwrap();
    drop(receipt);
    let report = tokio::time::timeout(Duration::from_secs(1), running.wait())
        .await
        .expect("fatal bounded work must initiate drain without an observer")
        .unwrap();
    assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("refresh"));
    assert!(!report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    let source = report.tasks[0]
        .error
        .as_ref()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(source.to_string(), "private refresh failure");
    assert!(matches!(
        process.try_spawn("after-failure", |_| async {
            Ok::<_, Fatal<Infallible>>(())
        }),
        Err(ProcessAdmissionError::Closed)
    ));
}

#[tokio::test]
async fn refused_refresh_is_a_nonfatal_value_and_never_invokes_work() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    bulkhead.close();
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let outcome = process
        .try_spawn("refresh", move |_| async move {
            refresh_task_result(
                bulkhead
                    .run(owner.context(), Admission::Reject, "refresh", |_| {
                        panic!("refused refresh cannot run");
                        #[expect(unreachable_code, reason = "fix the future result type")]
                        pending::<Result<(), Fatal<std::io::Error>>>()
                    })
                    .await,
            )
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        Err(BulkheadError::NotAdmitted(AdmissionError::Closed))
    ));
    assert!(!running.status().is_draining());
    let report = running.shutdown().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
}

#[tokio::test]
async fn interrupted_refresh_is_an_explicit_nonfatal_receipt_value() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let outcome = process
        .try_spawn("refresh", move |_| async move {
            refresh_task_result(
                bulkhead
                    .run(owner.context(), Admission::Reject, "refresh", |_| async {
                        owner.cancel();
                        pending::<Result<(), Fatal<std::io::Error>>>().await
                    })
                    .await,
            )
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        Err(BulkheadError::Operation(OperationError::Interrupted(
            Interruption::Cancelled
        )))
    ));
    assert!(!running.status().is_draining());
    let report = running.shutdown().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
}
