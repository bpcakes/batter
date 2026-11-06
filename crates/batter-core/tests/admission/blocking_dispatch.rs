//! A blocking run keeps the tracing dispatcher captured at its first poll for
//! the closure's execution and for its destruction, and the runtime checked at
//! that poll for its result wait, however later polls or the destroying caller
//! are scoped.

use crate::blocking_support::{Logs, bulkhead, fresh, gated, owner};
use crate::bulkhead_testing::BulkheadTestExt;
use batter_core::{
    admission::{Admission, BlockingError},
    operation::Interruption,
};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::sync::oneshot;
use tracing::instrument::WithSubscriber;

fn markers(logs: &Logs, marker: &str) -> usize {
    let field = format!("marker=\"{marker}\"");
    logs.text()
        .lines()
        .filter(|line| line.contains(&field))
        .count()
}

#[tokio::test]
async fn blocking_work_runs_under_the_dispatcher_captured_at_first_poll() {
    let first = Logs::new();
    let later = Logs::new();
    let bulkhead = bulkhead(1);
    let held = bulkhead.hold(&fresh(), Admission::Reject).await.unwrap();
    let context = fresh();
    let mut call = Box::pin(bulkhead.run_blocking(&context, Admission::Wait, || {
        tracing::info!(marker = "blocking.event", "blocking work ran");
    }));
    // The first poll queues for the held permit under `first`.
    let pending = tracing::dispatcher::with_default(&first.dispatch, || {
        call.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    });
    assert!(pending);
    drop(held);
    // Admission and handoff happen in later polls scoped to `later`.
    assert!(call.with_subscriber(later.dispatch.clone()).await.is_ok());
    assert_eq!(markers(&first, "blocking.event"), 1, "{}", first.text());
    assert_eq!(markers(&later, "blocking.event"), 0, "{}", later.text());
}

/// Logs when the closure that captured it is destroyed.
struct Announce;

impl Drop for Announce {
    fn drop(&mut self) {
        tracing::info!(marker = "capture.dropped", "closure capture destroyed");
    }
}

#[tokio::test]
async fn a_call_destroyed_while_waiting_for_admission_destroys_work_under_its_first_dispatcher() {
    let first = Logs::new();
    let later = Logs::new();
    let bulkhead = bulkhead(1);
    let _held = bulkhead.hold(&fresh(), Admission::Reject).await.unwrap();
    let context = fresh();
    let announce = Announce;
    let mut call = Box::pin(bulkhead.run_blocking(&context, Admission::Wait, move || {
        drop(announce);
        0_u8
    }));
    let pending = tracing::dispatcher::with_default(&first.dispatch, || {
        call.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    });
    assert!(pending);
    tracing::dispatcher::with_default(&later.dispatch, || drop(call));
    assert_eq!(markers(&first, "capture.dropped"), 1, "{}", first.text());
    assert_eq!(markers(&later, "capture.dropped"), 0, "{}", later.text());
}

#[test]
fn a_wait_call_finished_elsewhere_keeps_the_runtime_checked_at_first_poll() {
    let bulkhead = bulkhead(1);
    let context = fresh();
    let first = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let held = first
        .block_on(bulkhead.hold(&context, Admission::Reject))
        .unwrap();
    let mut call = Box::pin(bulkhead.run_blocking(&context, Admission::Wait, || 7_u8));
    // The first poll checks `first` and queues for the held permit.
    let pending = first.block_on(std::future::poll_fn(|cx| {
        Poll::Ready(call.as_mut().poll(cx).is_pending())
    }));
    assert!(pending);
    drop(held);
    // No `enable_time`: a result-wait timer built from this context would
    // panic after the closure was already handed to Tokio.
    let later = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert_eq!(later.block_on(call).unwrap(), 7);
    assert!(
        first
            .block_on(bulkhead.hold(&context, Admission::Reject))
            .is_ok()
    );
}

/// An outcome Tokio destroys after its caller abandoned it. It holds a span
/// created inside the closure, logs inside that span, closes it and only then
/// reports its destruction, so every close it causes is already logged.
struct Leftover {
    span: tracing::Span,
    destroyed: Option<oneshot::Sender<()>>,
}

impl Leftover {
    fn new(destroyed: oneshot::Sender<()>) -> Self {
        Self {
            span: tracing::info_span!("abandoned.outcome"),
            destroyed: Some(destroyed),
        }
    }
}

impl Drop for Leftover {
    fn drop(&mut self) {
        self.span.in_scope(|| {
            tracing::info!(marker = "outcome.dropped", "abandoned outcome destroyed");
        });
        drop(std::mem::replace(&mut self.span, tracing::Span::none()));
        if let Some(destroyed) = self.destroyed.take() {
            let _ = destroyed.send(());
        }
    }
}

/// First-polls a gated closure under `first` inside a caller span, abandons
/// its wait under `later`, then releases the closure and waits until Tokio
/// destroys the undelivered outcome that `finish` builds from a `Leftover`.
async fn abandon_then_destroy<T: Send + 'static>(
    finish: impl FnOnce(Leftover) -> T + Send + 'static,
) -> (Logs, Logs) {
    let first = Logs::new();
    let later = Logs::new();
    let bulkhead = bulkhead(1);
    let owner = owner(10);
    let (destroyed, destroyed_rx) = oneshot::channel();
    let (mut gate, work) = gated(move || finish(Leftover::new(destroyed)));
    let mut call = Box::pin(bulkhead.run_blocking(owner.context(), Admission::Reject, work));
    // The caller span is dropped here, so only the closure's spans keep it open.
    let handed_off = tracing::dispatcher::with_default(&first.dispatch, || {
        tracing::info_span!("caller.request").in_scope(|| {
            call.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        })
    });
    assert!(handed_off);
    gate.started().await;
    owner.cancel();
    assert!(matches!(
        call.with_subscriber(later.dispatch.clone()).await,
        Err(BlockingError::Abandoned(Interruption::Cancelled))
    ));
    // The join handle is gone, so Tokio destroys the outcome on the blocking
    // thread, which has no default dispatcher.
    gate.release();
    tokio::time::timeout(Duration::from_secs(30), destroyed_rx)
        .await
        .expect("the abandoned outcome is destroyed")
        .expect("the abandoned outcome reports its destruction");
    (first, later)
}

fn assert_destroyed_under_first(first: &Logs, later: &Logs) {
    let text = first.text();
    let dropped: Vec<_> = text
        .lines()
        .filter(|line| line.contains("marker=\"outcome.dropped\""))
        .collect();
    assert_eq!(dropped.len(), 1, "{text}");
    assert!(dropped[0].contains("caller.request"), "{text}");
    assert!(dropped[0].contains("batter.blocking_work"), "{text}");
    assert_eq!(markers(later, "outcome.dropped"), 0, "{}", later.text());
    for span in [
        "abandoned.outcome",
        "batter.blocking_work",
        "caller.request",
    ] {
        assert_eq!(first.closes_of(span), 1, "{span} in {text}");
    }
}

#[tokio::test]
async fn an_abandoned_value_is_destroyed_under_the_first_poll_dispatcher() {
    let (first, later) = abandon_then_destroy(|leftover| leftover).await;
    assert_destroyed_under_first(&first, &later);
}

#[tokio::test]
async fn an_abandoned_panic_payload_is_destroyed_under_the_first_poll_dispatcher() {
    let (first, later) =
        abandon_then_destroy(|leftover| -> u8 { std::panic::panic_any(leftover) }).await;
    assert_destroyed_under_first(&first, &later);
}
