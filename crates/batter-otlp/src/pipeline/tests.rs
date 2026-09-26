//! Real recorder, SDK aggregation and OTLP transport against a loopback sink.
//! Recording uses thread-scoped recorders; global installation is exercised
//! only in isolated Unix child processes by the `metrics_export` test target.

#[path = "../../tests/support/collector.rs"]
mod collector;
#[path = "tests/ordering.rs"]
mod ordering;
#[path = "tests/transport.rs"]
mod transport;

use super::{
    HISTOGRAM_BOUNDS, Schedule, Session,
    guard::{CatalogRecorder, DESCRIPTION_MAX_BYTES, GuardState, KEY_CAPACITY},
    prepare,
    transport::PAYLOAD_MAX_BYTES,
};
use crate::diagnostics::{ExportOutcome, FinalCoverage, GuardRejections};
use batter_core::telemetry::metrics::{
    self as catalog,
    facade::{
        Counter, Gauge, Histogram, Key, KeyName, Label, Level, Metadata, Recorder, SharedString,
        Unit,
    },
};
use collector::{Behavior, Collector, Value, counter, find, resource, series};
use metrics_exporter_otel::OpenTelemetryRecorder;
use prost::Message;
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// No periodic attempt runs within a test; only explicit and final exports.
const QUIET: Schedule = Schedule {
    interval: Duration::from_secs(3600),
    attempt: Duration::from_secs(2),
    final_allowance: Duration::from_secs(2),
};
static METADATA: Metadata<'static> = Metadata::new("test", Level::INFO, None);

fn pipeline(
    endpoint: &str,
    schedule: Schedule,
) -> (CatalogRecorder<OpenTelemetryRecorder>, Session) {
    prepare(endpoint, super::SERVICE_NAME, schedule)
        .expect("valid loopback pipeline")
        .into_parts()
}

/// One current-thread runtime owning a loopback collector.
struct Harness {
    runtime: tokio::runtime::Runtime,
    collector: Option<Collector>,
}

impl Harness {
    fn new(behavior: Behavior) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let collector = runtime
            .block_on(Collector::start(behavior))
            .expect("collector starts");
        Self {
            runtime,
            collector: Some(collector),
        }
    }

    fn collector(&self) -> &Collector {
        self.collector.as_ref().expect("collector is open")
    }

    fn pipeline(&self, schedule: Schedule) -> (CatalogRecorder<OpenTelemetryRecorder>, Session) {
        pipeline(&self.collector().endpoint(), schedule)
    }

    /// Run `body` with `recorder` scoped to this thread, so every task polled
    /// by the current-thread runtime records locally.
    fn run<T>(&self, recorder: &dyn Recorder, body: impl Future<Output = T>) -> T {
        catalog::facade::with_local_recorder(recorder, || self.runtime.block_on(body))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(collector) = self.collector.take() {
            let _ = self.runtime.block_on(collector.close());
        }
    }
}

fn key(name: &'static str, labels: &[(&'static str, &str)]) -> Key {
    Key::from_parts(
        name,
        labels
            .iter()
            .map(|(key, value)| Label::new(*key, (*value).to_owned()))
            .collect::<Vec<_>>(),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Counter,
    Histogram,
}

/// Every complete key the foundation can create: both name tables full of
/// maximum-length names, plus both placeholders, across every vocabulary.
fn every_series() -> Vec<(Kind, Key)> {
    let mut names: Vec<String> = (0..catalog::NAME_CAPACITY)
        .map(|index| format!("{index:.>width$}", width = catalog::MAX_NAME_LEN))
        .collect();
    names.extend([catalog::INVALID_NAME, catalog::OVERFLOW_NAME].map(str::to_owned));
    let mut keys = Vec::new();
    let mut push =
        |kind, name, labels: &[(&'static str, &str)]| keys.push((kind, key(name, labels)));
    for name in &names {
        for outcome in catalog::OUTCOMES {
            let labels = [("operation", name.as_str()), ("outcome", outcome)];
            push(Kind::Counter, catalog::OPERATION_COMPLETIONS, &labels);
            push(Kind::Histogram, catalog::OPERATION_DURATION, &labels);
            push(Kind::Counter, catalog::RETRY_ATTEMPTS, &labels);
        }
        for result in catalog::RETRY_RESULTS {
            let labels = [("operation", name.as_str()), ("result", result)];
            push(Kind::Counter, catalog::RETRY_EXECUTIONS, &labels);
        }
        for kind in catalog::TASK_KINDS {
            for outcome in catalog::TASK_OUTCOMES {
                let labels = [
                    ("kind", *kind),
                    ("task", name.as_str()),
                    ("outcome", outcome),
                ];
                push(Kind::Counter, catalog::TASK_EXITS, &labels);
            }
        }
    }
    for (admission, decisions) in [
        ("bulkhead", catalog::BULKHEAD_DECISIONS),
        ("process", catalog::PROCESS_DECISIONS),
        ("root", catalog::ROOT_DECISIONS),
    ] {
        for decision in decisions {
            let labels = [("admission", admission), ("decision", decision)];
            push(Kind::Counter, catalog::ADMISSION_DECISIONS, &labels);
        }
    }
    for outcome in catalog::CLEANUP_OUTCOMES {
        push(
            Kind::Counter,
            catalog::CLEANUP_HOOKS,
            &[("outcome", outcome)],
        );
    }
    for result in catalog::SHUTDOWN_RESULTS {
        for cause in catalog::SHUTDOWN_CAUSES {
            push(
                Kind::Counter,
                catalog::SHUTDOWNS,
                &[("cause", cause), ("result", result)],
            );
        }
        push(
            Kind::Histogram,
            catalog::SHUTDOWN_DURATION,
            &[("result", result)],
        );
    }
    for domain in catalog::COALESCE_DOMAINS {
        for reason in catalog::COALESCE_REASONS {
            let labels = [("domain", *domain), ("reason", reason)];
            push(Kind::Counter, catalog::LABELS_COALESCED, &labels);
        }
    }
    keys
}

fn register(recorder: &dyn Recorder, kind: Kind, key: &Key) {
    match kind {
        Kind::Counter => recorder.register_counter(key, &METADATA).increment(1),
        Kind::Histogram => recorder.register_histogram(key, &METADATA).record(0.02),
    }
}

/// Publish the catalog descriptions exactly as the canonical installation does.
fn describe(recorder: &dyn Recorder) {
    for (name, kind, unit) in [
        (catalog::OPERATION_COMPLETIONS, Kind::Counter, Unit::Count),
        (catalog::OPERATION_DURATION, Kind::Histogram, Unit::Seconds),
        (catalog::SHUTDOWNS, Kind::Counter, Unit::Count),
        (catalog::SHUTDOWN_DURATION, Kind::Histogram, Unit::Seconds),
    ] {
        let description = SharedString::from(format!("{name} description"));
        match kind {
            Kind::Counter => recorder.describe_counter(name.into(), Some(unit), description),
            Kind::Histogram => recorder.describe_histogram(name.into(), Some(unit), description),
        }
    }
}

#[derive(Default)]
struct Counting {
    registrations: AtomicUsize,
    descriptions: AtomicUsize,
}

impl Recorder for Counting {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {
        self.descriptions.fetch_add(1, Ordering::SeqCst);
    }
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {
        self.descriptions.fetch_add(1, Ordering::SeqCst);
    }
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {
        self.descriptions.fetch_add(1, Ordering::SeqCst);
    }
    fn register_counter(&self, _: &Key, _: &Metadata<'_>) -> Counter {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        Counter::noop()
    }
    fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        Gauge::noop()
    }
    fn register_histogram(&self, _: &Key, _: &Metadata<'_>) -> Histogram {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        Histogram::noop()
    }
}

#[test]
fn guard_rejects_outside_the_catalog_before_delegation() {
    let state = Arc::new(GuardState::default());
    let guard = CatalogRecorder::new(Counting::default(), state.clone());
    let long = "x".repeat(catalog::MAX_NAME_LEN + 1);
    let _ = guard.register_counter(&key("library_requests_total", &[]), &METADATA);
    let _ = guard.register_histogram(&key("library_latency", &[("route", "/r")]), &METADATA);
    for labels in [
        vec![("operation", "example.read")],
        vec![("operation", "example.read"), ("outcome", "bogus")],
        vec![("outcome", "succeeded"), ("operation", "example.read")],
        vec![("operation", "https://host/path"), ("outcome", "succeeded")],
        vec![("operation", long.as_str()), ("outcome", "succeeded")],
        vec![("operation", ""), ("outcome", "succeeded")],
        vec![("operation", "a"), ("outcome", "succeeded"), ("extra", "x")],
    ] {
        let _ = guard.register_counter(&key(catalog::OPERATION_COMPLETIONS, &labels), &METADATA);
    }
    // A bulkhead decision under the root admission label is outside its domain.
    let root = [("admission", "root"), ("decision", "overloaded")];
    let _ = guard.register_counter(&key(catalog::ADMISSION_DECISIONS, &root), &METADATA);
    let labels = [("operation", "a"), ("outcome", "succeeded")];
    let _ = guard.register_histogram(&key(catalog::OPERATION_COMPLETIONS, &labels), &METADATA);
    let shutdown = [("cause", "requested"), ("result", "success")];
    let _ = guard.register_gauge(&key(catalog::SHUTDOWNS, &shutdown), &METADATA);
    guard.describe_counter(
        "library_requests_total".into(),
        Some(Unit::Count),
        "x".into(),
    );
    let name = || KeyName::from_const_str(catalog::OPERATION_COMPLETIONS);
    guard.describe_counter(name(), Some(Unit::Seconds), "unit".into());
    guard.describe_counter(name(), None, "missing unit".into());
    let oversized = "x".repeat(DESCRIPTION_MAX_BYTES + 1);
    guard.describe_counter(name(), Some(Unit::Count), oversized.into());
    guard.describe_histogram(name(), Some(Unit::Count), "kind".into());
    guard.describe_gauge(catalog::SHUTDOWNS.into(), None, "gauge".into());

    assert_eq!(
        guard_delegate(&guard).registrations.load(Ordering::SeqCst),
        0
    );
    assert_eq!(
        guard_delegate(&guard).descriptions.load(Ordering::SeqCst),
        0
    );
    assert_eq!(state.admitted(), 0);
    assert_eq!(
        state.rejections(),
        GuardRejections {
            unknown_names: 3,
            invalid_labels: 8,
            invalid_descriptions: 5,
            unsupported_kinds: 2,
            capacity: 0,
        }
    );
    describe(&guard);
    assert_eq!(
        guard_delegate(&guard).descriptions.load(Ordering::SeqCst),
        4
    );
}

fn guard_delegate(guard: &CatalogRecorder<Counting>) -> &Counting {
    guard.inner()
}

#[test]
fn every_foundation_series_fits_and_the_next_key_is_rejected() {
    let series = every_series();
    assert_eq!(series.len(), catalog::MAX_SERIES);
    assert_eq!(KEY_CAPACITY, catalog::MAX_SERIES);
    let state = Arc::new(GuardState::default());
    let guard = CatalogRecorder::new(Counting::default(), state.clone());
    for (kind, key) in &series {
        register(&guard, *kind, key);
        // Repeated observations of an admitted key never add capacity.
        register(&guard, *kind, key);
    }
    assert_eq!(state.admitted(), catalog::MAX_SERIES);
    assert_eq!(state.rejections(), GuardRejections::default());
    // A native producer reusing a catalog name with a new valid operation name.
    let storm = [("operation", "storm.extra"), ("outcome", "succeeded")];
    for _ in 0..1000 {
        let _ = guard.register_counter(&key(catalog::OPERATION_COMPLETIONS, &storm), &METADATA);
    }
    assert_eq!(state.admitted(), catalog::MAX_SERIES);
    assert_eq!(state.rejections().capacity, 1000);
    assert_eq!(
        guard_delegate(&guard).registrations.load(Ordering::SeqCst),
        catalog::MAX_SERIES * 2
    );
}

#[test]
fn full_catalog_aggregates_into_one_bounded_cumulative_payload() {
    let harness = Harness::new(Behavior::Accept);
    let (recorder, session) = harness.pipeline(QUIET);
    let collector = harness.collector();
    let series_keys = every_series();
    harness.run(&recorder, async {
        let mut session = session;
        describe(&recorder);
        for (kind, key) in &series_keys {
            register(&recorder, *kind, key);
        }
        let first = session.attempt(deadline()).await;
        assert_eq!(first, ExportOutcome::Acknowledged);
        for (kind, key) in &series_keys {
            register(&recorder, *kind, key);
        }
        let (_, report) = session.around(async {}, |_| FinalCoverage::Reported).await;
        assert_eq!(report.final_export, ExportOutcome::Acknowledged);
        assert_eq!(report.periodic.attempts, 0);
        assert_eq!(report.rejected, GuardRejections::default());

        let requests = collector.requests();
        assert_eq!(requests.len(), 2);
        let encoded = requests[1].encoded_len();
        eprintln!("full catalog payload bytes: {encoded}");
        assert!(
            encoded <= PAYLOAD_MAX_BYTES / 2,
            "worst-case catalog payload {encoded} must leave headroom"
        );
        let exported = series(&requests[1]);
        assert_eq!(exported.len(), catalog::MAX_SERIES);
        assert_eq!(
            resource(&requests[1]),
            [("service.name".to_owned(), super::SERVICE_NAME.to_owned())]
        );
        let name = format!("{:.>width$}", 0, width = catalog::MAX_NAME_LEN);
        let labels = [("operation", name.as_str()), ("outcome", "succeeded")];
        // Cumulative temporality: the second snapshot contains both observations.
        assert_eq!(
            counter(
                &series(&requests[0]),
                catalog::OPERATION_COMPLETIONS,
                &labels
            ),
            Some(1)
        );
        assert_eq!(
            counter(&exported, catalog::OPERATION_COMPLETIONS, &labels),
            Some(2)
        );
        let completions = find(&exported, catalog::OPERATION_COMPLETIONS, &labels).unwrap();
        assert_eq!(completions.unit, "1");
        assert_eq!(
            completions.description,
            format!("{} description", catalog::OPERATION_COMPLETIONS)
        );
        let duration = find(&exported, catalog::OPERATION_DURATION, &labels).unwrap();
        assert_eq!(duration.unit, "s");
        let Value::Histogram {
            count,
            bounds,
            buckets,
        } = &duration.value
        else {
            panic!("duration is a histogram")
        };
        assert_eq!(
            (*count, bounds.as_slice()),
            (2, HISTOGRAM_BOUNDS.as_slice())
        );
        assert_eq!(buckets.len(), HISTOGRAM_BOUNDS.len() + 1);
        assert_eq!(
            buckets[2], 2,
            "0.02 seconds falls in the (0.01, 0.025] bucket"
        );
        let shutdown = [("result", "success")];
        let Value::Histogram { bounds, .. } =
            &find(&exported, catalog::SHUTDOWN_DURATION, &shutdown)
                .unwrap()
                .value
        else {
            panic!("shutdown duration is a histogram")
        };
        assert_eq!(bounds.as_slice(), HISTOGRAM_BOUNDS.as_slice());
    });
}

fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + QUIET.attempt
}
