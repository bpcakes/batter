//! Completed blocking work delivers its value, its concrete domain error or
//! its retained panic after freeing capacity, under the caller's tracing.

use crate::blocking_support::{Logs, bulkhead, fresh};
use batter_core::admission::{Admission, BlockingError};
use std::io;
use tracing::{Instrument, instrument::WithSubscriber};

#[tokio::test]
async fn closure_panic_is_reported_with_its_original_payload() {
    let bulkhead = bulkhead(1);
    let result = bulkhead
        .run_blocking(&fresh(), Admission::Reject, || -> u8 {
            panic!("blocking defect")
        })
        .await;
    let Err(BlockingError::Panicked(payload)) = result else {
        panic!("a panicking closure must report Panicked");
    };
    assert_eq!(
        payload.try_inspect(|value| value.downcast_ref::<&str>().copied()),
        Ok(Some("blocking defect"))
    );
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProbePayload(u8);

#[tokio::test]
async fn opaque_panic_payload_is_retained() {
    let result = bulkhead(1)
        .run_blocking(&fresh(), Admission::Reject, || -> u8 {
            std::panic::panic_any(ProbePayload(7))
        })
        .await;
    let Err(BlockingError::Panicked(payload)) = result else {
        panic!("a panicking closure must report Panicked");
    };
    assert_eq!(
        payload.try_inspect(|value| value.downcast_ref::<ProbePayload>().copied()),
        Ok(Some(ProbePayload(7)))
    );
}

#[tokio::test]
async fn returns_a_value_from_a_blocking_thread_and_frees_capacity_first() {
    let bulkhead = bulkhead(1);
    let caller = std::thread::current().id();
    let worker = bulkhead
        .run_blocking(&fresh(), Admission::Reject, || std::thread::current().id())
        .await
        .unwrap();
    assert_ne!(worker, caller);
    // The permit was released before the value was delivered.
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test]
async fn fallible_closure_keeps_its_concrete_domain_error() {
    let bulkhead = bulkhead(1);
    let result = bulkhead
        .run_blocking(&fresh(), Admission::Reject, || {
            Err::<u8, io::Error>(io::Error::other("domain refusal"))
        })
        .await;
    let Ok(Err(error)) = result else {
        panic!("a domain error stays inside Ok");
    };
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), "domain refusal");
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test]
async fn blocking_work_runs_under_the_callers_dispatcher_and_span() {
    let logs = Logs::new();
    let bulkhead = bulkhead(1);
    let context = fresh();
    let result = async {
        // Created inside the subscriber, so it is the caller's live span.
        let span = tracing::info_span!("caller.request");
        async {
            bulkhead
                .run_blocking(&context, Admission::Reject, || {
                    tracing::info!(marker = "blocking.event", "blocking work ran");
                })
                .await
        }
        .instrument(span)
        .await
    }
    .with_subscriber(logs.dispatch.clone())
    .await;
    assert!(result.is_ok());
    let text = logs.text();
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains("marker=\"blocking.event\""))
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(lines[0].contains("caller.request"), "{text}");
    assert!(lines[0].contains("batter.blocking_work"), "{text}");
    assert!(text.contains("batter.blocking_wait"), "{text}");
    // The child span closed under the caller's dispatcher, so the caller's
    // own span was released and closed too.
    assert_eq!(logs.closes_of("batter.blocking_work"), 1, "{text}");
    assert_eq!(logs.closes_of("caller.request"), 1, "{text}");
}

fn assert_send<T: Send>(_: &T) {}

#[test]
fn run_blocking_future_is_send() {
    let bulkhead = bulkhead(1);
    let context = fresh();
    let call = bulkhead.run_blocking(&context, Admission::Reject, || 1_u8);
    assert_send(&call);
}
