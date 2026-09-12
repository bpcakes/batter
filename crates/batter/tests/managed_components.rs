//! Public managed-registration contracts, using actual owned Tokio descendants.

#[path = "managed_components/failures.rs"]
mod failures;
mod managed_support;

#[path = "managed_components/clocks.rs"]
mod clocks;

#[path = "managed_components/initialization.rs"]
mod initialization;

use batter::{
    BoxError, RegistrationError,
    cleanup::SkipReason,
    lifecycle::{ManagedComponent, ManagedInitialization, Readiness},
    operation::OperationContext,
};
use managed_support::*;
use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::oneshot, time::Instant};

#[tokio::test(start_paused = true)]
async fn registration_is_inert_and_names_are_shared_with_direct_components() {
    let calls = Arc::new(AtomicBool::new(false));
    let mut supervisor = supervisor();
    let called = calls.clone();
    supervisor
        .register_managed("native", context(), move |_| {
            called.store(true, Ordering::SeqCst);
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                |started| started,
                pending::<NativeReport>(),
            ))
        })
        .unwrap();
    assert_eq!(
        supervisor.register("native", |_| async { Ok(()) }),
        Err(RegistrationError::Duplicate("native"))
    );
    assert_eq!(
        supervisor.register_managed(
            "native",
            context(),
            |_| -> Result<ManagedComponent<NativeReport>, BoxError> {
                panic!("duplicate factory must remain inert")
            }
        ),
        Err(RegistrationError::Duplicate("native"))
    );
    drop(supervisor);
    assert!(!calls.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn native_ack_requires_approval_and_cleanup_follows_actual_descendant_join() {
    let mut supervisor = supervisor();
    let child_dropped = Arc::new(AtomicBool::new(false));
    let closed = cleanup(&mut supervisor, child_dropped.clone());
    let (entered, entering) = oneshot::channel();
    let (initialize, initialized) = oneshot::channel();
    let lifetime = child_dropped.clone();
    supervisor
        .register_managed("native", context(), move |budget| {
            assert_eq!(budget.graceful(), Duration::from_secs(1));
            assert_eq!(budget.abort(), Duration::from_secs(1));
            let (stop, stopping) = oneshot::channel();
            let mut stop = Some(stop);
            let child = tokio::spawn(async move {
                let _lifetime = Dropped(lifetime);
                entered.send(()).unwrap();
                stopping.await.unwrap();
            });
            Ok(ManagedComponent::new(
                async move {
                    initialized.await.unwrap();
                    Ok(())
                },
                pending(),
                move |started| {
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    child.await.unwrap();
                    NativeReport {
                        joined: true,
                        detail: "native joined",
                    }
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    entering.await.unwrap();
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    initialize.send(()).unwrap();
    // Approval is independent of the initialization acknowledgement.
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success(), "{report}");
    assert!(child_dropped.load(Ordering::SeqCst));
    assert!(closed.load(Ordering::SeqCst));
    let native = report.managed[0]
        .outcome
        .settlement
        .as_ref()
        .unwrap()
        .downcast_ref::<NativeReport>()
        .unwrap();
    assert_eq!(native.detail, "native joined");
}

#[tokio::test(start_paused = true)]
async fn native_stop_drains_process_before_settlement_and_preserves_late_report() {
    let mut supervisor = supervisor();
    let (native_stop, stopped) = oneshot::channel();
    let (release, settle) = oneshot::channel();
    supervisor
        .register_managed("native", context(), move |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                async move {
                    stopped.await.unwrap();
                },
                |started| started,
                async move {
                    settle.await.unwrap();
                    NativeReport {
                        joined: false,
                        detail: "late descendant failure",
                    }
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    native_stop.send(()).unwrap();
    running.handle().draining().await;
    assert_eq!(running.handle().readiness(), Readiness::Draining);
    release.send(()).unwrap();
    let report = running.wait().await.unwrap();
    assert!(!report.is_success());
    let native = &report.managed[0].outcome;
    assert!(native.unexpected_stop);
    assert_eq!(
        native
            .settlement
            .as_ref()
            .unwrap()
            .downcast_ref::<NativeReport>()
            .unwrap()
            .detail,
        "late descendant failure"
    );
    assert!(!native.allows_dependency_cleanup());
}

#[tokio::test(start_paused = true)]
async fn wrapper_abortion_keeps_settlement_owned_and_never_runs_cleanup_later() {
    let mut supervisor = supervisor();
    let finished = Arc::new(AtomicBool::new(false));
    let closed = cleanup(&mut supervisor, finished.clone());
    let (release, settle) = oneshot::channel();
    let dropped = finished.clone();
    supervisor
        .register_managed("native", context(), move |_| {
            // This fixture violates the native settlement budget deliberately.
            let child = tokio::spawn(async move {
                let _life = Dropped(dropped);
                settle.await.unwrap();
            });
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                |started| started,
                async move {
                    child.await.unwrap();
                    NativeReport {
                        joined: true,
                        detail: "late join",
                    }
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert_eq!(report.abort_requested, ["native"]);
    assert!(!report.managed[0].outcome.finished);
    assert!(!finished.load(Ordering::SeqCst));
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    release.send(()).unwrap();
    let late = report.managed[0].observer.wait().await;
    assert!(late.is_success());
    assert!(finished.load(Ordering::SeqCst));
    assert!(!closed.load(Ordering::SeqCst));
    assert!(!report.is_success()); // The bounded process result is frozen.
}

#[tokio::test(start_paused = true)]
async fn expired_registration_never_invokes_native_factory() {
    let mut supervisor = supervisor();
    let closed = cleanup(&mut supervisor, Arc::new(AtomicBool::new(true)));
    supervisor
        .register_managed(
            "native",
            OperationContext::at(Instant::now()),
            |_| -> Result<ManagedComponent<NativeReport>, BoxError> {
                panic!("expired factory must remain inert")
            },
        )
        .unwrap();
    let report = supervisor.start().wait().await.unwrap();
    assert!(matches!(
        report.managed[0].outcome.initialization,
        ManagedInitialization::NotStarted {
            interruption: Some(batter::operation::Interruption::DeadlineExceeded)
        }
    ));
    assert!(!report.is_success());
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn startup_timeout_stops_native_and_still_retains_its_report() {
    let mut supervisor = supervisor();
    let (stop, stopping) = oneshot::channel();
    let mut stop = Some(stop);
    let begin = Instant::now();
    supervisor
        .register_managed(
            "native",
            OperationContext::new(Duration::from_millis(25)).unwrap(),
            |_| {
                Ok(ManagedComponent::new(
                    pending(),
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
                            detail: "clean after timeout",
                        }
                    },
                ))
            },
        )
        .unwrap();
    let report = supervisor.start().wait().await.unwrap();
    assert_eq!(Instant::now() - begin, Duration::from_millis(25));
    assert!(matches!(
        report.managed[0].outcome.initialization,
        ManagedInitialization::Interrupted(batter::operation::Interruption::DeadlineExceeded)
    ));
    assert!(report.managed[0].outcome.settlement.is_some());
    assert!(report.managed[0].outcome.allows_dependency_cleanup());
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn delayed_driver_observation_does_not_reset_stop_allowance() {
    let mut supervisor = supervisor();
    supervisor
        .register("direct", |_| async { pending().await })
        .unwrap();
    let handle = supervisor.handle();
    handle.request();
    let begin = Instant::now();
    tokio::time::advance(Duration::from_secs(5)).await;
    handle.request(); // Neither another request nor first driver poll resets time.
    let report = supervisor.run_until(pending()).await;
    assert_eq!(Instant::now() - begin, Duration::from_secs(5));
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn native_stop_callback_receives_the_original_parent_stop_time() {
    let mut supervisor = supervisor();
    let (stop, stopping) = oneshot::channel();
    let mut stop = Some(stop);
    let (observed, observation) = oneshot::channel();
    let mut observed = Some(observed);
    supervisor
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                move |started| {
                    if let Some(observed) = observed.take() {
                        let _ = observed.send(started);
                    }
                    if let Some(stop) = stop.take() {
                        let _ = stop.send(());
                    }
                    started
                },
                async move {
                    stopping.await.unwrap();
                    NativeReport {
                        joined: true,
                        detail: "bounded native stop",
                    }
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    let started = Instant::now();
    running.handle().request();
    tokio::time::advance(Duration::from_millis(250)).await;
    running.handle().request();
    assert_eq!(observation.await.unwrap(), started);
    assert!(running.wait().await.unwrap().is_success());
}
