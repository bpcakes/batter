//! Listener removal on every completion path, and when nothing is installed.

use super::{
    Marker, assert_removed_once, budget, entries, failed_report, injected, parent, push,
    start_injected, succeeded,
};
use crate::{
    BoxError,
    command::{Command, CommandCause, CommandFuture, CommandScope, signals},
    operation::{Interruption, OperationContext, OperationOwner, RootDeadline},
};
use std::{
    convert::Infallible,
    future::{pending, poll_fn},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::{runtime::Handle, sync::oneshot, time::Instant};

fn panicking_factory(_: &mut CommandScope) -> CommandFuture<'_, (), Infallible> {
    panic!("factory-marker")
}

fn panicking_poll(_: &mut CommandScope) -> CommandFuture<'_, (), Infallible> {
    Box::pin(poll_fn(|_| -> Poll<Result<(), Infallible>> {
        panic!("poll-marker")
    }))
}

fn never_returns(_: &mut CommandScope) -> CommandFuture<'_, (), Infallible> {
    Box::pin(pending())
}

#[tokio::test]
async fn success_and_returned_error_remove_listeners_once() {
    let (source, sender, log) = injected();
    let command = Command::new(parent().into_context(), budget(), |_| {
        Box::pin(async { Ok::<_, Infallible>(5_u8) })
    });
    let success = succeeded(start_injected(command, source).finish().await);
    assert_eq!(success.into_value(), 5);
    assert_removed_once(&sender, &log);

    let (source, sender, log) = injected();
    let command = Command::new(parent().into_context(), budget(), |_| {
        Box::pin(async { Err::<(), _>(io::Error::other("work-marker")) })
    });
    let report = failed_report(start_injected(command, source).finish().await);
    let Err(CommandCause::Failed(error)) = &report.work else {
        panic!("returned error was not retained")
    };
    assert_eq!(error.to_string(), "work-marker");
    assert_eq!(report.interruption_after_work, None);
    assert_removed_once(&sender, &log);
}

#[tokio::test]
async fn factory_and_poll_panics_remove_listeners_once() {
    for (factory, stage) in [
        panicking_factory as fn(&mut CommandScope) -> CommandFuture<'_, (), Infallible>,
        panicking_poll,
    ]
    .into_iter()
    .zip(["factory", "poll"])
    {
        let (source, sender, log) = injected();
        let command = Command::new(parent().into_context(), budget(), factory);
        let report = failed_report(start_injected(command, source).finish().await);
        assert!(
            matches!(report.work, Err(CommandCause::Panicked(_))),
            "{stage} panic was not retained"
        );
        assert_eq!(report.interruption_after_work, None, "{stage}");
        assert!(report.destruction_panic.is_none(), "{stage}");
        assert_removed_once(&sender, &log);
    }
}

/// Paused time: with every task idle the clock reaches the work deadline.
#[tokio::test(start_paused = true)]
async fn deadline_expiry_removes_listeners_once() {
    let (source, sender, log) = injected();
    let parent = OperationOwner::new(Duration::from_secs(1)).unwrap();
    let command = Command::new(parent.context().clone(), budget(), never_returns);
    let report = failed_report(start_injected(command, source).finish().await);
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_removed_once(&sender, &log);
}

#[tokio::test]
async fn owner_cancel_during_work_removes_listeners_once() {
    let (source, sender, log) = injected();
    let (started, running) = oneshot::channel();
    let command = Command::new(parent().into_context(), budget(), move |_| {
        Box::pin(async move {
            started.send(()).map_err(|()| "test stopped waiting")?;
            pending::<Result<(), BoxError>>().await
        })
    });
    let owner = start_injected(command, source);
    running.await.expect("work started");
    owner.cancel();
    let report = failed_report(owner.finish().await);
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert_removed_once(&sender, &log);
}

#[tokio::test]
async fn owner_drop_during_work_removes_listeners_before_its_unclaimed_cleanup() {
    let (source, sender, log) = injected();
    let (started, running) = oneshot::channel();
    let (finalized, finalization) = oneshot::channel();
    let (hook_log, removal) = (log.clone(), sender.clone());
    let command = Command::new(parent().into_context(), budget(), move |scope| {
        Box::pin(async move {
            scope
                .reserve_cleanup("dropped.owner")?
                .register(move || async move {
                    push(&hook_log, "cleanup");
                    finalized
                        .send(removal.is_closed())
                        .map_err(|_| "cleanup observer dropped")?;
                    Ok::<(), BoxError>(())
                });
            started.send(()).map_err(|()| "test stopped waiting")?;
            pending::<Result<(), BoxError>>().await
        })
    });
    let owner = start_injected(command, source);
    running.await.expect("work started");
    drop(owner);
    let removed = finalization
        .await
        .expect("cleanup continued after owner drop");
    assert!(removed, "a cleanup hook could still receive a signal");
    assert_eq!(entries(&log), ["listeners-dropped", "cleanup"]);
    assert_removed_once(&sender, &log);
}

#[tokio::test]
async fn queued_delivery_path_removes_listeners_once() {
    let (source, sender, log) = injected();
    sender.send(()).unwrap();
    let command = Command::new(parent().into_context(), budget(), never_returns);
    let report = failed_report(start_injected(command, source).finish().await);
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert_removed_once(&sender, &log);
}

/// A factory whose destruction is logged and whose invocation is recorded.
fn probed(
    context: OperationContext,
    log: &super::Log,
    invoked: &Arc<AtomicBool>,
) -> Command<impl use<> + FnOnce(&mut CommandScope) -> CommandFuture<'_, (), Infallible> + Send> {
    let invoking = invoked.clone();
    let factory = Marker::new(log, "factory-destroyed");
    Command::new(context, budget(), move |_| {
        invoking.store(true, Ordering::SeqCst);
        Box::pin(async move {
            drop(factory);
            Ok::<_, Infallible>(())
        })
    })
}

#[tokio::test]
async fn installation_failure_destroys_the_factory_uninvoked_and_spawns_nothing() {
    let log = super::Log::default();
    let invoked = Arc::default();
    let command = probed(parent().into_context(), &log, &invoked);
    let alive = Handle::current().metrics().num_alive_tasks();
    let result = signals::start_with(command, || {
        Err::<signals::NoSignals, _>(io::Error::other("install-marker"))
    });
    // Synchronously, before this test yields to any spawned task.
    assert_eq!(entries(&log), ["factory-destroyed"]);
    assert_eq!(Handle::current().metrics().num_alive_tasks(), alive);
    let Err(error) = result else {
        panic!("the installation error was not returned")
    };
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), "install-marker");
    tokio::task::yield_now().await;
    assert!(!invoked.load(Ordering::SeqCst));
}

fn refuse_installation() -> io::Result<signals::NoSignals> {
    panic!("an interrupted context must not install listeners")
}

#[tokio::test]
async fn interrupted_contexts_skip_installation_and_record_their_interruption() {
    let cancelled = parent();
    cancelled.cancel();
    let expired = OperationOwner::at(RootDeadline::at(Instant::now()));
    for (owner, expected) in [
        (cancelled, Interruption::Cancelled),
        (expired, Interruption::DeadlineExceeded),
    ] {
        let log = super::Log::default();
        let invoked = Arc::default();
        let command = probed(owner.context().clone(), &log, &invoked);
        let started = signals::start_with(command, refuse_installation);
        let report = failed_report(started.expect("nothing to install").finish().await);
        assert!(
            matches!(report.work, Err(CommandCause::Interrupted(cause)) if cause == expected),
            "{expected:?}"
        );
        assert_eq!(report.interruption_after_work, None);
        assert!(
            report
                .cleanup
                .as_ref()
                .is_ok_and(|cleanup| cleanup.is_success())
        );
        assert!(!invoked.load(Ordering::SeqCst), "{expected:?}");
        assert_eq!(entries(&log), ["factory-destroyed"], "{expected:?}");
    }
}
