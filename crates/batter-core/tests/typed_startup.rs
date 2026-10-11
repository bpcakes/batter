//! Async-closure protected startup: typed output, rejected output destruction
//! and boxed application failures.

use batter_core::{
    BoxError, RegistrationError,
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::{Interruption, OperationOwner},
    service::{self, NoDiagnostics, ServiceOutcome},
    startup::{
        ApplicationError, InitializationError, StartupCause, StartupError, StartupFailure,
        StartupOutcome,
    },
};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

type Events = Arc<Mutex<Vec<&'static str>>>;
type ProtectedFailure = StartupFailure<InitializationError<ApplicationError>>;

fn cleanup_budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(second, second, second).unwrap()
}

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(ShutdownBudget::new(second, second, second, cleanup_budget()).unwrap())
}

fn context() -> batter_core::operation::OperationContext {
    OperationOwner::new(Duration::from_secs(2))
        .unwrap()
        .into_context()
}

/// Typed output that records its destruction.
struct Output {
    value: u16,
    events: Events,
}

impl Drop for Output {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("output destroyed");
    }
}

fn record_cleanup(scope: &mut batter_core::startup::ProtectedStartupScope, events: Events) {
    scope
        .reserve_cleanup("resource")
        .unwrap()
        .register(move || async move {
            events.lock().unwrap().push("cleanup");
            Ok(())
        });
}

#[tokio::test]
async fn typed_output_arrives_with_the_running_owner_and_never_with_observers() {
    let events = Events::default();
    let cleanup_events = events.clone();
    let output_events = events.clone();
    let mut starting = batter_core::startup::Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async move |scope| {
            scope.stage("resource.acquire")?;
            let slot = scope.reserve_cleanup("resource")?;
            // The scope borrow and the reservation both cross this await.
            tokio::task::yield_now().await;
            slot.register(move || async move {
                cleanup_events.lock().unwrap().push("cleanup");
                Ok(())
            });
            scope.stage("worker.register")?;
            scope
                .registration()
                .register("worker", |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    shutdown.draining().await;
                    Ok(shutdown.stopped())
                })?;
            Ok::<_, RegistrationError>(Output {
                value: 8080,
                events: output_events,
            })
        },
    )
    .start();
    let observer = starting.observer();

    let (running, output) = starting.wait_with_output().await.unwrap();
    assert_eq!(output.value, 8080);
    let StartupOutcome::Running(_) = observer.wait().await else {
        panic!("observer must see the running handoff")
    };
    running.status().wait_ready().await.unwrap();
    assert!(events.lock().unwrap().is_empty());
    let report = running.shutdown_checked().await.unwrap();
    assert!(report.report().is_success());
    assert_eq!(*events.lock().unwrap(), ["cleanup"]);
    drop(output);
    assert_eq!(*events.lock().unwrap(), ["cleanup", "output destroyed"]);
}

#[tokio::test]
async fn deferred_typed_output_keeps_readiness_with_the_application() {
    let mut starting = batter_core::startup::Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async |scope| {
            scope
                .registration()
                .register("worker", |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    shutdown.draining().await;
                    Ok(shutdown.stopped())
                })?;
            Ok::<_, RegistrationError>("bound")
        },
    )
    .without_readiness_approval()
    .start();

    let (pending, output) = starting.wait_with_output().await.unwrap();
    assert_eq!(output, "bound");
    assert_eq!(pending.status().readiness(), Readiness::Starting);
    let running = pending.approve_readiness();
    running.status().wait_ready().await.unwrap();
    assert!(
        running
            .shutdown_checked()
            .await
            .unwrap()
            .report()
            .is_success()
    );
}

#[tokio::test]
async fn output_returned_with_an_interruption_is_destroyed_before_cleanup_and_never_handed_off() {
    for interruption in ["drain", "cancel"] {
        let events = Events::default();
        let scope_events = events.clone();
        let output_events = events.clone();
        let process = supervisor();
        let handle = process.handle();
        let owner = OperationOwner::new(Duration::from_secs(2)).unwrap();
        let context = owner.context().clone();
        let mut starting = batter_core::startup::Startup::protected(
            process,
            context,
            cleanup_budget(),
            async move |scope| {
                record_cleanup(scope, scope_events);
                // The interruption arrives in the poll that returns output.
                match interruption {
                    "drain" => handle.request(),
                    _ => owner.cancel(),
                }
                Ok::<_, RegistrationError>(Output {
                    value: 1,
                    events: output_events,
                })
            },
        )
        .start();
        let Err(StartupError::Failed(report)) = starting.wait_with_output().await else {
            panic!("an interrupted startup must not hand off its output")
        };
        match interruption {
            "drain" => assert!(matches!(report.cause, StartupCause::Draining)),
            _ => assert!(matches!(
                report.cause,
                StartupCause::Interrupted(Interruption::Cancelled)
            )),
        }
        assert!(report.destruction_panic.is_none());
        assert!(report.cleanup.is_success());
        assert_eq!(*events.lock().unwrap(), ["output destroyed", "cleanup"]);
    }
}

#[tokio::test]
async fn rejected_output_destructor_panic_is_retained_and_cleanup_still_runs() {
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("rejected-output-drop-marker");
        }
    }

    let events = Events::default();
    let scope_events = events.clone();
    let process = supervisor();
    let handle = process.handle();
    let mut starting = batter_core::startup::Startup::protected(
        process,
        context(),
        cleanup_budget(),
        async move |scope| {
            record_cleanup(scope, scope_events);
            handle.request();
            Ok::<_, RegistrationError>(PanicOnDrop)
        },
    )
    .start();
    let Err(StartupError::Failed(report)) = starting.wait_with_output().await else {
        panic!("a drained startup must not hand off its output")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    report
        .destruction_panic
        .as_ref()
        .expect("the output destructor panic is retained")
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"rejected-output-drop-marker")
            );
        })
        .unwrap();
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_eq!(*events.lock().unwrap(), ["cleanup"]);
}

#[tokio::test]
async fn secondary_output_panic_with_a_panicking_payload_cannot_bypass_cleanup() {
    /// A `panic_any` payload whose own destructor panics again.
    struct PanickingPayload;
    impl Drop for PanickingPayload {
        fn drop(&mut self) {
            panic!("secondary-payload-drop-marker");
        }
    }
    struct OutputPanics;
    impl Drop for OutputPanics {
        fn drop(&mut self) {
            std::panic::panic_any(PanickingPayload);
        }
    }
    struct FuturePanics;
    impl Drop for FuturePanics {
        fn drop(&mut self) {
            panic!("future-destruction-marker");
        }
    }
    /// Completes with output, then panics when the completed future is destroyed.
    struct Completing(FuturePanics, Option<OutputPanics>);
    impl std::future::Future for Completing {
        type Output = Result<OutputPanics, RegistrationError>;
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Ready(Ok(self.1.take().expect("polled once")))
        }
    }

    let events = Events::default();
    let mut process = supervisor();
    let cleanup_events = events.clone();
    process
        .reserve_cleanup("resource")
        .unwrap()
        .register(move || async move {
            cleanup_events.lock().unwrap().push("cleanup");
            Ok(())
        });
    let mut starting = batter_core::startup::Startup::protected(
        process,
        context(),
        cleanup_budget(),
        |_scope: &mut batter_core::startup::ProtectedStartupScope| {
            Completing(FuturePanics, Some(OutputPanics))
        },
    )
    .start();
    let Err(StartupError::Failed(report)) = starting.wait_with_output().await else {
        panic!("the coordinator must retain a report and run cleanup")
    };
    assert!(matches!(report.cause, StartupCause::DestructionPanicked));
    report
        .destruction_panic
        .as_ref()
        .unwrap()
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"future-destruction-marker")
            );
        })
        .unwrap();
    assert!(report.cleanup.is_success());
    assert_eq!(*events.lock().unwrap(), ["cleanup"]);
}

#[tokio::test]
async fn dropping_the_starting_owner_after_handoff_destroys_queued_output_and_drains() {
    let events = Events::default();
    let output_events = events.clone();
    let starting = batter_core::startup::Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async move |scope| {
            scope
                .registration()
                .register("worker", |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    shutdown.draining().await;
                    Ok(shutdown.stopped())
                })?;
            Ok::<_, RegistrationError>(Output {
                value: 2,
                events: output_events,
            })
        },
    )
    .start();
    let StartupOutcome::Running(running) = starting.observer().wait().await else {
        panic!("startup must hand off before owner loss")
    };
    assert!(events.lock().unwrap().is_empty());
    drop(starting);
    assert_eq!(*events.lock().unwrap(), ["output destroyed"]);
    assert!(running.wait_checked().await.unwrap().report().is_success());
}

#[tokio::test]
async fn failed_and_panicking_initializers_yield_no_output_and_close_cleanup() {
    let events = Events::default();
    let scope_events = events.clone();
    let mut failed = batter_core::startup::Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async move |scope| {
            record_cleanup(scope, scope_events);
            scope.stage("resource.verify")?;
            Err::<u16, _>(RegistrationError::Duplicate("resource"))
        },
    )
    .start();
    let Err(StartupError::Failed(report)) = failed.wait_with_output().await else {
        panic!("failed initializer must not yield output")
    };
    assert_eq!(report.stage, "resource.verify");
    assert!(matches!(
        report.cause,
        StartupCause::Failed(InitializationError::Application(
            RegistrationError::Duplicate("resource")
        ))
    ));
    assert!(report.cleanup.is_success());
    assert_eq!(*events.lock().unwrap(), ["cleanup"]);

    let scope_events = events.clone();
    let mut panicked = batter_core::startup::Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async move |scope| {
            record_cleanup(scope, scope_events);
            tokio::task::yield_now().await;
            if scope.stage("resource.panic").is_ok() {
                panic!("typed-initializer-panic-marker");
            }
            Ok::<u16, RegistrationError>(1)
        },
    )
    .start();
    let Err(StartupError::Failed(report)) = panicked.wait_with_output().await else {
        panic!("panicking initializer must not yield output")
    };
    let StartupCause::Panicked(payload) = &report.cause else {
        panic!("missing initializer panic")
    };
    payload
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"typed-initializer-panic-marker")
            );
        })
        .unwrap();
    assert!(report.cleanup.is_success());
    assert_eq!(*events.lock().unwrap(), ["cleanup", "cleanup"]);
}

async fn boxed_boundary(
    starting: &mut batter_core::startup::StartingSupervisor<
        InitializationError<ApplicationError>,
        batter_core::lifecycle::RunningSupervisor,
        u16,
    >,
) -> Result<u16, BoxError> {
    let (running, port) = starting.wait_with_output().await?;
    running.shutdown_checked().await?;
    Ok(port)
}

#[tokio::test]
async fn boxed_application_failures_propagate_with_question_mark_and_keep_concrete_causes() {
    let process = supervisor();
    let mut starting = batter_core::startup::Startup::protected(
        process,
        context(),
        cleanup_budget(),
        async |scope| {
            scope.stage("settings.parse")?;
            let port: u16 = "8080".parse()?;
            let boxed: Result<(), BoxError> = Ok(());
            boxed?;
            scope.stage("listener.bind")?;
            Err::<(), _>(io::Error::other("private-bind-marker"))?;
            Ok::<_, ApplicationError>(port)
        },
    )
    .start();
    let error = boxed_boundary(&mut starting).await.unwrap_err();
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains("private-bind-marker"), "{rendered}");

    let startup = error
        .downcast_ref::<StartupError<InitializationError<ApplicationError>>>()
        .expect("the protected startup error is boxed unchanged");
    let StartupError::Failed(report) = startup else {
        panic!("missing retained report")
    };
    let report: &ProtectedFailure = report;
    assert_eq!(report.stage, "listener.bind");
    let StartupCause::Failed(InitializationError::Application(application)) = &report.cause else {
        panic!("missing application failure")
    };
    let concrete = application.downcast_ref::<io::Error>().unwrap();
    assert_eq!(concrete.to_string(), "private-bind-marker");
    assert_eq!(application.to_string(), "application initialization failed");
    assert_eq!(
        format!("{application:?}"),
        "application initialization failed"
    );

    let mut source = std::error::Error::source(&*error);
    let mut reached = false;
    while let Some(cause) = source {
        reached |= cause.downcast_ref::<io::Error>().is_some();
        source = cause.source();
    }
    assert!(reached, "the source chain reaches the concrete cause");
}

#[tokio::test]
async fn unit_async_initializer_composes_with_protected_service_completion() {
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    let startup = batter_core::startup::Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async move |scope| {
            scope
                .registration()
                .register("worker", move |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    observed.store(true, Ordering::SeqCst);
                    shutdown.draining().await;
                    Ok(shutdown.stopped())
                })?;
            Ok::<_, RegistrationError>(())
        },
    );
    let owner = service::start(startup, NoDiagnostics);
    while !started.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    owner.request_shutdown();
    let completion = owner.wait().await;
    let ServiceOutcome::Shutdown(Ok(success)) = completion.service() else {
        panic!("unit async initializer must reach checked completion")
    };
    assert!(success.report().is_success());
}
