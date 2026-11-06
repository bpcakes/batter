//! Returned evidence must reach core before the application future is dropped.

use std::{
    future::Future,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
};

use batter_core::{
    cleanup::SkipReason,
    lifecycle::{ShutdownFailure, TaskOutcome},
    periodic::{PeriodicCompletion, PeriodicFailure, PeriodicRun},
};
use batter_flags_core::{PrePublication, SwitchState, Toggle};
use tokio::time::sleep;

use super::{Failed, policy, support::*};

#[derive(Clone, Copy)]
enum Returned {
    Success,
    Recoverable,
    Fatal,
}

/// A valid !Unpin future that returns normally but whose destructor panics.
/// Its output is fixed; like Ready, its caller must stop polling after Ready.
struct ReturnThenPanic {
    returned: Returned,
    _pin: PhantomPinned,
}

impl Future for ReturnThenPanic {
    type Output = PeriodicRun<Failed>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(match self.returned {
            Returned::Success => Ok(()),
            Returned::Recoverable => Err(PeriodicFailure::Recoverable(Failed(7))),
            Returned::Fatal => Err(PeriodicFailure::Fatal(Failed(9))),
        })
    }
}

impl Drop for ReturnThenPanic {
    fn drop(&mut self) {
        panic!("application future destructor panicked");
    }
}

async fn retained_despite_destruction(returned: Returned) {
    let mut fixture = Fixture::new(PrePublication::Unready);
    let mut supervisor = supervisor();
    let reader = fixture
        .flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.pruning",
            policy(),
            &fixture.halt,
            move |_, _| ReturnThenPanic {
                returned,
                _pin: PhantomPinned,
            },
        )
        .unwrap();
    supervisor
        .on_cleanup("storage", || async {
            panic!("a panicked task cannot authorize dependency cleanup")
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(SECOND / 2).await;
    assert_eq!(reader.skipped(), 1);
    fixture.publish(1, SwitchState::Released, Toggle::Off);
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected retained task-panic report")
    };
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert_eq!(reader.skipped(), 1);
    let summary = &report.periodic[0].summary;
    assert_eq!(summary.invocations, 2);
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert_eq!(summary.deadline_exceeded, 0);
    assert_eq!(summary.stop_interrupted, 0);
    match returned {
        Returned::Success => {
            assert_eq!(summary.succeeded, 2);
            assert_eq!(reader.snapshot().succeeded, 2);
            assert_eq!(summary.recoverable_failures, 0);
            assert!(summary.first_failure.is_none());
            assert!(summary.terminal_failure.is_none());
        }
        Returned::Recoverable => {
            assert_eq!(summary.succeeded, 1);
            assert_eq!(summary.recoverable_failures, 1);
            assert_eq!(reader.snapshot().recoverable_failures, 1);
            let sample = summary.first_failure.as_ref().unwrap();
            assert_eq!(sample.invocation, 2);
            assert_eq!(sample.error.downcast_ref::<Failed>().unwrap().0, 7);
            assert!(summary.terminal_failure.is_none());
        }
        Returned::Fatal => {
            assert_eq!(summary.succeeded, 1);
            assert_eq!(summary.recoverable_failures, 0);
            assert!(summary.first_failure.is_none());
            let sample = summary.terminal_failure.as_ref().unwrap();
            assert_eq!(sample.invocation, 2);
            assert_eq!(sample.error.downcast_ref::<Failed>().unwrap().0, 9);
            assert!(reader.snapshot().terminal_failure.is_some());
        }
    }
    assert!(!format!("{report:?}").contains("private maintenance failure"));
}

#[tokio::test(start_paused = true)]
async fn recoverable_error_survives_application_future_drop_panic() {
    retained_despite_destruction(Returned::Recoverable).await;
}

#[tokio::test(start_paused = true)]
async fn fatal_error_survives_application_future_drop_panic() {
    retained_despite_destruction(Returned::Fatal).await;
}

#[tokio::test(start_paused = true)]
async fn success_count_survives_application_future_drop_panic() {
    retained_despite_destruction(Returned::Success).await;
}
