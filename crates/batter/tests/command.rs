#[path = "command/budgets.rs"]
mod budgets;
#[path = "command/completion.rs"]
mod completion;
#[path = "command/failures.rs"]
mod failures;
#[path = "command/runtime.rs"]
mod runtime;

use batter::{
    cleanup::{CleanupBudget, CleanupOutcome},
    command::{Command, CommandCause},
    operation::{Interruption, OperationContext},
};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};

fn budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(second, second, second).unwrap()
}

fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(10)).unwrap()
}

async fn poll_once(future: impl Future) {
    tokio::pin!(future);
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn inert_specification_and_owner_drop_before_first_poll_invoke_no_work() {
    let called = Arc::new(AtomicBool::new(false));
    let call = called.clone();
    let command = Command::new(context(), budget(), move |_| {
        call.store(true, Ordering::SeqCst);
        Box::pin(async { Ok::<_, Infallible>(()) })
    });
    drop(command);
    assert!(!called.load(Ordering::SeqCst));
    let command = Command::new(context(), budget(), |_| {
        panic!("abandoned owner must not invoke a never-polled factory");
        #[allow(unreachable_code)]
        Box::pin(async { Ok::<_, Infallible>(()) })
    })
    .start();
    let observer = command.observer();
    drop(command);
    let report = observer.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(report.cleanup.as_ref().unwrap().is_success());
}

#[tokio::test(start_paused = true)]
async fn cancelled_waiter_does_not_cancel_work_and_observers_share_value_identity() {
    struct Value(u32);
    let (release, released) = oneshot::channel();
    let command = Command::new(context(), budget(), |_| {
        Box::pin(async move {
            released.await.unwrap();
            Ok::<_, Infallible>(Value(42))
        })
    })
    .start();
    let observer = command.observer();
    poll_once(command.wait()).await;
    release.send(()).unwrap();
    let first = command.wait().await.unwrap();
    let second = observer.wait().await.unwrap();
    assert!(first.is_success());
    let value = first.work.as_ref().unwrap();
    assert_eq!(value.0, 42);
    assert!(std::ptr::eq(value, second.work.as_ref().unwrap()));
}

#[tokio::test(start_paused = true)]
async fn owner_drop_cancels_work_downward_and_finishes_lifo_resources() {
    let parent = context();
    let semaphore = Arc::new(Semaphore::new(1));
    let acquiring = semaphore.clone();
    let (acquired, acquisition) = oneshot::channel();
    let (cleanup_entered, cleanup_entry) = oneshot::channel();
    let (release_cleanup, released_cleanup) = oneshot::channel();
    let command = Command::new(parent.clone(), budget(), move |scope| {
        Box::pin(async move {
            let slot = scope.reserve_cleanup("capacity")?;
            let permit = acquiring.acquire_owned().await.unwrap();
            slot.register(move || async move {
                drop(permit);
                Ok(())
            });
            scope
                .reserve_cleanup("dependent")?
                .register(move || async move {
                    cleanup_entered.send(()).unwrap();
                    released_cleanup.await.unwrap();
                    Ok(())
                });
            acquired.send(()).unwrap();
            pending::<()>().await;
            Ok::<_, batter::RegistrationError>(())
        })
    })
    .start();
    acquisition.await.unwrap();
    assert_eq!(semaphore.available_permits(), 0);
    let observer = command.observer();
    drop(command);
    cleanup_entry.await.unwrap();
    assert!(parent.check().is_ok());
    assert_eq!(
        semaphore.available_permits(),
        0,
        "dependent cleanup still owns its prerequisite"
    );
    poll_once(observer.wait()).await;
    release_cleanup.send(()).unwrap();
    let report = observer.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(
        cleanup
            .records
            .iter()
            .map(|record| record.name)
            .collect::<Vec<_>>(),
        ["dependent", "capacity"]
    );
    assert!(cleanup.is_success());
    assert_eq!(semaphore.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_cleanup_does_not_interrupt_its_independent_budget() {
    let parent = context();
    let (entered, entering) = oneshot::channel();
    let (finish, finishing) = oneshot::channel();
    let command = Command::new(parent.clone(), budget(), |scope| {
        Box::pin(async move {
            scope
                .reserve_cleanup("resource")?
                .register(move || async move {
                    entered.send(()).unwrap();
                    finishing.await.unwrap();
                    Ok(())
                });
            Ok::<_, batter::RegistrationError>(42)
        })
    })
    .start();
    entering.await.unwrap();
    command.cancel();
    parent.cancel();
    finish.send(()).unwrap();
    let report = command.wait().await.unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(
        report.cleanup.as_ref().unwrap().records[0].outcome,
        CleanupOutcome::Succeeded
    );
}

#[tokio::test(start_paused = true)]
async fn finishing_work_cancels_its_children_before_cleanup_but_not_parent() {
    let parent = context();
    let command = Command::new(parent.clone(), budget(), |scope| {
        Box::pin(async move {
            let child = scope.context().child(Duration::from_secs(3)).unwrap();
            scope
                .reserve_cleanup("resource")?
                .register(move || async move {
                    assert_eq!(child.check(), Err(Interruption::Cancelled));
                    Ok(())
                });
            Ok::<_, batter::RegistrationError>(())
        })
    })
    .start();
    assert!(command.wait().await.unwrap().is_success());
    assert!(parent.check().is_ok());
}
