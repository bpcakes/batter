//! A classified terminal value travels through one library-owned boundary.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "../../../test-support/capture.rs"]
mod capture;
#[path = "resolved_value/dispatch.rs"]
mod dispatch;

use batter_core::operation::{
    Interruption, OperationError, OperationOwner, RootDeadline,
    low_level::{ResolvedOutcome, run_resolved_value},
};
use capture::Capture;
use std::{cell::Cell, convert::Infallible, future::Future, rc::Rc, time::Duration};
use tracing::instrument::WithSubscriber;
#[cfg(feature = "metrics")]
#[path = "metrics/capture.rs"]
mod metric_capture;
#[cfg(feature = "metrics")]
use batter_core::telemetry::metrics::catalog;

#[tokio::test]
async fn every_terminal_classification_returns_its_owned_value_and_records_once() {
    // No Clone or Debug bound is needed for returned values.
    struct Payload(String);
    for (resolved, expected) in [
        (
            ResolvedOutcome::Succeeded(Payload("secret-success".into())),
            "succeeded",
        ),
        (
            ResolvedOutcome::Failed(Payload("secret-failure".into())),
            "failed",
        ),
        (
            ResolvedOutcome::Interrupted {
                reason: Interruption::Cancelled,
                value: Payload("secret-cancelled".into()),
            },
            "cancelled",
        ),
        (
            ResolvedOutcome::Interrupted {
                reason: Interruption::DeadlineExceeded,
                value: Payload("secret-deadline".into()),
            },
            "deadline_exceeded",
        ),
    ] {
        assert_eq!(format!("{resolved:?}"), expected);
        let capture = Capture::new();
        let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        let calls = Cell::new(0);
        let child = std::cell::RefCell::new(None);
        let value = run_resolved_value(
            owner.context(),
            "adapter.resolved",
            |scope| {
                *child.borrow_mut() = Some(scope);
                async { Err::<(), _>(std::io::Error::other("secret-original")) }
            },
            |result| {
                assert!(matches!(result, Err(OperationError::Failed(_))));
                calls.set(calls.get() + 1);
                resolved
            },
        )
        .with_subscriber(capture.dispatch.clone())
        .await;
        assert!(value.0.starts_with("secret-"));
        assert_eq!(calls.get(), 1);
        assert_eq!(
            child.borrow().as_ref().unwrap().check(),
            Err(Interruption::Cancelled)
        );
        assert!(owner.context().check().is_ok());
        let text = capture.text();
        let events: Vec<_> = text
            .lines()
            .filter_map(|line| {
                line.split_once("operation boundary finished")
                    .map(|(_, fields)| fields)
            })
            .collect();
        assert_eq!(events.len(), 1, "{text}");
        assert!(
            events[0].contains(&format!("outcome=\"{expected}\"")),
            "{text}"
        );
        assert!(!text.contains("secret-"), "{text}");
    }
}

#[tokio::test]
async fn preflight_interruption_resolves_without_invoking_work() {
    for reason in [Interruption::Cancelled, Interruption::DeadlineExceeded] {
        let owner = match reason {
            Interruption::Cancelled => {
                let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
                owner.cancel();
                owner
            }
            Interruption::DeadlineExceeded => {
                OperationOwner::at(RootDeadline::at(tokio::time::Instant::now()))
            }
        };
        let value = run_resolved_value(
            owner.context(),
            "adapter.preflight",
            |_| -> std::future::Ready<Result<(), Infallible>> { panic!("refused factory ran") },
            |result| {
                assert!(
                    matches!(result, Err(OperationError::Interrupted(actual)) if actual == reason)
                );
                ResolvedOutcome::Interrupted {
                    reason,
                    value: String::from("reply"),
                }
            },
        )
        .await;
        assert_eq!(value, "reply");
    }
}

#[test]
fn borrowed_non_send_values_remain_supported_and_unpolled_callbacks_are_inert() {
    let capture = Capture::new();
    capture.block_on(async {
        let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        let calls = Rc::new(Cell::new(0));
        let work_calls = calls.clone();
        let resolve_calls = calls.clone();
        let future = run_resolved_value(
            owner.context(),
            "adapter.unpolled",
            |_| {
                work_calls.set(1);
                async { Ok::<_, Infallible>(()) }
            },
            |_| {
                resolve_calls.set(2);
                ResolvedOutcome::Succeeded(())
            },
        );
        drop(future);
        assert_eq!(calls.get(), 0);
        let mut text = String::from("borrowed");
        let size = run_resolved_value(
            owner.context(),
            "adapter.borrowed",
            |_| async {
                text.push_str(" value");
                Ok::<_, Infallible>(text.len())
            },
            |result| {
                calls.set(calls.get() + 1);
                ResolvedOutcome::Succeeded(result.unwrap())
            },
        )
        .await;
        assert_eq!(size, "borrowed value".len());
        assert_eq!(calls.get(), 1);
    });
    assert!(!capture.text().contains("adapter.unpolled"));
}

#[test]
fn send_inputs_keep_a_send_future() {
    fn send(_: impl Future + Send) {}
    let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
    send(run_resolved_value(
        owner.context(),
        "adapter.send",
        |_| std::future::ready(Ok::<_, Infallible>(String::from("reply"))),
        |result| ResolvedOutcome::Succeeded(result.unwrap()),
    ));
}

#[tokio::test]
async fn work_and_resolver_panics_propagate_and_do_not_publish_a_resolved_value() {
    #[derive(Clone, Copy)]
    enum PanicAt {
        Work,
        Resolver,
    }
    for phase in [PanicAt::Work, PanicAt::Resolver] {
        let capture = Capture::new();
        let child = std::sync::Arc::new(std::sync::Mutex::new(None));
        let saved = child.clone();
        // The task panics in both phases, so an assertion inside it proves nothing.
        let resolver_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let resolving = resolver_ran.clone();
        let task = tokio::spawn(
            async move {
                let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
                run_resolved_value(
                    owner.context(),
                    "adapter.panic",
                    |scope| async move {
                        *saved.lock().unwrap() = Some(scope);
                        if matches!(phase, PanicAt::Work) {
                            panic!("work panicked");
                        }
                        Ok::<_, Infallible>(())
                    },
                    move |_| -> ResolvedOutcome<()> {
                        resolving.store(true, std::sync::atomic::Ordering::SeqCst);
                        panic!("resolver panicked");
                    },
                )
                .await
            }
            .with_subscriber(capture.dispatch.clone()),
        );
        assert!(task.await.unwrap_err().is_panic());
        assert_eq!(
            resolver_ran.load(std::sync::atomic::Ordering::SeqCst),
            matches!(phase, PanicAt::Resolver),
            "a work panic must not invoke the resolver"
        );
        assert_eq!(
            child.lock().unwrap().as_ref().unwrap().check(),
            Err(Interruption::Cancelled)
        );
        let text = capture.text();
        assert_eq!(
            text.matches("operation boundary finished").count(),
            1,
            "{text}"
        );
        assert!(text.contains("outcome=\"dropped\""), "{text}");
    }
}

#[cfg(feature = "metrics")]
#[tokio::test]
async fn classified_values_record_one_operation_metric_with_the_resolved_outcome() {
    use batter_core::telemetry::metrics::{OPERATION_COMPLETIONS, OPERATION_DURATION, facade};
    let capture = metric_capture::Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
    for (outcome, expected) in [
        (ResolvedOutcome::Succeeded(()), "succeeded"),
        (ResolvedOutcome::Failed(()), "failed"),
        (
            ResolvedOutcome::Interrupted {
                reason: Interruption::Cancelled,
                value: (),
            },
            "cancelled",
        ),
        (
            ResolvedOutcome::Interrupted {
                reason: Interruption::DeadlineExceeded,
                value: (),
            },
            "deadline_exceeded",
        ),
    ] {
        run_resolved_value(
            owner.context(),
            "adapter.metrics",
            |_| async { Ok::<_, Infallible>(()) },
            |_| outcome,
        )
        .await;
        assert_eq!(
            capture.count(
                OPERATION_COMPLETIONS,
                &[("operation", "adapter.metrics"), ("outcome", expected)]
            ),
            1.0
        );
    }
    assert_eq!(capture.samples(OPERATION_COMPLETIONS).len(), 4);
    assert_eq!(capture.samples(OPERATION_DURATION).len(), 4);
}
