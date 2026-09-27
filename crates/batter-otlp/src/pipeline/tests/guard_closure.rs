use super::{
    CatalogRecorder, Counting, GuardState, METADATA, QUIET, describe, every_series, key, pipeline,
    register,
};
use crate::diagnostics::FinalCoverage;
use batter_core::telemetry::metrics::{
    self as catalog,
    facade::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit},
};
use std::{
    sync::{Arc, Barrier, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::yield_now();
    }
}

struct PausedRecorder {
    counting: Counting,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl Recorder for PausedRecorder {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.counting.describe_counter(key, unit, description);
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.counting.describe_gauge(key, unit, description);
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.counting.describe_histogram(key, unit, description);
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        self.entered.wait();
        self.release.wait();
        self.counting.register_counter(key, metadata)
    }

    fn register_gauge(&self, key: &Key, metadata: &Metadata<'_>) -> Gauge {
        self.counting.register_gauge(key, metadata)
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        self.counting.register_histogram(key, metadata)
    }
}

#[test]
fn close_waits_for_delegation_and_rejects_later_recorder_calls() {
    let state = Arc::new(GuardState::default());
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let guard = Arc::new(CatalogRecorder::new(
        PausedRecorder {
            counting: Counting::default(),
            entered: entered.clone(),
            release: release.clone(),
        },
        state.clone(),
    ));
    let first = key(
        catalog::OPERATION_COMPLETIONS,
        &[("operation", "first"), ("outcome", "succeeded")],
    );
    thread::scope(|scope| {
        let guard = guard.clone();
        let registration = scope.spawn(move || guard.register_counter(&first, &METADATA));
        entered.wait();

        let (closed_tx, closed_rx) = mpsc::channel();
        let closing_state = state.clone();
        let closure = scope.spawn(move || {
            closing_state.close();
            closed_tx.send(()).unwrap();
        });
        let close_entered = wait_until(Duration::from_secs(2), || state.gate_state().0);
        let closed_while_delegating = closed_rx.recv_timeout(Duration::from_millis(100)).is_ok();
        release.wait();
        let _ = registration.join().unwrap();
        if !closed_while_delegating {
            closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        closure.join().unwrap();
        assert!(
            close_entered,
            "closure did not enter before delegation ended"
        );
        assert!(
            !closed_while_delegating,
            "closure returned during delegation"
        );
    });

    assert_eq!(
        guard.inner().counting.registrations.load(Ordering::SeqCst),
        1
    );
    assert_eq!(state.admitted(), 1);
    let later = key(
        catalog::OPERATION_COMPLETIONS,
        &[("operation", "later"), ("outcome", "succeeded")],
    );
    let _ = guard.register_counter(&later, &METADATA);
    let _ = guard.register_histogram(
        &key(
            catalog::OPERATION_DURATION,
            &[("operation", "later"), ("outcome", "succeeded")],
        ),
        &METADATA,
    );
    describe(&*guard);
    assert_eq!(
        guard.inner().counting.registrations.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        guard.inner().counting.descriptions.load(Ordering::SeqCst),
        0
    );
    assert_eq!(state.admitted(), 1);
}

#[test]
fn final_report_includes_rejection_completed_during_closure() {
    if super::otel_env::rerun_if_ambient() {
        return;
    }
    let (recorder, session) = pipeline("http://127.0.0.1:1/v1/metrics", QUIET);
    let state = session.resources.state.clone();
    for (kind, key) in every_series() {
        register(&recorder, kind, &key);
    }
    assert_eq!(state.admitted(), catalog::MAX_SERIES);
    let keys = state.hold_keys();
    let excess = key(
        catalog::OPERATION_COMPLETIONS,
        &[("operation", "excess"), ("outcome", "succeeded")],
    );
    thread::scope(|scope| {
        let registration = scope.spawn(|| recorder.register_counter(&excess, &METADATA));
        let admission_started = wait_until(Duration::from_secs(2), || state.gate_state().1 == 1);
        if !admission_started {
            drop(keys);
            let _ = registration.join();
            panic!("registration did not enter before finalization");
        }
        let finalization = scope.spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(session.finish(FinalCoverage::Reported))
        });
        let close_entered = wait_until(Duration::from_secs(5), || state.gate_state().0);
        drop(keys);
        let _ = registration.join().unwrap();
        let report = finalization.join().unwrap();
        assert!(close_entered, "finalization did not close the guard");
        assert_eq!(report.rejected.capacity, 1);
    });
}
