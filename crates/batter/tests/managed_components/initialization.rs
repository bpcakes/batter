use crate::managed_support::*;
use batter::{
    BoxError,
    lifecycle::{ManagedComponent, ManagedFailure, ManagedInitialization},
    operation::{Interruption, OperationContext},
};
use std::{
    future::{Future, pending, poll_fn},
    sync::{Arc, atomic::AtomicBool},
    task::Poll,
    time::Duration,
};
use tokio::sync::oneshot;

struct CancelOnDrop(OperationContext);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Clone, Copy)]
enum Completion {
    CancelInPoll,
    CancelInDrop,
    ExpireInPoll,
    FailureAndCancellation,
}

async fn completion_boundary(completion: Completion) {
    let mut process = supervisor();
    let closed = cleanup(&mut process, Arc::new(AtomicBool::new(true)));
    let context = context();
    let stopping = context.clone();
    let (stop, stopped) = oneshot::channel();
    let mut stop = Some(stop);
    process
        .register_managed("native", context, move |_| {
            let on_drop = matches!(completion, Completion::CancelInDrop)
                .then(|| CancelOnDrop(stopping.clone()));
            let initialized = poll_fn(move |cx| {
                // Keep this guard alive until the completed future is destroyed.
                let _retained = &on_drop;
                match completion {
                    Completion::CancelInPoll | Completion::FailureAndCancellation => {
                        stopping.cancel()
                    }
                    Completion::ExpireInPoll => {
                        // Advance the test clock during this single poll without
                        // returning Pending to the managed driver's select loop.
                        let mut advancing = Box::pin(tokio::time::advance(Duration::from_secs(11)));
                        let _ = advancing.as_mut().poll(cx);
                        assert_eq!(stopping.check(), Err(Interruption::DeadlineExceeded));
                    }
                    Completion::CancelInDrop => {}
                }
                Poll::Ready(
                    if matches!(completion, Completion::FailureAndCancellation) {
                        Err(
                            Box::new(std::io::Error::other("original initialization failure"))
                                as BoxError,
                        )
                    } else {
                        Ok(())
                    },
                )
            });
            Ok(ManagedComponent::new(
                initialized,
                pending(),
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    stopped
                        .await
                        .expect("native stop requested before settlement");
                    NativeReport {
                        joined: true,
                        detail: "native settled after interrupted initialization",
                    }
                },
            ))
        })
        .unwrap();
    let running = process.start();
    running.handle().mark_ready();
    let ready = running.handle().wait_ready().await;
    // Always drain and join even when the original defect allowed readiness.
    let report = running.shutdown().await.unwrap();
    assert!(ready.is_err(), "interrupted initialization became ready");
    let outcome = &report.managed[0].outcome;
    match completion {
        Completion::FailureAndCancellation => {
            let ManagedInitialization::Failed(ManagedFailure::Failed(error)) =
                &outcome.initialization
            else {
                panic!("original initialization error was replaced");
            };
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().to_string(),
                "original initialization failure"
            );
        }
        Completion::ExpireInPoll => assert!(matches!(
            outcome.initialization,
            ManagedInitialization::Interrupted(Interruption::DeadlineExceeded)
        )),
        _ => assert!(matches!(
            outcome.initialization,
            ManagedInitialization::Interrupted(Interruption::Cancelled)
        )),
    }
    assert!(
        outcome
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<NativeReport>()
            .unwrap()
            .joined
    );
    assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn cancellation_in_final_initialization_poll_prevents_readiness() {
    completion_boundary(Completion::CancelInPoll).await;
}

#[tokio::test(start_paused = true)]
async fn cancellation_in_initialization_destruction_prevents_readiness() {
    completion_boundary(Completion::CancelInDrop).await;
}

#[tokio::test(start_paused = true)]
async fn deadline_crossed_in_final_initialization_poll_prevents_readiness() {
    completion_boundary(Completion::ExpireInPoll).await;
}

#[tokio::test(start_paused = true)]
async fn initialization_error_survives_simultaneous_cancellation() {
    completion_boundary(Completion::FailureAndCancellation).await;
}

#[derive(Clone, Copy, PartialEq)]
enum EarlyExit {
    NativeStop,
    NativeSettlement,
    ProcessDrain,
}

async fn early_exit_before_initialization(exit: EarlyExit) {
    let mut process = supervisor();
    let closed = cleanup(&mut process, Arc::new(AtomicBool::new(true)));
    let (entered, entering) = oneshot::channel();
    let (stop, stopped) = oneshot::channel();
    process
        .register_managed("native", context(), move |_| {
            let mut stop = Some(stop);
            Ok(ManagedComponent::new(
                async move {
                    let _ = entered.send(());
                    pending().await
                },
                async move {
                    if exit != EarlyExit::NativeStop {
                        pending::<()>().await;
                    }
                },
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    if exit != EarlyExit::NativeSettlement {
                        stopped.await.unwrap();
                    }
                    NativeReport {
                        joined: true,
                        detail: "settled before initialization",
                    }
                },
            ))
        })
        .unwrap();
    let running = process.start();
    running.handle().mark_ready();
    if exit == EarlyExit::ProcessDrain {
        entering.await.unwrap();
        running.handle().request();
    }
    let ready = running.handle().wait_ready().await;
    let report = running.wait().await.unwrap();
    assert!(ready.is_err());
    let outcome = &report.managed[0].outcome;
    assert_eq!(outcome.unexpected_stop, exit != EarlyExit::ProcessDrain);
    if exit == EarlyExit::ProcessDrain {
        assert!(matches!(
            outcome.initialization,
            ManagedInitialization::Draining
        ));
    } else {
        assert!(matches!(
            outcome.initialization,
            ManagedInitialization::Stopped
        ));
    }
    assert!(
        outcome
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<NativeReport>()
            .unwrap()
            .joined
    );
    assert!(outcome.allows_dependency_cleanup());
    assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn native_stop_before_initialization_has_its_own_state() {
    early_exit_before_initialization(EarlyExit::NativeStop).await;
}

#[tokio::test(start_paused = true)]
async fn native_settlement_before_initialization_has_its_own_state() {
    early_exit_before_initialization(EarlyExit::NativeSettlement).await;
}

#[tokio::test(start_paused = true)]
async fn process_drain_before_initialization_keeps_draining_state() {
    early_exit_before_initialization(EarlyExit::ProcessDrain).await;
}
