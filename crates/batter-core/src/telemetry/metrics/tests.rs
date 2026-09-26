//! Name-table, closed-vocabulary and production timing checks.

// Shares the integration tests' recorder rather than keeping a second fake.
#[path = "../../../tests/metrics/capture.rs"]
mod capture;

use super::{names::NameTable, *};
use crate::telemetry::Observation;
use capture::Capture;

#[test]
fn registration_vocabulary_is_shared_and_rejects_urls_and_error_text() {
    static TABLE: NameTable = NameTable::new(NameDomain::Operation);
    for name in ["example.read", "http-server", "Refresh", "x9_y.z"] {
        assert_eq!(TABLE.label(name).text, name);
    }
    let longest: &'static str = String::leak("a".repeat(MAX_NAME_LEN));
    assert_eq!(TABLE.label(longest).text, longest);
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
        assert_eq!(TABLE.label(name).text, INVALID_NAME, "{name}");
    }
}

#[test]
fn full_table_coalesces_without_eviction() {
    static TABLE: NameTable = NameTable::new(NameDomain::Task);
    let names: Vec<&'static str> = (0..NAME_CAPACITY + 8)
        .map(|index| &*String::leak(format!("name.n{index}")))
        .collect();
    for name in &names[..NAME_CAPACITY] {
        assert_eq!(TABLE.label(name).text, *name);
    }
    for name in &names[NAME_CAPACITY..] {
        assert_eq!(TABLE.label(name).text, OVERFLOW_NAME);
    }
    // A distinct copy of an admitted name reuses its slot.
    let copy: &'static str = String::leak(names[0].to_owned());
    assert_eq!(TABLE.label(copy).text, names[0]);
    assert_eq!(TABLE.label("Not Valid").text, INVALID_NAME);
    // Placeholders use the two series indices after the admitted slots.
    assert_eq!(TABLE.label("Not Valid").index, NAME_CAPACITY);
    assert_eq!(TABLE.label("name.late").index, NAME_CAPACITY + 1);
}

#[test]
fn closed_domains_are_distinct_and_outside_the_name_vocabulary_placeholders() {
    for domain in [
        &OUTCOMES[..],
        RETRY_RESULTS,
        ADMISSIONS,
        BULKHEAD_DECISIONS,
        PROCESS_DECISIONS,
        ROOT_DECISIONS,
        TASK_KINDS,
        TASK_OUTCOMES,
        CLEANUP_OUTCOMES,
        SHUTDOWN_CAUSES,
        SHUTDOWN_RESULTS,
        COALESCE_DOMAINS,
        COALESCE_REASONS,
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
        assert_eq!(vocabulary::outcome(outcome, false), outcome.as_str());
        assert!(OUTCOMES.contains(&vocabulary::outcome(outcome, true)));
    }
    assert_eq!(vocabulary::outcome(Outcome::Dropped, true), "panicked");
    assert_eq!(vocabulary::outcome(Outcome::Failed, true), "failed");
}

#[test]
fn production_timing_clamps_clock_regression_to_a_finite_zero() {
    let capture = Capture::unbounded();
    let _recorder = metrics::set_default_local_recorder(&capture);
    let mut regressed = Observation::new("example.read", Boundary::Operation);
    // A start instant after "now" models a regressed or inconsistent clock.
    regressed.started = tokio::time::Instant::now() + Duration::from_secs(60);
    regressed.finish(Outcome::Succeeded);
    drop(regressed);
    let mut ordinary = Observation::new("example.read", Boundary::Operation);
    ordinary.finish(Outcome::Failed);
    drop(ordinary);
    let recorded: Vec<f64> = capture
        .samples(OPERATION_DURATION)
        .into_iter()
        .map(|sample| sample.value)
        .collect();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0], 0.0);
    assert!(recorded[1].is_finite() && recorded[1] >= 0.0);
}

#[tokio::test]
async fn internal_boundaries_record_no_operation_metrics() {
    let capture = Capture::unbounded();
    let _recorder = metrics::set_default_local_recorder(&capture);
    let context = crate::operation::OperationOwner::new(Duration::from_secs(1))
        .unwrap()
        .into_context();
    let waited = context
        .run_internal("batter.admission", |_| async { Ok::<(), ()>(()) })
        .await;
    assert!(waited.is_ok());
    let attempted = context
        .run_retry_attempt(
            "provider.read",
            |_| async { Ok::<(), ()>(()) },
            |_| Outcome::Succeeded,
        )
        .await;
    assert!(attempted.is_ok());
    assert_eq!(capture.series().len(), 0);
}

#[test]
fn series_keys_are_built_once_and_reused() {
    let labels = [("admission", "root"), ("decision", "admitted")];
    let first: *const metrics::Key = keys_probe(labels);
    let again: *const metrics::Key = keys_probe(labels);
    assert!(std::ptr::eq(first, again));
}

fn keys_probe(labels: [(&'static str, &'static str); 2]) -> &'static metrics::Key {
    static PROBE: keys::KeyCache<1> = keys::KeyCache::new(ADMISSION_DECISIONS);
    PROBE.key(0, labels)
}
