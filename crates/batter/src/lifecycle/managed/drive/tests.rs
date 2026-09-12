use super::*;
use std::{future::pending, sync::Mutex, time::Duration};
use tokio::time::Instant;

#[tokio::test(start_paused = true)]
async fn settlement_propagates_each_earlier_clock_even_during_a_stop_callback() {
    use std::task::{Context, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let shared = Arc::new(crate::lifecycle::state::Shared::new(true));
    let handle = crate::lifecycle::ShutdownHandle {
        shared: shared.clone(),
    };
    let signal = handle.signal();
    let first = Instant::now();
    tokio::time::advance(Duration::from_secs(5)).await;
    handle.request();
    let sent = Instant::now();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let mut stop = move |started| {
        observed.lock().unwrap().push(started);
        // Another native owner reveals an even earlier clock before this callback
        // returns. The acknowledgement must not swallow that unsent update.
        shared.request_since(first);
        started
    };
    let mut settlement = Guarded::new(Box::pin(pending::<SettlementEvidence>()));
    let mut outcome = ManagedOutcome::default();
    let (publication, _) = watch::channel(outcome.clone());
    let waiter = settle_with_clock(
        &mut settlement,
        &mut stop,
        Some(sent),
        &signal,
        &mut outcome,
        &publication,
    );
    tokio::pin!(waiter);
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    handle.shared.request_since(first + Duration::from_secs(2));
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        *calls.lock().unwrap(),
        [first + Duration::from_secs(2), first]
    );
}

struct JoinedNative;

impl crate::lifecycle::ManagedSettlement for JoinedNative {
    fn is_success(&self) -> bool {
        true
    }
    fn allows_dependency_cleanup(&self) -> bool {
        true
    }
}

#[tokio::test(start_paused = true)]
async fn repeated_stop_panic_is_retained_before_pending_settlement_finishes() {
    use crate::{cleanup::CleanupBudget, lifecycle::Supervisor, operation::OperationContext};
    use tokio::sync::oneshot;
    let second = Duration::from_secs(1);
    let mut process = Supervisor::new(
        ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap(),
    );
    process
        .on_cleanup("dependency", || async {
            panic!("pending native settlement must prevent dependency cleanup")
        })
        .unwrap();
    let (first_stop, first_called) = oneshot::channel();
    let (repeated_stop, repeated_called) = oneshot::channel();
    let (release, settling) = oneshot::channel();
    process
        .register_managed(
            "native",
            OperationContext::new(Duration::from_secs(10)).unwrap(),
            move |_| {
                let mut first_stop = Some(first_stop);
                let mut repeated_stop = Some(repeated_stop);
                Ok(ManagedComponent::new(
                    async { Ok(()) },
                    pending(),
                    move |started| {
                        if let Some(first_stop) = first_stop.take() {
                            first_stop.send(()).unwrap();
                            return started;
                        }
                        repeated_stop.take().unwrap().send(()).unwrap();
                        panic!("original repeated stop failure");
                    },
                    async move {
                        settling.await.unwrap();
                        JoinedNative
                    },
                ))
            },
        )
        .unwrap();
    let running = process.start();
    let handle = running.handle();
    handle.mark_ready();
    handle.wait_ready().await.unwrap();
    let native_started = Instant::now();
    tokio::time::advance(second).await;
    handle.request();
    first_called.await.unwrap();
    // Model another native component revealing its earlier stop timestamp.
    handle.shared.request_since(native_started);
    repeated_called.await.unwrap();
    let report = running.wait().await.unwrap();
    let record = &report.managed[0];
    let frozen = record.outcome.clone();
    release.send(()).unwrap();
    let eventual = record.observer.wait().await;
    assert!(eventual.finished);
    assert!(!frozen.finished);
    assert!(frozen.settlement.is_none());
    assert!(!frozen.allows_dependency_cleanup());
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(
        frozen.failures.len(),
        1,
        "already-observed stop panic missing from frozen report"
    );
    let ManagedFailure::Panicked(payload) = &frozen.failures[0] else {
        panic!("original stop panic was replaced");
    };
    payload
        .try_inspect(|value| {
            assert_eq!(
                value.downcast_ref::<&str>(),
                Some(&"original repeated stop failure")
            )
        })
        .expect("payload inspection is uncontended");
    assert_eq!(eventual.failures.len(), 1);
}
