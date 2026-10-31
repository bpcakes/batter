//! Refused or interrupted admission destroys blocking work uncalled and
//! never hands it to Tokio.

use crate::blocking_support::{Unrun, bulkhead, fresh, owner};
use batter_core::{
    admission::{Admission, AdmissionError, BlockingError},
    operation::Interruption,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    task::{Context, Waker},
    time::Duration,
};
use tokio::sync::OwnedSemaphorePermit;

#[tokio::test]
async fn reject_overload_destroys_work_uncalled() {
    let bulkhead = bulkhead(1);
    let _held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
    let probe = Unrun::default();
    assert!(matches!(
        bulkhead
            .run_blocking(&fresh(), Admission::Reject, probe.closure())
            .await,
        Err(BlockingError::NotAdmitted(AdmissionError::Overloaded))
    ));
    probe.assert_destroyed_uncalled();
}

#[tokio::test]
async fn precancelled_context_is_not_admitted_with_free_capacity() {
    let bulkhead = bulkhead(1);
    let owner = owner(10);
    owner.cancel();
    for admission in [Admission::Reject, Admission::Wait] {
        let probe = Unrun::default();
        assert!(
            matches!(
                bulkhead
                    .run_blocking(owner.context(), admission, probe.closure())
                    .await,
                Err(BlockingError::NotAdmitted(AdmissionError::Interrupted(
                    Interruption::Cancelled
                )))
            ),
            "{admission:?}"
        );
        probe.assert_destroyed_uncalled();
    }
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn wait_deadline_interrupts_admission_before_work_starts() {
    let bulkhead = bulkhead(1);
    let held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
    let context = owner(1).into_context();
    let probe = Unrun::default();
    assert!(matches!(
        bulkhead
            .run_blocking(&context, Admission::Wait, probe.closure())
            .await,
        Err(BlockingError::NotAdmitted(AdmissionError::Interrupted(
            Interruption::DeadlineExceeded
        )))
    ));
    probe.assert_destroyed_uncalled();
    drop(held);
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test]
async fn wait_cancellation_interrupts_admission_before_work_starts() {
    let bulkhead = bulkhead(1);
    let held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
    let owner = owner(10);
    let probe = Unrun::default();
    let waiter = tokio::spawn({
        let bulkhead = bulkhead.clone();
        let context = owner.context().clone();
        let work = probe.closure();
        async move { bulkhead.run_blocking(&context, Admission::Wait, work).await }
    });
    // One yield queues the waiter for the held permit.
    tokio::task::yield_now().await;
    owner.cancel();
    assert!(matches!(
        waiter.await.unwrap(),
        Err(BlockingError::NotAdmitted(AdmissionError::Interrupted(
            Interruption::Cancelled
        )))
    ));
    probe.assert_destroyed_uncalled();
    drop(held);
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn unpolled_call_and_dropped_admission_wait_do_no_work() {
    let bulkhead = bulkhead(1);
    let context = fresh();
    let unpolled = Unrun::default();
    let call = bulkhead.run_blocking(&context, Admission::Reject, unpolled.closure());
    drop(call);
    unpolled.assert_destroyed_uncalled();
    // The unpolled call took no permit.
    let held = bulkhead.enter(&context, Admission::Reject).await.unwrap();

    let dropped = Unrun::default();
    let waited = tokio::time::timeout(
        Duration::ZERO,
        bulkhead.run_blocking(&context, Admission::Wait, dropped.closure()),
    )
    .await;
    assert!(waited.is_err());
    dropped.assert_destroyed_uncalled();
    drop(held);
    tokio::task::yield_now().await;
    dropped.assert_destroyed_uncalled();
}

/// Queue `wait` for the held permit, then grant it and pass the one-second
/// deadline in one step. On the current-thread scheduler the woken waiter
/// runs before the driver next fires timers, so its deadline timer is unfired.
async fn granted_after_an_unfired_deadline<T: Send + 'static>(
    held: OwnedSemaphorePermit,
    wait: impl Future<Output = T> + Send + 'static,
) -> T {
    let waiter = tokio::spawn(wait);
    // One yield lets the waiter register its deadline timer and queue.
    tokio::task::yield_now().await;
    drop(held);
    tokio::time::advance(Duration::from_secs(2)).await;
    waiter.await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn admission_granted_after_an_unfired_deadline_never_starts_work() {
    let bulkhead = bulkhead(1);
    let held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
    let probe = Unrun::default();
    let result = granted_after_an_unfired_deadline(held, {
        let bulkhead = bulkhead.clone();
        let context = owner(1).into_context();
        let work = probe.closure();
        async move { bulkhead.run_blocking(&context, Admission::Wait, work).await }
    })
    .await;
    assert!(matches!(
        result,
        Err(BlockingError::NotAdmitted(AdmissionError::Interrupted(
            Interruption::DeadlineExceeded
        )))
    ));
    probe.assert_destroyed_uncalled();
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

/// Control for the recheck: if Tokio stops granting the permit before the
/// timer fires, this fails and the recheck test no longer exercises it.
#[tokio::test(start_paused = true)]
async fn enter_alone_can_grant_a_permit_after_an_unfired_deadline() {
    let bulkhead = bulkhead(1);
    let held = bulkhead.enter(&fresh(), Admission::Reject).await.unwrap();
    let context = owner(1).into_context();
    let permit = granted_after_an_unfired_deadline(held, {
        let bulkhead = bulkhead.clone();
        let context = context.clone();
        async move { bulkhead.enter(&context, Admission::Wait).await }
    })
    .await;
    assert!(permit.is_ok(), "{permit:?}");
    assert_eq!(context.check(), Err(Interruption::DeadlineExceeded));
}

#[test]
fn polling_outside_a_runtime_panics_before_admission() {
    let bulkhead = bulkhead(1);
    let context = fresh();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    // Reject admission needs no runtime: with the only permit held, admitting
    // first would return `NotAdmitted(Overloaded)` instead of panicking.
    let held = runtime
        .block_on(bulkhead.enter(&context, Admission::Reject))
        .unwrap();
    let probe = Unrun::default();
    let mut call = Box::pin(bulkhead.run_blocking(&context, Admission::Reject, probe.closure()));
    let polled = catch_unwind(AssertUnwindSafe(|| {
        call.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    }));
    assert!(polled.is_err());
    drop(call);
    probe.assert_destroyed_uncalled();
    drop(held);
    assert!(
        runtime
            .block_on(bulkhead.enter(&context, Admission::Reject))
            .is_ok()
    );
}

#[test]
fn polling_on_a_runtime_without_timers_panics_before_admission() {
    let bulkhead = bulkhead(1);
    let context = fresh();
    // No `enable_time`: the result wait could not set its deadline timer.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let probe = Unrun::default();
    let call = bulkhead.run_blocking(&context, Admission::Reject, probe.closure());
    let polled = catch_unwind(AssertUnwindSafe(|| runtime.block_on(call)));
    assert!(polled.is_err());
    // The closure was never handed to Tokio, and no permit stayed with it.
    probe.assert_destroyed_uncalled();
    assert!(
        runtime
            .block_on(bulkhead.enter(&context, Admission::Reject))
            .is_ok()
    );
}
