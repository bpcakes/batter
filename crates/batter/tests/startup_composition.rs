use batter::{
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{RunningSupervisor, ShutdownBudget, ShutdownCause, Supervisor},
    operation::OperationContext,
    startup::Startup,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn cleanup_budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(second, second, second).unwrap()
}

fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(second, second, second, cleanup_budget()).unwrap()
}

async fn initialize(supervisor: Supervisor, capacity: Arc<Semaphore>) -> RunningSupervisor {
    let mut starting = Startup::new(
        supervisor,
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                let slot = scope.supervisor().reserve_cleanup("resource")?;
                let permit = capacity.acquire_owned().await.unwrap();
                slot.register(move || async move {
                    drop(permit);
                    Ok(())
                });
                Ok::<_, batter::RegistrationError>(())
            })
        },
    )
    .start();
    starting.wait().await.unwrap()
}

#[tokio::test]
async fn successful_startup_without_components_or_capacity_reports_empty_after_cleanup() {
    let capacity = Arc::new(Semaphore::new(1));
    let running = initialize(Supervisor::new(budget()), capacity.clone()).await;
    // Successful initialization is not a successful running-service report.
    let report = running.shutdown().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::EmptySupervisor);
    assert!(!report.is_success());
    assert!(report.tasks.is_empty());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "resource");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(report.cleanup.skipped.is_empty());
    assert_eq!(capacity.available_permits(), 1);
}

#[tokio::test]
async fn finite_work_without_critical_components_has_a_successful_running_lifetime() {
    let capacity = Arc::new(Semaphore::new(1));
    let supervisor = Supervisor::with_process_capacity(budget(), 1).unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = initialize(supervisor, capacity.clone()).await;
    running.handle().wait_ready().await.unwrap();
    assert_eq!(capacity.available_permits(), 0);
    let receipt = process
        .try_spawn("calculation", |_| async { Ok::<_, Infallible>(42) })
        .unwrap();
    assert_eq!(receipt.wait().await.unwrap(), 42);
    let report = running.shutdown().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_eq!(capacity.available_permits(), 1);
}
