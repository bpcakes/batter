use super::*;
use batter::{
    BoxError,
    command::{CommandFailure, check_command},
    startup::PanicPayload,
};
use std::{pin::Pin, task::Context};

#[derive(Debug, thiserror::Error)]
#[error("secret original failure")]
struct Original;

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

fn assert_payload(payload: &PanicPayload, expected: &str) {
    payload.inspect(|value| assert_eq!(value.downcast_ref::<&str>().copied(), Some(expected)));
}

#[tokio::test(start_paused = true)]
async fn question_mark_preserves_work_error_and_all_cleanup_errors() {
    let command = Command::new(context(), budget(), |scope| {
        Box::pin(async move {
            scope.stage("command.prepare").unwrap();
            for name in ["first", "second"] {
                scope
                    .reserve_cleanup(name)
                    .unwrap()
                    .register(|| async { Err(Box::new(Original) as BoxError) });
            }
            Err::<(), _>(Original)?;
            Ok::<_, Original>(())
        })
    })
    .start();
    let result = check_command(command.wait().await);
    let Err(CommandFailure::Report(report)) = result else {
        panic!("failure report missing")
    };
    let Err(CommandCause::Failed(original)) = &report.work else {
        panic!("original work error missing")
    };
    let observer = command.observer().wait().await.unwrap();
    let Err(CommandCause::Failed(same)) = &observer.work else {
        panic!("observer lost original")
    };
    assert!(std::ptr::eq(original, same));
    assert_eq!(report.stage, "command.prepare");
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(
        cleanup
            .records
            .iter()
            .map(|record| record.name)
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
    assert!(cleanup.records.iter().all(|record| {
        record.outcome == CleanupOutcome::Failed
            && record
                .error
                .as_ref()
                .unwrap()
                .downcast_ref::<Original>()
                .is_some()
    }));
    assert!(!format!("{report:?}").contains("secret"));
}

#[tokio::test(start_paused = true)]
async fn work_poll_and_future_destruction_panics_both_survive_finalization() {
    let closed = Arc::new(AtomicBool::new(false));
    let closing = closed.clone();
    let command = Command::new(context(), budget(), |scope| {
        scope
            .reserve_cleanup("resource")
            .unwrap()
            .register(move || async move {
                closing.store(true, Ordering::SeqCst);
                Ok(())
            });
        Box::pin(DropPanic::new(async {
            panic!("secret polling payload");
            #[allow(unreachable_code)]
            Ok::<_, Original>(())
        }))
    })
    .start();
    let report = command.wait().await.unwrap();
    let Err(CommandCause::Panicked(payload)) = &report.work else {
        panic!("work panic missing")
    };
    assert_payload(payload, "secret polling payload");
    assert_payload(
        report.destruction_panic.as_ref().unwrap(),
        "secret destructor payload",
    );
    assert!(closed.load(Ordering::SeqCst));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn factory_panic_still_finalizes_registered_resources() {
    let closed = Arc::new(AtomicBool::new(false));
    let closing = closed.clone();
    let command = Command::new(context(), budget(), |scope| {
        scope
            .reserve_cleanup("resource")
            .unwrap()
            .register(move || async move {
                closing.store(true, Ordering::SeqCst);
                Ok(())
            });
        panic!("secret factory payload");
        #[allow(unreachable_code)]
        Box::pin(async { Ok::<_, Original>(()) })
    })
    .start();
    let report = command.wait().await.unwrap();
    assert!(matches!(report.work, Err(CommandCause::Panicked(_))));
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn returned_value_is_retained_when_destruction_panics_and_cleanup_fails() {
    let command = Command::new(context(), budget(), |scope| {
        scope
            .reserve_cleanup("resource")
            .unwrap()
            .register(|| async { Err(Box::new(Original) as BoxError) });
        Box::pin(DropPanic::new(async { Ok::<_, Original>(42) }))
    })
    .start();
    let report = command.wait().await.unwrap();
    assert_eq!(*report.work.as_ref().unwrap(), 42);
    assert!(report.destruction_panic.is_some());
    assert_eq!(
        report.cleanup.as_ref().unwrap().records[0].outcome,
        CleanupOutcome::Failed
    );
    assert!(!report.is_success());
}
