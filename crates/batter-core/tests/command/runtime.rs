use super::*;

#[test]
fn published_report_survives_its_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let observer = runtime.block_on(async {
        let command = Command::new(context(), budget(), |_| {
            Box::pin(async { Ok::<_, Infallible>(42) })
        })
        .start();
        assert!(command.wait().await.unwrap().is_success());
        command.observer()
    });
    drop(runtime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let report = runtime.block_on(observer.wait()).unwrap();
    assert_eq!(*report.work.as_ref().unwrap(), 42);
}

#[test]
fn runtime_death_before_publication_cannot_report_completed_cleanup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (command, observer) = runtime.block_on(async {
        let (started, starting) = oneshot::channel();
        let command = Command::new(context(), budget(), |_| {
            Box::pin(async move {
                started.send(()).unwrap();
                pending::<()>().await;
                Ok::<_, Infallible>(())
            })
        })
        .start();
        starting.await.unwrap();
        let observer = command.observer();
        (command, observer)
    });
    drop(runtime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let observed =
        runtime.block_on(async move { tokio::spawn(async move { observer.wait().await }).await });
    assert!(observed.unwrap_err().is_panic());
    drop(command);
}

#[tokio::test(start_paused = true)]
async fn final_poll_cancellation_keeps_returned_value_without_false_success() {
    let command = Command::new(context(), budget(), |scope| {
        Box::pin(async move {
            scope.cancel_work();
            Ok::<_, Infallible>(42)
        })
    })
    .start();
    let report = command.wait().await.unwrap();
    assert_eq!(*report.work.as_ref().unwrap(), 42);
    assert_eq!(
        report.interruption_after_work,
        Some(Interruption::Cancelled)
    );
    assert!(!report.is_success());
}

fn current_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Yield until every task spawned on this runtime has completed and released
/// its slot. The iteration cap is a failure guard, not the ordering mechanism.
async fn settle_spawned_tasks() {
    let metrics = tokio::runtime::Handle::current().metrics();
    for _ in 0..10_000 {
        if metrics.num_alive_tasks() == 0 {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("spawned tasks did not complete");
}

#[test]
fn owner_of_work_lost_with_its_runtime_reports_coordinator_failure_without_panicking() {
    let runtime = current_thread();
    let owner = runtime.block_on(async {
        let (started, starting) = oneshot::channel();
        let owner = Command::new(context(), budget(), move |scope| {
            Box::pin(async move {
                scope
                    .reserve_cleanup("resource")?
                    .register(|| async { Ok(()) });
                started.send(()).unwrap();
                pending::<()>().await;
                Ok::<u32, batter_core::RegistrationError>(42)
            })
        })
        .start_without_signals();
        starting.await.unwrap();
        owner
    });
    drop(runtime);
    let finished = current_thread().block_on(owner.finish());
    let Err(batter_core::command::CommandFailure::Coordinator(error)) = finished else {
        panic!("runtime death must not claim a complete report")
    };
    assert!(error.is_cancelled(), "{error}");
    assert!(!error.is_panic(), "{error}");
}

#[test]
fn owner_of_a_command_completed_before_runtime_death_still_receives_its_value() {
    let runtime = current_thread();
    let owner = runtime.block_on(async {
        let (finalized, finalizing) = oneshot::channel();
        let owner = Command::new(context(), budget(), move |scope| {
            Box::pin(async move {
                scope
                    .reserve_cleanup("resource")?
                    .register(move || async move {
                        finalized.send(()).unwrap();
                        Ok(())
                    });
                Ok::<_, batter_core::RegistrationError>(String::from("completed value"))
            })
        })
        .start_without_signals();
        finalizing.await.unwrap();
        // The coordinator still assembles its report after the last hook;
        // observe its task's completion without consuming the owner.
        settle_spawned_tasks().await;
        owner
    });
    drop(runtime);
    let success = current_thread()
        .block_on(owner.finish())
        .expect("the stored report survives its runtime");
    assert_eq!(success.cleanup().records.len(), 1);
    assert_eq!(
        success.cleanup().records[0].outcome,
        CleanupOutcome::Succeeded
    );
    assert_eq!(success.into_value(), "completed value");
}

#[test]
fn owner_on_a_runtime_without_its_time_driver_reports_a_coordinator_panic() {
    // The coordinator bounds work with the runtime's timer; without it the
    // coordinator itself panics, and `finish` returns that, never a panic.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        Command::new(context(), budget(), |_| {
            Box::pin(async { Ok::<_, Infallible>(42) })
        })
        .start_without_signals()
        .finish()
        .await
    });
    match result {
        Err(batter_core::command::CommandFailure::Coordinator(error)) => {
            assert!(error.is_panic(), "{error}");
        }
        Err(batter_core::command::CommandFailure::Report(_)) => {
            panic!("a report claimed cleanup without a timer")
        }
        Ok(_) => panic!("work succeeded without the time driver"),
    }
}
