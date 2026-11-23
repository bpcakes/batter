//! Native acquisition has no caller-owned Ready-to-registration interval.
use batter_core::{
    cleanup::{CleanupBudget, CleanupOutcome, CleanupStack},
    command::{Command, CommandCause, CommandFailure},
    operation::OperationOwner,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

fn budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(second, second, second).expect("valid test budget")
}

#[tokio::test]
async fn semaphore_is_registered_before_caller_can_await_or_fail() {
    let capacity = Arc::new(Semaphore::new(1));
    let acquired = capacity.clone();
    // A failed assertion inside the work would only turn one error into another.
    let held = Arc::new(AtomicUsize::new(usize::MAX));
    let observed = held.clone();
    let command = Command::new(
        OperationOwner::new(Duration::from_secs(2))
            .unwrap()
            .into_context(),
        budget(),
        move |scope| {
            Box::pin(async move {
                scope
                    .reserve_cleanup("capacity")
                    .unwrap()
                    .acquire_semaphore(acquired.clone())
                    .await
                    .unwrap();
                observed.store(acquired.available_permits(), Ordering::SeqCst);
                tokio::task::yield_now().await;
                Err::<(), _>("caller failed")
            })
        },
    )
    .start_without_signals();
    let failure = command.finish().await.unwrap_err();
    assert_eq!(
        held.load(Ordering::SeqCst),
        0,
        "the permit was held while the caller ran"
    );
    let CommandFailure::Report(report) = &failure else {
        panic!("the command coordinator produced no report: {failure:?}")
    };
    assert!(
        matches!(report.work, Err(CommandCause::Failed("caller failed"))),
        "the caller's own failure is retained: {failure:?}"
    );
    let cleanup = report.cleanup.as_ref().expect("cleanup coordinator ran");
    assert!(cleanup.skipped.is_empty(), "{failure:?}");
    assert_eq!(cleanup.records.len(), 1, "{failure:?}");
    assert_eq!(cleanup.records[0].name, "capacity");
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_eq!(capacity.available_permits(), 1, "{failure:?}");
}

#[tokio::test]
async fn pending_and_unpolled_acquisitions_have_no_effect() {
    let capacity = Arc::new(Semaphore::new(0));
    let mut stack = CleanupStack::new();
    drop(
        stack
            .reserve("unpolled")
            .unwrap()
            .acquire_semaphore(capacity.clone()),
    );
    let mut waiting = Box::pin(
        stack
            .reserve("pending")
            .unwrap()
            .acquire_semaphore(capacity.clone()),
    );
    tokio::select! { biased;
        _ = &mut waiting => panic!("no permit available"),
        _ = tokio::task::yield_now() => {}
    }
    drop(waiting);
    capacity.add_permits(1);
    assert_eq!(capacity.available_permits(), 1);
    assert!(stack.is_empty());
    capacity.close();
    assert!(
        stack
            .reserve("closed")
            .unwrap()
            .acquire_semaphore(capacity)
            .await
            .is_err()
    );
    assert!(stack.is_empty());
}

#[tokio::test]
async fn socket_registration_precedes_exposure_and_cleanup_releases_owned_reference() {
    let mut stack = CleanupStack::new();
    let socket = stack
        .reserve("socket")
        .unwrap()
        .bind_udp("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(stack.len(), 1);
    assert_eq!(Arc::strong_count(&socket), 2);
    let weak = Arc::downgrade(&socket);
    let address = socket.local_addr().unwrap();
    socket.send_to(b"hello", address).await.unwrap();
    let mut message = [0; 5];
    socket.recv_from(&mut message).await.unwrap();
    assert_eq!(&message, b"hello");
    drop(socket);
    assert!(weak.upgrade().is_some(), "cleanup retains socket");
    assert!(stack.close(budget()).await.is_success());
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn supported_acquisitions_preserve_lifo_and_native_bind_errors() {
    let capacity = Arc::new(Semaphore::new(1));
    let mut stack = CleanupStack::new();
    stack
        .reserve("capacity")
        .unwrap()
        .acquire_semaphore(capacity.clone())
        .await
        .unwrap();
    let socket = stack
        .reserve("socket")
        .unwrap()
        .bind_udp("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    assert!(
        stack
            .reserve("conflict")
            .unwrap()
            .bind_udp(socket.local_addr().unwrap())
            .await
            .is_err()
    );
    assert_eq!(stack.len(), 2);
    drop(socket);
    let report = stack.close(budget()).await;
    assert!(report.is_success());
    assert_eq!(
        report.records.iter().map(|r| r.name).collect::<Vec<_>>(),
        ["socket", "capacity"]
    );
    assert_eq!(capacity.available_permits(), 1);
}
