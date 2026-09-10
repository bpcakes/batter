#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;

use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupStack, SkipReason},
    lifecycle::{ProcessTaskError, ShutdownBudget, Supervisor, TaskOutcome},
};
use std::{
    future::pending,
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;
use tracing::{Instrument, instrument::WithSubscriber};

#[path = "scoped_owned_tasks/filtered.rs"]
mod filtered;

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Logs {
    dispatch: tracing::Dispatch,
    output: Arc<Mutex<Vec<u8>>>,
}

impl Logs {
    fn new() -> Self {
        Self::with_filter("info")
    }

    fn with_filter(filter: &str) -> Self {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = Buffer(output.clone());
        Self {
            dispatch: crate::test_dispatch::new(
                tracing_subscriber::fmt()
                    .with_writer(move || writer.clone())
                    .with_ansi(false)
                    .without_time()
                    .with_env_filter(filter)
                    .finish(),
            ),
            output,
        }
    }

    fn within<T>(&self, scope: &'static str, action: impl FnOnce() -> T) -> T {
        tracing::dispatcher::with_default(&self.dispatch, || {
            let span = tracing::info_span!("test.scope", scope);
            let _entered = span.enter();
            action()
        })
    }

    fn text(&self) -> String {
        String::from_utf8(self.output.lock().unwrap().clone()).unwrap()
    }

    fn one_event(&self, text: &str, scope: &str) -> String {
        let output = self.text();
        let events = output
            .lines()
            .filter(|line| line.contains(text))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1, "expected one {text}: {output}");
        assert!(
            events[0].contains(&format!("scope=\"{scope}\"")),
            "{}",
            events[0]
        );
        events[0].to_owned()
    }
}

struct DropMarker {
    name: &'static str,
    dropped: Option<oneshot::Sender<()>>,
}

impl Drop for DropMarker {
    fn drop(&mut self) {
        tracing::info!(marker = self.name, "owned capture dropped");
        if let Some(dropped) = self.dropped.take() {
            // A cancelled observer does not change the captured resource's Drop.
            let _ = dropped.send(());
        }
    }
}

fn shutdown_budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        Duration::ZERO,
        Duration::ZERO,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap()
}

fn assert_cancelled(error: &BoxError) {
    let error = error.downcast_ref::<tokio::task::JoinError>().unwrap();
    assert!(error.is_cancelled(), "{error}");
    assert!(!error.is_panic(), "{error}");
}

#[tokio::test(flavor = "current_thread")]
async fn critical_abort_drops_captures_under_driver_subscriber_and_retains_cancelled_join() {
    let ambient = Logs::new();
    // Current-thread execution deliberately drops aborted futures outside their
    // scoped poll dispatch, while a different registry is active on this thread.
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let mut supervisor = Supervisor::new(shutdown_budget());
    let (started, ready) = oneshot::channel();
    supervisor
        .register("critical.worker", |signal| async move {
            let _capture = DropMarker {
                name: "critical.capture",
                dropped: None,
            };
            signal.mark_started();
            started.send(()).unwrap();
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();
    supervisor.handle().mark_ready();
    let running = driver.within("driver", || supervisor.start());
    ready.await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.abort_requested, ["critical.worker"]);
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Aborted);
    assert_cancelled(report.tasks[0].error.as_ref().unwrap());
    let event = driver.one_event("marker=\"critical.capture\"", "driver");
    assert!(event.contains("batter.task"), "{event}");
    driver.one_event("task failure observed", "driver");
    assert!(
        !ambient.text().contains("critical.capture"),
        "{}",
        ambient.text()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn finite_abort_keeps_submitter_context_while_driver_and_skipped_cleanup_keep_their_owner() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let request = Logs::new();
    let mut supervisor = Supervisor::with_process_capacity(shutdown_budget(), 1).unwrap();
    let process = supervisor.process_handle().unwrap();
    let cleanup_invoked = Arc::new(AtomicBool::new(false));
    let invoked = cleanup_invoked.clone();
    let capture = DropMarker {
        name: "cleanup.capture",
        dropped: None,
    };
    supervisor
        .on_cleanup("dependency.close", move || async move {
            invoked.store(true, Ordering::SeqCst);
            drop(capture);
            Ok(())
        })
        .unwrap();
    supervisor.handle().mark_ready();
    let running = driver.within("driver", || supervisor.start());
    running.handle().wait_ready().await.unwrap();
    let (started, ready) = oneshot::channel();
    let receipt = request.within("request", || {
        process
            .try_spawn("finite.work", |_| async move {
                let _capture = DropMarker {
                    name: "finite.capture",
                    dropped: None,
                };
                started.send(()).unwrap();
                pending::<Result<(), io::Error>>().await
            })
            .unwrap()
    });
    ready.await.unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.abort_requested, ["finite.work"]);
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Aborted);
    assert_cancelled(report.tasks[0].error.as_ref().unwrap());
    assert!(matches!(
        receipt.wait().await,
        Err(ProcessTaskError::Terminated)
    ));
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].name, "dependency.close");
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!cleanup_invoked.load(Ordering::SeqCst));
    let event = request.one_event("marker=\"finite.capture\"", "request");
    assert!(event.contains("batter.process_task"), "{event}");
    assert!(!event.contains("scope=\"driver\""), "{event}");
    driver.one_event("shutdown drain started", "driver");
    driver.one_event("task failure observed", "driver");
    driver.one_event("cleanup skipped", "driver");
    driver.one_event("marker=\"cleanup.capture\"", "driver");
    assert!(
        !driver.text().contains("finite.capture"),
        "{}",
        driver.text()
    );
    assert!(
        !request.text().contains("cleanup.capture"),
        "{}",
        request.text()
    );
    assert!(
        !ambient.text().contains("owned capture dropped"),
        "{}",
        ambient.text()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn aborting_close_preserves_context_for_pending_captures_and_the_aborted_active_hook() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let owner = Logs::new();
    let dependency_invoked = Arc::new(AtomicBool::new(false));
    let invoked = dependency_invoked.clone();
    let dependency_capture = DropMarker {
        name: "pending.capture",
        dropped: None,
    };
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("dependency.close", move || async move {
            invoked.store(true, Ordering::SeqCst);
            drop(dependency_capture);
            Ok(())
        })
        .unwrap();
    let (started, ready) = oneshot::channel();
    let (dropped, ended) = oneshot::channel();
    cleanup
        .push("active.close", || async move {
            let _capture = DropMarker {
                name: "active.capture",
                dropped: Some(dropped),
            };
            started.send(()).unwrap();
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();
    let second = Duration::from_secs(1);
    let close = owner.within("cleanup", || {
        tokio::spawn(
            cleanup
                .close(CleanupBudget::new(second, second, second).unwrap())
                .in_current_span()
                .with_current_subscriber(),
        )
    });
    ready.await.unwrap();
    close.abort();
    let error = close.await.unwrap_err();
    assert!(error.is_cancelled(), "{error}");
    assert!(!error.is_panic(), "{error}");
    tokio::time::timeout(second, ended).await.unwrap().unwrap();
    assert!(!dependency_invoked.load(Ordering::SeqCst));
    owner.one_event(
        "cleanup driver dropped before hook result was observed",
        "cleanup",
    );
    owner.one_event("cleanup stack dropped without close", "cleanup");
    owner.one_event("marker=\"pending.capture\"", "cleanup");
    let active = owner.one_event("marker=\"active.capture\"", "cleanup");
    assert!(active.contains("batter.cleanup"), "{active}");
    assert!(!ambient.text().contains("capture"), "{}", ambient.text());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn timed_out_hook_drops_under_its_context_and_the_next_dependency_stays_under_cleanup_owner()
{
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let owner = Logs::new();
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("dependency.close", || async {
            tracing::info!("dependency finalizer invoked");
            Ok(())
        })
        .unwrap();
    cleanup
        .push("active.close", || async {
            let _capture = DropMarker {
                name: "timedout.capture",
                dropped: None,
            };
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();
    let close = owner.within("cleanup", || {
        tokio::spawn(
            cleanup
                .close(
                    CleanupBudget::new(
                        Duration::from_secs(1),
                        Duration::from_millis(5),
                        Duration::from_secs(1),
                    )
                    .unwrap(),
                )
                .in_current_span()
                .with_current_subscriber(),
        )
    });
    let report = close.await.unwrap();
    assert_eq!(report.records.len(), 2);
    assert_eq!(report.records[0].name, "active.close");
    assert_eq!(report.records[0].outcome, CleanupOutcome::TimedOut);
    assert_cancelled(report.records[0].error.as_ref().unwrap());
    assert_eq!(report.records[1].name, "dependency.close");
    assert_eq!(report.records[1].outcome, CleanupOutcome::Succeeded);
    assert!(report.skipped.is_empty());
    let event = owner.one_event("marker=\"timedout.capture\"", "cleanup");
    assert!(event.contains("batter.cleanup"), "{event}");
    owner.one_event("dependency finalizer invoked", "cleanup");
    assert!(
        !ambient.text().contains("timedout.capture"),
        "{}",
        ambient.text()
    );
}
