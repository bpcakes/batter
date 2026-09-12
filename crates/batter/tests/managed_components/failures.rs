use crate::managed_support::*;
use batter::{
    BoxError,
    lifecycle::{ManagedComponent, ManagedFailure, ManagedInitialization, ManagedSettlement},
};
use std::{
    future::{Future, pending},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::oneshot;

struct DropPanic<F> {
    inner: Pin<Box<F>>,
}
impl<F> DropPanic<F> {
    fn new(inner: F) -> Self {
        Self {
            inner: Box::pin(inner),
        }
    }
}
impl<F: Future> Future for DropPanic<F> {
    type Output = F::Output;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.as_mut().poll(cx)
    }
}
impl<F> Drop for DropPanic<F> {
    fn drop(&mut self) {
        panic!("secret destructor payload");
    }
}

#[tokio::test(start_paused = true)]
async fn initialization_error_and_its_destructor_panic_cannot_discard_native_report() {
    let mut supervisor = supervisor();
    let closed = cleanup(&mut supervisor, Arc::new(AtomicBool::new(true)));
    let (stop, stopping) = oneshot::channel();
    let mut stop = Some(stop);
    supervisor
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                DropPanic::new(async {
                    Err(
                        Box::new(std::io::Error::other("secret initialization failure"))
                            as BoxError,
                    )
                }),
                pending(),
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    stopping.await.unwrap();
                    NativeReport {
                        joined: true,
                        detail: "native settled after failure",
                    }
                },
            ))
        })
        .unwrap();
    let report = supervisor.start().wait().await.unwrap();
    let outcome = &report.managed[0].outcome;
    let ManagedInitialization::Failed(ManagedFailure::Failed(error)) = &outcome.initialization
    else {
        panic!("original error missing")
    };
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().to_string(),
        "secret initialization failure"
    );
    assert_eq!(outcome.failures.len(), 1);
    let ManagedFailure::Panicked(payload) = &outcome.failures[0] else {
        panic!("destruction panic missing")
    };
    payload
        .try_inspect(|value| {
            assert_eq!(
                value.downcast_ref::<&str>(),
                Some(&"secret destructor payload")
            )
        })
        .expect("payload inspection is uncontended");
    assert!(outcome.settlement.is_some());
    assert!(!format!("{outcome:?}").contains("secret"));
    assert!(!closed.load(Ordering::SeqCst));
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn native_report_is_retained_before_its_future_destructor_panics() {
    let mut supervisor = supervisor();
    let closed = cleanup(&mut supervisor, Arc::new(AtomicBool::new(true)));
    let (stop, stopping) = oneshot::channel();
    let mut stop = Some(stop);
    supervisor
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                DropPanic::new(async move {
                    stopping.await.unwrap();
                    NativeReport {
                        joined: true,
                        detail: "retained before destruction",
                    }
                }),
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    let outcome = &report.managed[0].outcome;
    assert_eq!(
        outcome
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<NativeReport>()
            .unwrap()
            .detail,
        "retained before destruction"
    );
    assert!(matches!(
        &outcome.failures[..],
        [ManagedFailure::Panicked(_)]
    ));
    assert!(!closed.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn dropped_service_owner_keeps_managed_driver_and_cleanup_alive() {
    let mut supervisor = supervisor();
    let closed = cleanup(&mut supervisor, Arc::new(AtomicBool::new(true)));
    let (stop, stopping) = oneshot::channel();
    let mut stop = Some(stop);
    supervisor
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    stopping.await.unwrap();
                    NativeReport {
                        joined: true,
                        detail: "owned settlement",
                    }
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let observer = running.observer();
    // Cancel a borrowed waiter without sending a stop request.
    {
        let waiter = running.wait();
        tokio::pin!(waiter);
        std::future::poll_fn(|cx| {
            assert!(waiter.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(!running.handle().is_draining());
    }
    drop(running);
    let report = observer.wait().await.unwrap();
    assert!(report.is_success());
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn initialization_error_is_visible_while_native_settlement_remains_pending() {
    let mut supervisor = supervisor();
    let (release, settling) = oneshot::channel();
    supervisor
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                async {
                    Err(
                        Box::new(std::io::Error::other("original initialization error"))
                            as BoxError,
                    )
                },
                pending(),
                |started| started,
                async move {
                    settling.await.unwrap();
                    NativeReport {
                        joined: false,
                        detail: "independent later failure",
                    }
                },
            ))
        })
        .unwrap();
    let report = supervisor.start().wait().await.unwrap();
    let managed = &report.managed[0];
    assert!(!managed.outcome.finished);
    assert!(matches!(
        managed.outcome.initialization,
        ManagedInitialization::Failed(ManagedFailure::Failed(_))
    ));
    release.send(()).unwrap();
    let final_outcome = managed.observer.wait().await;
    assert!(matches!(
        final_outcome.initialization,
        ManagedInitialization::Failed(ManagedFailure::Failed(_))
    ));
    assert_eq!(
        final_outcome
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<NativeReport>()
            .unwrap()
            .detail,
        "independent later failure"
    );
}

#[tokio::test(start_paused = true)]
async fn rejected_native_configuration_retains_error_and_releases_acquired_dependencies() {
    let mut supervisor = supervisor();
    let closed = cleanup(&mut supervisor, Arc::new(AtomicBool::new(true)));
    supervisor
        .register_managed(
            "native",
            context(),
            |_| -> Result<ManagedComponent<NativeReport>, BoxError> {
                Err(Box::new(std::io::Error::other(
                    "secret configuration error",
                )))
            },
        )
        .unwrap();
    let report = supervisor.start().wait().await.unwrap();
    let outcome = &report.managed[0].outcome;
    let ManagedInitialization::Rejected(error) = &outcome.initialization else {
        panic!("construction rejection missing")
    };
    assert_eq!(error.to_string(), "secret configuration error");
    assert!(!format!("{outcome:?}").contains("secret"));
    assert!(closed.load(Ordering::SeqCst));
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn report_classifier_panic_preserves_original_native_evidence_and_skips_cleanup() {
    struct Unclassifiable {
        detail: &'static str,
    }
    impl ManagedSettlement for Unclassifiable {
        fn is_success(&self) -> bool {
            panic!("secret classifier panic")
        }
        fn allows_dependency_cleanup(&self) -> bool {
            true
        }
    }
    let mut supervisor = supervisor();
    let closed = cleanup(&mut supervisor, Arc::new(AtomicBool::new(true)));
    let (stop, stopping) = oneshot::channel();
    let mut stop = Some(stop);
    supervisor
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    stopping.await.unwrap();
                    Unclassifiable {
                        detail: "native evidence",
                    }
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    let outcome = &report.managed[0].outcome;
    assert!(matches!(
        &outcome.failures[..],
        [ManagedFailure::Panicked(_)]
    ));
    assert_eq!(
        outcome
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<Unclassifiable>()
            .unwrap()
            .detail,
        "native evidence"
    );
    assert!(!closed.load(Ordering::SeqCst));
    assert!(!format!("{outcome:?}").contains("secret"));
}
