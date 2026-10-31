//! A blocking run keeps the tracing dispatcher captured at its first poll for
//! the closure's execution and for its destruction, and the runtime checked at
//! that poll for its result wait, however later polls or the destroying caller
//! are scoped.

use crate::blocking_support::{Logs, bulkhead, fresh};
use batter_core::admission::Admission;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};
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
    let held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
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
    let _held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
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
        .block_on(bulkhead.enter(&context, Admission::Reject))
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
        later
            .block_on(bulkhead.enter(&context, Admission::Reject))
            .is_ok()
    );
}
