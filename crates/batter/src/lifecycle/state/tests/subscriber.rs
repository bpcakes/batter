use super::super::Shared;
use crate::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tracing::{
    Event, Metadata, Subscriber,
    span::{Attributes, Id, Record},
};
use tracing_core::span::Current;

struct SubscriberProbe {
    state: Arc<Shared>,
    registry: tracing_subscriber::Registry,
    filtered: bool,
    armed: AtomicBool,
    calls: [AtomicUsize; 3],
}

impl SubscriberProbe {
    fn check(&self, callback: usize) {
        if self.armed.load(Ordering::Relaxed) {
            assert!(
                self.state.admission.try_lock().is_ok(),
                "subscriber callback {} ran under admission lock",
                ["new_span", "current_span", "clone_span"][callback]
            );
            self.calls[callback].fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Subscriber for SubscriberProbe {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        !self.filtered || metadata.target() != "batter"
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        self.check(0);
        self.registry.new_span(attributes)
    }

    fn current_span(&self) -> Current {
        self.check(1);
        self.registry.current_span()
    }

    fn clone_span(&self, id: &Id) -> Id {
        self.check(2);
        self.registry.clone_span(id)
    }

    fn record(&self, id: &Id, values: &Record<'_>) {
        self.registry.record(id, values);
    }

    fn record_follows_from(&self, id: &Id, follows: &Id) {
        self.registry.record_follows_from(id, follows);
    }

    fn event(&self, event: &Event<'_>) {
        self.registry.event(event);
    }

    fn enter(&self, id: &Id) {
        self.registry.enter(id);
    }

    fn exit(&self, id: &Id) {
        self.registry.exit(id);
    }

    fn try_close(&self, id: Id) -> bool {
        self.registry.try_close(id)
    }
}

#[test]
fn subscriber_callbacks_run_before_finite_admission_lock() {
    for filtered in [false, true] {
        let second = Duration::from_secs(1);
        let budget = ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap();
        let supervisor = Supervisor::with_process_capacity(budget, 1).unwrap();
        let process = supervisor.process_handle().unwrap();
        let handle = supervisor.handle();
        // Set readiness synchronously: the coordinator must not contend for
        // this lock, and queued application work must remain unpolled.
        handle.shared.start_driver();
        handle.mark_ready();
        let probe = Arc::new(SubscriberProbe {
            state: handle.shared.clone(),
            registry: tracing_subscriber::registry(),
            filtered,
            armed: AtomicBool::new(false),
            calls: std::array::from_fn(|_| AtomicUsize::new(0)),
        });
        let dispatch = tracing::Dispatch::new(probe.clone());
        tracing::dispatcher::with_default(&dispatch, || {
            let parent = tracing::info_span!("application.parent");
            let _entered = parent.enter();
            probe.armed.store(true, Ordering::Relaxed);
            let receipt = process
                .try_spawn(
                    "finite.work",
                    |_| -> std::future::Ready<Result<(), Infallible>> {
                        panic!("this test must not invoke the application factory")
                    },
                )
                .unwrap();
            probe.armed.store(false, Ordering::Relaxed);
            drop(receipt);
        });
        let calls = probe
            .calls
            .each_ref()
            .map(|calls| calls.load(Ordering::Relaxed));
        if filtered {
            assert_eq!(calls[0], 0, "disabled task span must not be created");
            assert!(calls[1] > 0, "fallback must query the current parent");
            assert!(calls[2] > 0, "fallback must retain the current parent");
        } else {
            assert!(calls[0] > 0, "enabled task span must be created");
        }
    }
}
