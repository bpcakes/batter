//! Registration occurs before the acquired handle leaves library control.
use batter_core::{
    BoxError,
    cleanup::{Acquired, CleanupBudget, CleanupStack},
    command::{Command, CommandCause},
    operation::OperationOwner,
};
use std::{
    convert::Infallible,
    future::{Future, pending},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

fn budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(2),
        Duration::from_secs(1),
        Duration::from_millis(100),
    )
    .expect("valid cleanup budget")
}

#[tokio::test]
async fn successful_handoffs_register_before_use_and_close_in_reverse_order() {
    let mut stack = CleanupStack::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    for name in ["dependency", "dependent"] {
        let closed = events.clone();
        let value = stack
            .reserve(name)
            .unwrap()
            .acquire(|| async move {
                Ok::<_, Infallible>(Acquired::new(name, move || async move {
                    closed.lock().unwrap().push(name);
                    Ok(())
                }))
            })
            .await
            .unwrap();
        assert_eq!(value, name);
        assert_eq!(stack.len(), if name == "dependency" { 1 } else { 2 });
    }
    let report = stack.close(budget()).await;
    assert!(report.is_success());
    assert_eq!(*events.lock().unwrap(), ["dependent", "dependency"]);
}

#[tokio::test]
async fn error_unpolled_and_cancelled_acquisitions_register_nothing() {
    let mut stack = CleanupStack::new();
    let error = stack
        .reserve("failed")
        .unwrap()
        .acquire(|| async {
            Err::<Acquired<(), fn() -> std::future::Ready<Result<(), BoxError>>>, _>("original")
        })
        .await
        .unwrap_err();
    assert_eq!(error, "original");
    let invoked = Arc::new(AtomicBool::new(false));
    let check = invoked.clone();
    let acquisition = stack.reserve("unpolled").unwrap().acquire(move || {
        check.store(true, Ordering::SeqCst);
        async { Ok::<_, Infallible>(Acquired::new((), || async { Ok(()) })) }
    });
    drop(acquisition);
    assert!(!invoked.load(Ordering::SeqCst), "factory remains inert");
    let mut acquisition = Box::pin(stack.reserve("cancelled").unwrap().acquire(|| async {
        pending::<()>().await;
        Ok::<_, Infallible>(Acquired::new((), || async { Ok(()) }))
    }));
    tokio::select! { biased;
        _ = &mut acquisition => panic!("pending acquisition returned"),
        _ = tokio::task::yield_now() => {}
    }
    drop(acquisition);
    assert!(stack.is_empty());
}

#[tokio::test]
async fn command_early_error_still_finalizes_the_acquired_resource() {
    let closed = Arc::new(AtomicBool::new(false));
    let mark = closed.clone();
    let owner = Command::new(
        OperationOwner::new(Duration::from_secs(5))
            .unwrap()
            .into_context(),
        budget(),
        move |scope| {
            Box::pin(async move {
                let handle = scope
                    .reserve_cleanup("resource")?
                    .acquire(|| async move {
                        Ok::<_, Infallible>(Acquired::new(42, move || async move {
                            mark.store(true, Ordering::SeqCst);
                            Ok(())
                        }))
                    })
                    .await
                    .unwrap();
                assert_eq!(handle, 42);
                Err::<(), BoxError>(std::io::Error::other("after acquisition").into())
            })
        },
    )
    .start_without_signals();
    let failure = owner.finish().await.unwrap_err();
    assert!(
        closed.load(Ordering::SeqCst),
        "registered before caller failed: {failure:?}"
    );
}

/// A native future can yield a resource and then panic while being destroyed.
struct PanicOnDrop<T>(Option<T>);
impl<T: Unpin> Future for PanicOnDrop<T> {
    type Output = Result<T, Infallible>;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.0.take().expect("polled once")))
    }
}
impl<T> Drop for PanicOnDrop<T> {
    fn drop(&mut self) {
        panic!("acquisition destructor panic");
    }
}

#[tokio::test]
async fn native_future_destruction_after_ready_cannot_skip_registration() {
    let closed = Arc::new(AtomicBool::new(false));
    let mark = closed.clone();
    let command = Command::new(
        OperationOwner::new(Duration::from_secs(5))
            .unwrap()
            .into_context(),
        budget(),
        move |scope| {
            Box::pin(async move {
                scope
                    .reserve_cleanup("resource")?
                    .acquire(|| {
                        PanicOnDrop(Some(Acquired::new((), move || async move {
                            mark.store(true, Ordering::SeqCst);
                            Ok(())
                        })))
                    })
                    .await
                    .unwrap();
                Ok::<_, batter_core::RegistrationError>(())
            })
        },
    )
    .start();
    let report = command.wait().await.unwrap();
    assert!(matches!(report.work, Err(CommandCause::Panicked(_))));
    assert!(
        closed.load(Ordering::SeqCst),
        "resource registered before native future destruction"
    );
    assert!(report.cleanup.as_ref().unwrap().is_success());
}
