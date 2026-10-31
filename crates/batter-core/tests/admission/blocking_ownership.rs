//! Admitted blocking work keeps its permit until the closure returns or
//! unwinds, however its caller stops waiting.

use crate::blocking_support::{Gate, Unrun, bulkhead, fresh, gated, owner, parked};
use batter_core::{
    admission::{Admission, AdmissionError, BlockingError, Bulkhead},
    operation::Interruption,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// The gated closure still holds the only permit and has not returned.
async fn assert_still_held(bulkhead: &Bulkhead, gate: &Gate) {
    let probe = Unrun::default();
    assert!(matches!(
        bulkhead
            .run_blocking(&fresh(), Admission::Reject, probe.closure())
            .await,
        Err(BlockingError::NotAdmitted(AdmissionError::Overloaded))
    ));
    probe.assert_destroyed_uncalled();
    assert!(matches!(
        bulkhead.enter(&fresh(), Admission::Reject).await,
        Err(AdmissionError::Overloaded)
    ));
    assert!(!gate.returned());
}

/// Wait admission succeeds once the abandoned closure frees its permit.
async fn wait_for_capacity(bulkhead: &Bulkhead) {
    assert!(bulkhead.enter(&fresh(), Admission::Wait).await.is_ok());
}

/// The closure is a `T: 'static` value, so it moves into the caller task.
fn spawn_caller(
    bulkhead: &Bulkhead,
    context: batter_core::operation::OperationContext,
    work: impl FnOnce() -> u8 + Send + 'static,
) -> tokio::task::JoinHandle<Result<u8, BlockingError>> {
    let bulkhead = bulkhead.clone();
    tokio::spawn(async move {
        bulkhead
            .run_blocking(&context, Admission::Reject, work)
            .await
    })
}

#[tokio::test]
async fn cancelled_caller_keeps_capacity_until_the_closure_returns() {
    let bulkhead = bulkhead(1);
    let owner = owner(10);
    let (mut gate, work) = parked(7_u8);
    let caller = spawn_caller(&bulkhead, owner.context().clone(), work);
    gate.started().await;
    owner.cancel();
    assert!(matches!(
        caller.await.unwrap(),
        Err(BlockingError::Abandoned(Interruption::Cancelled))
    ));
    assert_still_held(&bulkhead, &gate).await;
    gate.release();
    wait_for_capacity(&bulkhead).await;
    assert!(gate.returned());
}

#[tokio::test]
async fn dropped_caller_keeps_capacity_until_the_closure_returns() {
    let bulkhead = bulkhead(1);
    let (mut gate, work) = parked(7_u8);
    let caller = spawn_caller(&bulkhead, fresh(), work);
    gate.started().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_still_held(&bulkhead, &gate).await;
    gate.release();
    wait_for_capacity(&bulkhead).await;
    // The closure finished after its caller was gone.
    assert!(gate.returned());
}

#[tokio::test(start_paused = true)]
async fn deadline_during_wait_abandons_the_result_not_the_work() {
    let bulkhead = bulkhead(1);
    let (mut gate, work) = parked(7_u8);
    let caller = spawn_caller(&bulkhead, owner(1).into_context(), work);
    gate.started().await;
    // An outstanding blocking task inhibits paused-time auto-advance on a
    // current-thread runtime, so the deadline is passed explicitly.
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        caller.await.unwrap(),
        Err(BlockingError::Abandoned(Interruption::DeadlineExceeded))
    ));
    assert_still_held(&bulkhead, &gate).await;
    gate.release();
    wait_for_capacity(&bulkhead).await;
    assert!(gate.returned());
}

#[test]
fn queued_admitted_work_holds_capacity_and_runs_after_its_caller_is_gone() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let bulkhead = bulkhead(1);
        // The occupant holds the only blocking thread and no permit.
        let (mut occupant, occupy) = parked(0_u8);
        let occupying = tokio::task::spawn_blocking(occupy);
        occupant.started().await;
        let owner = owner(10);
        let (mut gate, work) = parked(7_u8);
        let caller = spawn_caller(&bulkhead, owner.context().clone(), work);
        // One yield lets the caller admit and queue its closure.
        tokio::task::yield_now().await;
        assert_still_held(&bulkhead, &gate).await;
        owner.cancel();
        assert!(matches!(
            caller.await.unwrap(),
            Err(BlockingError::Abandoned(Interruption::Cancelled))
        ));
        assert_still_held(&bulkhead, &gate).await;
        occupant.release();
        assert_eq!(occupying.await.unwrap(), 0);
        // The queued closure starts with no caller left, holding its permit.
        gate.started().await;
        assert_still_held(&bulkhead, &gate).await;
        gate.release();
        wait_for_capacity(&bulkhead).await;
        assert!(gate.returned());
    });
}

/// Records whether it was destroyed while its thread was unwinding.
struct UnwindProbe(Arc<AtomicBool>);

impl Drop for UnwindProbe {
    fn drop(&mut self) {
        self.0.store(std::thread::panicking(), Ordering::SeqCst);
    }
}

#[tokio::test]
async fn abandoned_closure_panic_releases_capacity_only_after_unwinding() {
    let bulkhead = bulkhead(1);
    let owner = owner(10);
    let unwound = Arc::new(AtomicBool::new(false));
    let probe = UnwindProbe(unwound.clone());
    let (mut gate, work) = gated(move || -> u8 {
        let _probe = probe;
        panic!("abandoned blocking defect")
    });
    let caller = spawn_caller(&bulkhead, owner.context().clone(), work);
    gate.started().await;
    owner.cancel();
    assert!(matches!(
        caller.await.unwrap(),
        Err(BlockingError::Abandoned(Interruption::Cancelled))
    ));
    assert_still_held(&bulkhead, &gate).await;
    assert!(!unwound.load(Ordering::SeqCst));
    gate.release();
    wait_for_capacity(&bulkhead).await;
    assert!(unwound.load(Ordering::SeqCst));
    assert!(!gate.returned());
}

/// Negative control: the hand-written shape loses capacity while work runs.
#[tokio::test]
async fn awaiter_held_permit_is_released_while_its_blocking_work_runs() {
    let bulkhead = bulkhead(1);
    let (mut gate, work) = parked(7_u8);
    let caller = tokio::spawn({
        let bulkhead = bulkhead.clone();
        async move {
            let _permit = bulkhead.enter(&fresh(), Admission::Reject).await?;
            Ok::<_, AdmissionError>(tokio::task::spawn_blocking(work).await)
        }
    });
    gate.started().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    // Capacity is free again although the blocking work is still running.
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
    assert!(!gate.returned());
    gate.release();
}

/// Closed admission refuses `work` uncalled under either policy.
async fn assert_closed(bulkhead: &Bulkhead, admission: Admission) {
    let probe = Unrun::default();
    assert!(matches!(
        bulkhead
            .run_blocking(&fresh(), admission, probe.closure())
            .await,
        Err(BlockingError::NotAdmitted(AdmissionError::Closed))
    ));
    probe.assert_destroyed_uncalled();
}

#[tokio::test]
async fn closed_bulkhead_admits_nothing_new_while_running_work_keeps_its_permit() {
    let bulkhead = bulkhead(1);
    let (mut gate, work) = parked(7_u8);
    let running = spawn_caller(&bulkhead, fresh(), work);
    gate.started().await;
    let waiting_probe = Unrun::default();
    let waiting = tokio::spawn({
        let bulkhead = bulkhead.clone();
        let work = waiting_probe.closure();
        async move { bulkhead.run_blocking(&fresh(), Admission::Wait, work).await }
    });
    // One yield queues the waiter behind the running closure's permit.
    tokio::task::yield_now().await;
    bulkhead.close();
    assert!(matches!(
        waiting.await.unwrap(),
        Err(BlockingError::NotAdmitted(AdmissionError::Closed))
    ));
    waiting_probe.assert_destroyed_uncalled();
    assert_closed(&bulkhead, Admission::Reject).await;
    assert!(matches!(
        bulkhead.enter(&fresh(), Admission::Reject).await,
        Err(AdmissionError::Closed)
    ));
    assert!(!gate.returned());
    gate.release();
    assert_eq!(running.await.unwrap().unwrap(), 7);
    assert_closed(&bulkhead, Admission::Wait).await;
}
