//! Name-table, closed-vocabulary and production timing checks.

use super::{names::NameTable, *};
use crate::telemetry::Observation;
use metrics::{
    Counter, Gauge, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder, SharedString,
};
use std::sync::{Arc, Mutex};

#[test]
fn registration_vocabulary_is_shared_and_rejects_urls_and_error_text() {
    static TABLE: NameTable = NameTable::new("vocabulary");
    for name in ["example.read", "http-server", "Refresh", "x9_y.z"] {
        assert_eq!(TABLE.label(name), name);
    }
    let longest: &'static str = String::leak("a".repeat(MAX_NAME_LEN));
    assert_eq!(TABLE.label(longest), longest);
    let too_long: &'static str = String::leak("a".repeat(MAX_NAME_LEN + 1));
    for name in [
        "",
        too_long,
        "https://example.test/items/7",
        "/items/7",
        "user@example.test",
        "connection refused: peer reset",
        INVALID_NAME,
        OVERFLOW_NAME,
    ] {
        assert_eq!(TABLE.label(name), INVALID_NAME, "{name}");
    }
}

#[test]
fn full_table_coalesces_without_eviction() {
    static TABLE: NameTable = NameTable::new("test");
    let names: Vec<&'static str> = (0..NAME_CAPACITY + 8)
        .map(|index| &*String::leak(format!("name.n{index}")))
        .collect();
    for name in &names[..NAME_CAPACITY] {
        assert_eq!(TABLE.label(name), *name);
    }
    for name in &names[NAME_CAPACITY..] {
        assert_eq!(TABLE.label(name), OVERFLOW_NAME);
    }
    // A distinct copy of an admitted name reuses its slot.
    let copy: &'static str = String::leak(names[0].to_owned());
    assert_eq!(TABLE.label(copy), names[0]);
    assert_eq!(TABLE.label("Not Valid"), INVALID_NAME);
}

#[test]
fn closed_domains_are_distinct_and_outside_the_name_vocabulary_placeholders() {
    for domain in [
        OUTCOMES,
        RETRY_RESULTS,
        BULKHEAD_DECISIONS,
        PROCESS_DECISIONS,
        ROOT_DECISIONS,
        TASK_KINDS,
        TASK_OUTCOMES,
        CLEANUP_OUTCOMES,
        SHUTDOWN_CAUSES,
        SHUTDOWN_RESULTS,
    ] {
        let mut sorted = domain.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), domain.len(), "{domain:?}");
    }
    for outcome in [
        Outcome::Succeeded,
        Outcome::Failed,
        Outcome::Cancelled,
        Outcome::DeadlineExceeded,
        Outcome::Dropped,
    ] {
        assert_eq!(vocabulary::outcome(outcome), outcome.as_str());
    }
}

/// Retains histogram values and the name of every registered metric.
#[derive(Clone, Default)]
struct Samples {
    values: Arc<Mutex<Vec<f64>>>,
    registered: Arc<Mutex<Vec<String>>>,
}

impl HistogramFn for Samples {
    fn record(&self, value: f64) {
        self.values.lock().unwrap().push(value);
    }
}

impl Recorder for Samples {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
        self.registered.lock().unwrap().push(key.name().to_owned());
        Counter::noop()
    }
    fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
        Gauge::noop()
    }
    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        self.registered.lock().unwrap().push(key.name().to_owned());
        Histogram::from_arc(Arc::new(self.clone()))
    }
}

#[test]
fn production_timing_clamps_clock_regression_to_a_finite_zero() {
    let samples = Samples::default();
    let _recorder = metrics::set_default_local_recorder(&samples);
    let mut regressed = Observation::new("example.read", Boundary::Operation);
    // A start instant after "now" models a regressed or inconsistent clock.
    regressed.started = tokio::time::Instant::now() + Duration::from_secs(60);
    regressed.finish(Outcome::Succeeded);
    drop(regressed);
    let mut ordinary = Observation::new("example.read", Boundary::Operation);
    ordinary.finish(Outcome::Failed);
    drop(ordinary);
    let recorded = samples.values.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0], 0.0);
    assert!(recorded[1].is_finite() && recorded[1] >= 0.0);
}

#[tokio::test]
async fn attempt_rejected_before_its_factory_is_not_a_finished_attempt() {
    let samples = Samples::default();
    let _recorder = metrics::set_default_local_recorder(&samples);
    let owner = crate::operation::OperationOwner::new(Duration::from_secs(1)).unwrap();
    owner.cancel();
    let mut started = false;
    let result = owner
        .context()
        .run_retry_attempt(
            "provider.read",
            |_| {
                started = true;
                async { Ok::<(), ()>(()) }
            },
            |_| Outcome::Cancelled,
        )
        .await;
    assert!(result.is_err());
    assert!(!started);
    assert_eq!(*samples.registered.lock().unwrap(), Vec::<String>::new());
}
