//! Optional bounded-cardinality metrics through the [`metrics`] facade.
//!
//! Enable the `metrics` feature to record foundation outcomes into whichever
//! [`metrics::Recorder`] the application root installs. Batter never installs a
//! recorder, exporter or global subscriber. Without a recorder, the facade's
//! no-op recorder discards every observation.
//!
//! # Catalog
//!
//! | Name | Kind | Unit | Labels |
//! |------|------|------|--------|
//! | [`OPERATION_COMPLETIONS`] | counter | count | `operation`, `outcome` |
//! | [`OPERATION_DURATION`] | histogram | seconds | `operation`, `outcome` |
//! | [`RETRY_ATTEMPTS`] | counter | count | `operation`, `outcome` |
//! | [`ADMISSION_DECISIONS`] | counter | count | `admission`, `decision` |
//! | [`TASK_EXITS`] | counter | count | `kind`, `task`, `outcome` |
//! | [`CLEANUP_HOOKS`] | counter | count | `outcome` |
//! | [`SHUTDOWNS`] | counter | count | `cause`, `result` |
//! | [`SHUTDOWN_DURATION`] | histogram | seconds | `result` |
//! | [`LABELS_COALESCED`] | counter | count | `domain`, `reason` |
//!
//! Each finished boundary increments its counter by exactly one. Histograms
//! receive one sample per finished boundary. Aggregation, buckets, temporality
//! and export are the recorder's policy.
//!
//! # Label domains
//!
//! Every label value is either a closed foundation vocabulary (see
//! [`OUTCOMES`], [`ADMISSION_LABELS`], [`TASK_KINDS`], [`TASK_OUTCOMES`],
//! [`CLEANUP_OUTCOMES`], [`SHUTDOWN_CAUSES`], [`SHUTDOWN_RESULTS`],
//! [`COALESCE_REASONS`]) or a developer-supplied operation/task name admitted
//! through a fixed-capacity table. A name is admitted only when it is 1 to
//! [`MAX_NAME_LEN`] bytes, starts with an ASCII lowercase letter and otherwise
//! contains only ASCII lowercase letters, digits, `.` or `_`. Rejected names
//! record [`INVALID_NAME`]; valid names beyond [`NAME_CAPACITY`] distinct values
//! per domain record [`OVERFLOW_NAME`]. URLs, UUIDs, e-mail addresses, numeric
//! identifiers and error text therefore cannot become labels. Coalescing is
//! reported through [`LABELS_COALESCED`], never by logging. A leaked runtime
//! string that fits the vocabulary can occupy at most one slot; the table never
//! grows or evicts. Task and cleanup names are not otherwise emitted, and
//! shutdown causes omit their component names.
//!
//! Therefore the number of distinct series Batter can create is at most
//! [`MAX_SERIES`], independent of traffic. Batter retains
//! `2 * NAME_CAPACITY` static name slots and no other observation storage.
//!
//! # Failure boundary
//!
//! Recording is synchronous, happens after the boundary result is known, runs
//! outside admission locks, and never returns an error, retries, logs or
//! queues. Batter owns no buffer or flush: a slow or unavailable collector,
//! a saturated exporter queue, or a recorder that drops samples cannot change
//! a returned result or an admission decision, and rejected-request storms do
//! not create Batter-owned write queues or database prerequisites. The
//! recorder decides how its own buffering drops or coalesces; choose one with
//! bounded aggregation and flush it at the application root.
//!
//! Timing uses [`std::time::Duration`] from a monotonic clock, which cannot be
//! negative; clock regression saturates to zero. Samples are finite seconds.
//!
//! Recorder code runs synchronously inside the finishing boundary, including in
//! destructors. As with `tracing` subscribers, Batter does not isolate recorder
//! code that blocks, panics, or allocates without bound, nor runtime death or
//! non-yielding destructors. These metrics are best-effort diagnostics, not an
//! authoritative audit record.
//!
//! ```
//! use batter_core::operation::OperationOwner;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // The application root installs a recorder once, for example an exporter's
//! // `install()` method, and may then publish the catalog descriptions.
//! batter_core::telemetry::metrics::describe();
//! let context = OperationOwner::new(Duration::from_secs(1))?.into_context();
//! context
//!     .run("example.read", |_| async { Ok::<_, std::io::Error>(()) })
//!     .await?;
//! # Ok(())
//! # }
//! ```

use super::Outcome;
use crate::{
    admission::AdmissionError,
    cleanup::CleanupOutcome,
    lifecycle::{ProcessAdmissionError, ShutdownCause, TaskOutcome},
    operation::Interruption,
};
use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};
use std::{sync::OnceLock, time::Duration};

/// Completed operation boundaries, including dropped ones.
pub const OPERATION_COMPLETIONS: &str = "batter_operation_completions_total";
/// Elapsed time of each operation boundary in seconds.
pub const OPERATION_DURATION: &str = "batter_operation_duration_seconds";
/// Completed retry attempts; retries do not also count as operations.
pub const RETRY_ATTEMPTS: &str = "batter_retry_attempts_total";
/// Bulkhead and process admission decisions.
pub const ADMISSION_DECISIONS: &str = "batter_admission_decisions_total";
/// Observed direct task exits, including successful finite completion.
pub const TASK_EXITS: &str = "batter_task_exits_total";
/// Cleanup hook outcomes, including skipped hooks.
pub const CLEANUP_HOOKS: &str = "batter_cleanup_hooks_total";
/// Completed supervisor shutdowns.
pub const SHUTDOWNS: &str = "batter_shutdowns_total";
/// Time from drain start to the final shutdown report in seconds.
pub const SHUTDOWN_DURATION: &str = "batter_shutdown_duration_seconds";
/// Names replaced by [`INVALID_NAME`] or [`OVERFLOW_NAME`].
pub const LABELS_COALESCED: &str = "batter_metric_labels_coalesced_total";

/// Distinct admitted names per domain (operations and tasks).
pub const NAME_CAPACITY: usize = 64;
/// Longest admitted operation or task name in bytes.
pub const MAX_NAME_LEN: usize = 64;
/// Label value recorded for a name outside the metric vocabulary.
pub const INVALID_NAME: &str = "_invalid";
/// Label value recorded once a name table is full.
pub const OVERFLOW_NAME: &str = "_overflow";

/// Values of the `outcome` label on operation and attempt metrics.
pub const OUTCOMES: [&str; 5] = [
    "succeeded",
    "failed",
    "cancelled",
    "deadline_exceeded",
    "dropped",
];
/// `(admission, decision)` pairs on [`ADMISSION_DECISIONS`].
pub const ADMISSION_LABELS: [(&str, &str); 11] = [
    ("bulkhead", "admitted"),
    ("bulkhead", "overloaded"),
    ("bulkhead", "closed"),
    ("bulkhead", "cancelled"),
    ("bulkhead", "deadline_exceeded"),
    ("process", "admitted"),
    ("process", "not_running"),
    ("process", "not_ready"),
    ("process", "closed"),
    ("process", "full"),
    ("process", "invalid_name"),
];
/// Values of the `kind` label on [`TASK_EXITS`].
pub const TASK_KINDS: [&str; 2] = ["component", "process"];
/// Values of the `outcome` label on [`TASK_EXITS`].
pub const TASK_OUTCOMES: [&str; 6] = [
    "completed",
    "stopped",
    "unexpected_exit",
    "failed",
    "panicked",
    "aborted",
];
/// Values of the `outcome` label on [`CLEANUP_HOOKS`].
pub const CLEANUP_OUTCOMES: [&str; 7] = [
    "succeeded",
    "failed",
    "panicked",
    "cancelled",
    "timed_out",
    "unjoined",
    "skipped",
];
/// Values of the `cause` label on [`SHUTDOWNS`].
pub const SHUTDOWN_CAUSES: [&str; 4] = [
    "requested",
    "component_exit",
    "finite_task_exit",
    "empty_supervisor",
];
/// Values of the `result` label on shutdown metrics.
pub const SHUTDOWN_RESULTS: [&str; 2] = ["success", "failure"];
/// `(domain, reason)` values on [`LABELS_COALESCED`].
pub const COALESCE_REASONS: [(&str, &str); 4] = [
    ("operation", "invalid"),
    ("operation", "capacity"),
    ("task", "invalid"),
    ("task", "capacity"),
];

const NAME_VALUES: usize = NAME_CAPACITY + 2;

/// Upper bound on distinct series across the whole catalog.
pub const MAX_SERIES: usize = NAME_VALUES * OUTCOMES.len() * 3
    + ADMISSION_LABELS.len()
    + TASK_KINDS.len() * NAME_VALUES * TASK_OUTCOMES.len()
    + CLEANUP_OUTCOMES.len()
    + SHUTDOWN_CAUSES.len() * SHUTDOWN_RESULTS.len()
    + SHUTDOWN_RESULTS.len()
    + COALESCE_REASONS.len();

/// Publish names, units and descriptions to the current recorder.
///
/// Call after installing the recorder. Descriptions are optional metadata;
/// recording does not depend on them.
pub fn describe() {
    describe_counter!(
        OPERATION_COMPLETIONS,
        Unit::Count,
        "Completed Batter operation boundaries by outcome"
    );
    describe_histogram!(
        OPERATION_DURATION,
        Unit::Seconds,
        "Elapsed Batter operation boundary time"
    );
    describe_counter!(
        RETRY_ATTEMPTS,
        Unit::Count,
        "Completed Batter retry attempts by outcome"
    );
    describe_counter!(
        ADMISSION_DECISIONS,
        Unit::Count,
        "Batter admission decisions"
    );
    describe_counter!(TASK_EXITS, Unit::Count, "Observed Batter task exits");
    describe_counter!(CLEANUP_HOOKS, Unit::Count, "Batter cleanup hook outcomes");
    describe_counter!(SHUTDOWNS, Unit::Count, "Completed Batter shutdowns");
    describe_histogram!(
        SHUTDOWN_DURATION,
        Unit::Seconds,
        "Batter drain-start to shutdown-report time"
    );
    describe_counter!(
        LABELS_COALESCED,
        Unit::Count,
        "Metric names replaced by a bounded placeholder"
    );
}

struct NameTable {
    domain: &'static str,
    slots: [OnceLock<&'static str>; NAME_CAPACITY],
}

impl NameTable {
    const fn new(domain: &'static str) -> Self {
        Self {
            domain,
            slots: [const { OnceLock::new() }; NAME_CAPACITY],
        }
    }

    /// Return a bounded label for `name`, admitting it when a slot is free.
    ///
    /// Slots are write-once and lock-free to read. Concurrent admission of the
    /// same name may occupy two slots; the distinct label values stay bounded.
    fn label(&self, name: &'static str) -> &'static str {
        if !valid_name(name) {
            self.coalesced("invalid");
            return INVALID_NAME;
        }
        for slot in &self.slots {
            match slot.get() {
                Some(existing) if *existing == name => return existing,
                Some(_) => {}
                None => match slot.set(name) {
                    Ok(()) => return name,
                    Err(_) if slot.get() == Some(&name) => return name,
                    Err(_) => {}
                },
            }
        }
        self.coalesced("capacity");
        OVERFLOW_NAME
    }

    fn coalesced(&self, reason: &'static str) {
        counter!(LABELS_COALESCED, "domain" => self.domain, "reason" => reason).increment(1);
    }
}

static OPERATION_NAMES: NameTable = NameTable::new("operation");
static TASK_NAMES: NameTable = NameTable::new("task");

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    matches!(bytes.first(), Some(b'a'..=b'z'))
        && bytes.len() <= MAX_NAME_LEN
        && bytes
            .iter()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_'))
}

fn seconds(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64()
}

pub(crate) fn operation(attempt: bool, name: &'static str, outcome: Outcome, elapsed: Duration) {
    let operation = OPERATION_NAMES.label(name);
    let outcome = outcome.as_str();
    if attempt {
        counter!(RETRY_ATTEMPTS, "operation" => operation, "outcome" => outcome).increment(1);
    } else {
        counter!(OPERATION_COMPLETIONS, "operation" => operation, "outcome" => outcome)
            .increment(1);
        histogram!(OPERATION_DURATION, "operation" => operation, "outcome" => outcome)
            .record(seconds(elapsed));
    }
}

pub(crate) fn bulkhead<T>(result: &Result<T, AdmissionError>) {
    let decision = match result {
        Ok(_) => "admitted",
        Err(AdmissionError::Overloaded) => "overloaded",
        Err(AdmissionError::Closed) => "closed",
        Err(AdmissionError::Interrupted(Interruption::Cancelled)) => "cancelled",
        Err(AdmissionError::Interrupted(Interruption::DeadlineExceeded)) => "deadline_exceeded",
    };
    admission("bulkhead", decision);
}

pub(crate) fn process<T>(result: &Result<T, ProcessAdmissionError>) {
    let decision = match result {
        Ok(_) => "admitted",
        Err(ProcessAdmissionError::NotRunning) => "not_running",
        Err(ProcessAdmissionError::NotReady) => "not_ready",
        Err(ProcessAdmissionError::Closed) => "closed",
        Err(ProcessAdmissionError::Full) => "full",
        Err(ProcessAdmissionError::InvalidName(_)) => "invalid_name",
    };
    admission("process", decision);
}

fn admission(admission: &'static str, decision: &'static str) {
    counter!(ADMISSION_DECISIONS, "admission" => admission, "decision" => decision).increment(1);
}

pub(crate) fn task(finite: bool, name: &'static str, outcome: TaskOutcome) {
    let kind = if finite { "process" } else { "component" };
    let outcome = match outcome {
        TaskOutcome::Completed => "completed",
        TaskOutcome::Stopped => "stopped",
        TaskOutcome::UnexpectedExit => "unexpected_exit",
        TaskOutcome::Failed => "failed",
        TaskOutcome::Panicked => "panicked",
        TaskOutcome::Aborted => "aborted",
    };
    let task = TASK_NAMES.label(name);
    counter!(TASK_EXITS, "kind" => kind, "task" => task, "outcome" => outcome).increment(1);
}

pub(crate) fn cleanup(outcome: Option<CleanupOutcome>) {
    let outcome = match outcome {
        Some(CleanupOutcome::Succeeded) => "succeeded",
        Some(CleanupOutcome::Failed) => "failed",
        Some(CleanupOutcome::Panicked) => "panicked",
        Some(CleanupOutcome::Cancelled) => "cancelled",
        Some(CleanupOutcome::TimedOut) => "timed_out",
        Some(CleanupOutcome::Unjoined) => "unjoined",
        None => "skipped",
    };
    counter!(CLEANUP_HOOKS, "outcome" => outcome).increment(1);
}

pub(crate) fn shutdown(cause: ShutdownCause, success: bool, elapsed: Duration) {
    let cause = match cause {
        ShutdownCause::Requested => "requested",
        ShutdownCause::ComponentExit(_) => "component_exit",
        ShutdownCause::FiniteTaskExit(_) => "finite_task_exit",
        ShutdownCause::EmptySupervisor => "empty_supervisor",
    };
    let result = if success { "success" } else { "failure" };
    counter!(SHUTDOWNS, "cause" => cause, "result" => result).increment(1);
    histogram!(SHUTDOWN_DURATION, "result" => result).record(seconds(elapsed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_rejects_identifiers_urls_and_error_text() {
        for name in ["example.read", "http.response_construction", "a", "x9_y.z"] {
            assert!(valid_name(name), "{name}");
        }
        for name in [
            "",
            "https://example.test/items/7",
            "/items/7",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "user@example.test",
            "12345",
            "connection refused: peer reset",
            "Example.Read",
            "_invalid",
            "_overflow",
            "name-with-hyphen",
        ] {
            assert!(!valid_name(name), "{name}");
        }
        assert!(valid_name(&"a".repeat(MAX_NAME_LEN)));
        assert!(!valid_name(&"a".repeat(MAX_NAME_LEN + 1)));
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
        assert_eq!(TABLE.label(names[0]), names[0]);
        assert_eq!(TABLE.label("Not Valid"), INVALID_NAME);
    }

    #[test]
    fn timing_samples_are_finite_and_nonnegative() {
        assert_eq!(seconds(Duration::ZERO), 0.0);
        let max = seconds(Duration::MAX);
        assert!(max.is_finite() && max > 0.0);
        let regressed = tokio::time::Instant::now()
            .saturating_duration_since(tokio::time::Instant::now() + Duration::from_secs(1));
        assert_eq!(seconds(regressed), 0.0);
    }

    #[test]
    fn series_bound_matches_catalog_domains() {
        assert_eq!(MAX_SERIES, 66 * 5 * 3 + 11 + 2 * 66 * 6 + 7 + 8 + 2 + 4);
    }
}
