//! Adversarial authority, cancellation and observation controls.

use super::*;
use crate::transport::Acknowledged;
use futures_util::{FutureExt, poll};
use std::{cell::Cell, future::pending, time::Duration};

#[tokio::test]
async fn parallel_threads_can_claim_only_one_dispatch() {
    let mut owner = Owner::default();
    let gate = owner.gate();
    let outcome = owner
        .run(Instant::now() + Duration::from_secs(2), || async {
            let barrier = std::sync::Barrier::new(8);
            std::thread::scope(|scope| {
                let threads: Vec<_> = (0..8)
                    .map(|_| {
                        scope.spawn(|| {
                            barrier.wait();
                            gate.claim()
                        })
                    })
                    .collect();
                let mut claimed: Vec<_> = threads
                    .into_iter()
                    .filter_map(|thread| thread.join().unwrap())
                    .collect();
                assert_eq!(claimed.len(), 1);
                claimed.pop().unwrap().fail(ExportFailure::PayloadTooLarge);
            });
        })
        .await;
    assert_eq!(
        outcome,
        ExportOutcome::Failed(ExportFailure::PayloadTooLarge)
    );
}

#[tokio::test(start_paused = true)]
async fn unpolled_and_encoding_cancelled_runs_never_leave_authority() {
    let mut owner = Owner::default();
    let gate = owner.gate();
    let invoked = Cell::new(false);
    let future = owner.run(Instant::now() + Duration::from_secs(1), || {
        invoked.set(true);
        pending::<()>()
    });
    drop(future);
    assert!(!invoked.get());
    assert!(gate.claim().is_none());

    let mut future = Box::pin(owner.run(Instant::now() + Duration::from_secs(1), pending::<()>));
    assert!(poll!(future.as_mut()).is_pending());
    drop(future);
    assert!(gate.claim().is_none());
    assert_eq!(
        owner
            .run(Instant::now() + Duration::from_secs(1), || async {})
            .await,
        ExportOutcome::Failed(ExportFailure::Encoding)
    );
}

#[tokio::test(start_paused = true)]
async fn claims_are_single_use_and_expired_permission_cannot_start_sending() {
    let mut owner = Owner::default();
    let gate = owner.gate();
    let deadline = Instant::now() + Duration::from_secs(1);
    let outcome = owner
        .run(deadline, || async {
            let first = gate.claim().unwrap();
            assert!(gate.claim().is_none());
            tokio::time::advance(Duration::from_secs(2)).await;
            assert!(first.send().is_none());
        })
        .await;
    assert_eq!(
        outcome,
        ExportOutcome::Failed(ExportFailure::HeadersTimedOut)
    );
    assert!(gate.claim().is_none());
}

#[tokio::test(start_paused = true)]
async fn phase_timeouts_and_typed_results_win_the_outer_deadline_race() {
    for phase in 0..4 {
        let mut owner = Owner::default();
        let gate = owner.gate();
        let outcome = owner
            .run(Instant::now() + Duration::from_secs(1), || async {
                match phase {
                    0 => {}
                    1 => {
                        let _sending = gate.claim().unwrap().send().unwrap();
                    }
                    2 => {
                        let _receiving = gate.claim().unwrap().send().unwrap().receive().unwrap();
                    }
                    3 => {
                        assert!(
                            gate.claim()
                                .unwrap()
                                .send()
                                .unwrap()
                                .receive()
                                .unwrap()
                                .complete(Ok(Acknowledged))
                        );
                    }
                    _ => unreachable!(),
                }
                pending::<()>().await;
            })
            .await;
        assert_eq!(
            outcome,
            match phase {
                0 | 1 => ExportOutcome::Failed(ExportFailure::HeadersTimedOut),
                2 => ExportOutcome::Failed(ExportFailure::BodyTimedOut),
                _ => ExportOutcome::Acknowledged,
            }
        );
        assert!(gate.claim().is_none());
    }
    let mut owner = Owner::default();
    let gate = owner.gate();
    let outcome = owner
        .run(Instant::now() + Duration::from_secs(1), || async {
            gate.claim()
                .unwrap()
                .send()
                .unwrap()
                .fail(ExportFailure::Refused);
            pending::<()>().await;
        })
        .await;
    assert_eq!(outcome, ExportOutcome::Failed(ExportFailure::Refused));
}

#[tokio::test(start_paused = true)]
async fn cancellation_revokes_old_capabilities_without_touching_the_next_attempt() {
    enum Old {
        Preparing(Preparing),
        Sending(Sending),
        Receiving(Receiving),
    }
    for phase in 0..3 {
        let mut owner = Owner::default();
        let gate = owner.gate();
        let old_deadline = Instant::now() + Duration::from_secs(1);
        let mut future = Box::pin(owner.run(old_deadline, pending::<()>));
        assert!(poll!(future.as_mut()).is_pending());
        let preparing = gate.claim().unwrap();
        let old = match phase {
            0 => Old::Preparing(preparing),
            1 => Old::Sending(preparing.send().unwrap()),
            _ => Old::Receiving(preparing.send().unwrap().receive().unwrap()),
        };
        drop(future);
        assert!(gate.claim().is_none());
        let new_deadline = Instant::now() + Duration::from_secs(10);
        let outcome = owner
            .run(new_deadline, || async {
                match old {
                    Old::Preparing(old) => assert!(old.send().is_none()),
                    Old::Sending(old) => assert!(old.receive().is_none()),
                    Old::Receiving(old) => assert!(!old.complete(Ok(Acknowledged))),
                }
                let sending = gate.claim().unwrap().send().unwrap();
                assert_eq!(sending.deadline(), new_deadline);
                sending.fail(ExportFailure::Refused);
            })
            .await;
        assert_eq!(outcome, ExportOutcome::Failed(ExportFailure::Refused));
    }
}

#[tokio::test]
async fn factory_panic_revokes_authority_before_a_later_run() {
    let mut owner = Owner::default();
    let gate = owner.gate();
    let panicked = std::panic::AssertUnwindSafe(owner.run(
        Instant::now() + Duration::from_secs(1),
        || -> std::future::Ready<()> { panic!("planned native factory failure") },
    ))
    .catch_unwind()
    .await;
    assert!(panicked.is_err());
    assert!(gate.claim().is_none());
    let outcome = owner
        .run(Instant::now() + Duration::from_secs(1), || async {
            gate.claim().unwrap().fail(ExportFailure::Encoding);
        })
        .await;
    assert_eq!(outcome, ExportOutcome::Failed(ExportFailure::Encoding));
}
