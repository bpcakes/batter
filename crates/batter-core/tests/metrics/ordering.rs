//! Recorder faults must not strand accepted work; cleanup warnings precede metrics.

use super::capture::Capture;
use batter_core::{
    cleanup::{CleanupBudget, CleanupStack, SkipReason},
    lifecycle::{Fatal, ProcessCapacity, Supervisor},
    telemetry::metrics::{ADMISSION_DECISIONS, CLEANUP_HOOKS},
};
use metrics::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
    task::Poll,
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

#[derive(Debug, PartialEq)]
enum Event {
    Warning,
    Metric,
    Destroyed(&'static str),
}

type Events = Arc<Mutex<Vec<Event>>>;

struct OrderedRecorder {
    capture: Capture,
    events: Events,
    panic_admission: bool,
}

impl Recorder for OrderedRecorder {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        if self.panic_admission && key.name() == ADMISSION_DECISIONS {
            panic!("admission recorder failed");
        }
        if key.name() == CLEANUP_HOOKS {
            self.events.lock().unwrap().push(Event::Metric);
        }
        self.capture.register_counter(key, metadata)
    }

    fn register_gauge(&self, key: &Key, metadata: &Metadata<'_>) -> Gauge {
        self.capture.register_gauge(key, metadata)
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        self.capture.register_histogram(key, metadata)
    }
}

struct Warnings(Events);

impl<S: tracing::Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.lock().unwrap().push(Event::Warning);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn recorder_panic_cannot_strand_an_accepted_process_task() {
    let capture = Capture::unbounded();
    let recorder = OrderedRecorder {
        capture: capture.clone(),
        events: Events::default(),
        panic_admission: true,
    };
    let _recorder = metrics::set_default_local_recorder(&recorder);
    let mut supervisor = Supervisor::with_process_capacity(
        super::shutdown_budget(),
        ProcessCapacity::new(1).unwrap(),
    );
    supervisor
        .on_cleanup("dependency.close", || async { Ok(()) })
        .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let started = tokio::time::Instant::now();
    let (completed, completion) = tokio::sync::oneshot::channel();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            process.try_spawn("job.run", |_| async move {
                completed.send(()).unwrap();
                Ok::<_, Fatal<Infallible>>(())
            })
        }))
        .is_err()
    );
    let report = running.shutdown_checked().await.unwrap();
    completion.await.unwrap();
    assert_eq!(report.report().completed_process_tasks, 1);
    assert!(!report.report().forced_cancellation);
    assert_eq!(report.report().cleanup.records.len(), 1);
    assert_eq!(started.elapsed(), std::time::Duration::ZERO);
    assert_eq!(
        capture.count(CLEANUP_HOOKS, &[("outcome", "succeeded")]),
        1.0
    );
}

#[tokio::test]
async fn abandoned_and_dropped_cleanup_warn_before_recording() {
    let events = Events::default();
    let recorder = OrderedRecorder {
        capture: Capture::unbounded(),
        events: events.clone(),
        panic_admission: false,
    };
    let _recorder = metrics::set_default_local_recorder(&recorder);
    let subscriber = tracing_subscriber::registry().with(Warnings(events.clone()));
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let mut unclosed = CleanupStack::new();
    unclosed.push("unused", || async { Ok(()) }).unwrap();
    drop(unclosed);
    assert_eq!(*events.lock().unwrap(), [Event::Warning, Event::Metric]);
    events.lock().unwrap().clear();

    let mut stack = CleanupStack::new();
    stack
        .push("pending", pending::<Result<(), batter_core::BoxError>>)
        .unwrap();
    let second = std::time::Duration::from_secs(1);
    let mut closing = Box::pin(stack.close(CleanupBudget::new(second, second, second).unwrap()));
    poll_fn(|cx| {
        assert!(closing.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(closing);
    assert_eq!(*events.lock().unwrap(), [Event::Warning, Event::Metric]);
}

#[test]
fn skip_warns_for_every_hook_before_recording_and_destroying_captures() {
    struct CaptureDrop {
        name: &'static str,
        events: Events,
        panics: bool,
    }
    impl Drop for CaptureDrop {
        fn drop(&mut self) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Destroyed(self.name));
            assert!(!self.panics, "capture destruction failed");
        }
    }

    for panics in [false, true] {
        let events = Events::default();
        let recorder = OrderedRecorder {
            capture: Capture::unbounded(),
            events: events.clone(),
            panic_admission: false,
        };
        let _recorder = metrics::set_default_local_recorder(&recorder);
        let subscriber = tracing_subscriber::registry().with(Warnings(events.clone()));
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let mut stack = CleanupStack::new();
        for name in ["first", "second"] {
            let captured = CaptureDrop {
                name,
                events: events.clone(),
                panics: panics && name == "second",
            };
            stack
                .push(name, move || async move {
                    drop(captured);
                    Ok(())
                })
                .unwrap();
        }
        let result = catch_unwind(AssertUnwindSafe(|| stack.skip(SkipReason::UnsafeTaskExit)));
        assert_eq!(result.is_err(), panics);
        if let Ok(report) = result {
            assert_eq!(
                report
                    .skipped
                    .iter()
                    .map(|hook| hook.name)
                    .collect::<Vec<_>>(),
                ["second", "first"]
            );
        }
        assert_eq!(
            *events.lock().unwrap(),
            [
                Event::Warning,
                Event::Warning,
                Event::Metric,
                Event::Destroyed("second"),
                Event::Destroyed("first"),
            ]
        );
        assert_eq!(
            recorder
                .capture
                .count(CLEANUP_HOOKS, &[("outcome", "skipped")]),
            2.0
        );
        assert_eq!(
            recorder
                .capture
                .count(CLEANUP_HOOKS, &[("outcome", "dropped")]),
            0.0
        );
    }
}
