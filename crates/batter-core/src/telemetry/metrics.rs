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
//! | [`RETRY_EXECUTIONS`] | counter | count | `operation`, `result` |
//! | [`ADMISSION_DECISIONS`] | counter | count | `admission`, `decision` |
//! | [`TASK_EXITS`] | counter | count | `kind`, `task`, `outcome` |
//! | [`CLEANUP_HOOKS`] | counter | count | `outcome` |
//! | [`SHUTDOWNS`] | counter | count | `cause`, `result` |
//! | [`SHUTDOWN_DURATION`] | histogram | seconds | `result` |
//! | [`LABELS_COALESCED`] | counter | count | `domain`, `reason` |
//!
//! Each finished boundary increments its counter by exactly one, including a
//! boundary whose future is dropped (`dropped`). A whole retry execution is one
//! [`RETRY_EXECUTIONS`] result; its attempts are [`RETRY_ATTEMPTS`], and neither
//! counts as an operation. Foundation-owned waits (bulkhead waiting, retry
//! backoff) are recorded only by their owning decision, not as operations.
//! Histograms receive one sample per finished boundary. Aggregation, buckets,
//! temporality and export are the recorder's policy.
//!
//! # Label domains
//!
//! Every label value is either a closed foundation vocabulary (see
//! [`OUTCOMES`], [`RETRY_RESULTS`], [`ADMISSION_LABELS`], [`TASK_KINDS`], [`TASK_OUTCOMES`],
//! [`CLEANUP_OUTCOMES`], [`SHUTDOWN_CAUSES`], [`SHUTDOWN_RESULTS`],
//! [`COALESCE_REASONS`]) or a developer-supplied operation/task name admitted
//! through a fixed-capacity table. Names use the same vocabulary as component
//! registration: 1 to [`MAX_NAME_LEN`] ASCII alphanumeric, `.`, `_` or `-`
//! bytes. Other names record [`INVALID_NAME`]; valid names beyond
//! [`NAME_CAPACITY`] distinct values per domain record [`OVERFLOW_NAME`].
//! Coalescing is reported through [`LABELS_COALESCED`], never by logging.
//!
//! The vocabulary structurally rejects URLs, paths, queries, e-mail addresses
//! and error text, but it cannot recognize an identifier embedded in an
//! otherwise valid name such as `order.12345`. Pass developer-controlled
//! constants. A leaked runtime name occupies at most one slot; slots are
//! first-come and never evicted, so leaked names admitted early can force
//! later constants into [`OVERFLOW_NAME`] for the life of the process, while the
//! series count stays bounded. Cleanup names are not emitted, and shutdown
//! causes omit their component names.
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

use super::{Boundary, Outcome};
use crate::{
    admission::AdmissionError,
    cleanup::CleanupOutcome,
    lifecycle::{ProcessAdmissionError, ShutdownCause, TaskOutcome},
    operation::Interruption,
    retry::{RetryExecutionError, StopReason},
};
use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};
use std::{sync::OnceLock, time::Duration};

/// Completed operation boundaries, including dropped ones.
pub const OPERATION_COMPLETIONS: &str = "batter_operation_completions_total";
/// Elapsed time of each operation boundary in seconds.
pub const OPERATION_DURATION: &str = "batter_operation_duration_seconds";
/// Completed retry attempts; attempts do not also count as operations.
pub const RETRY_ATTEMPTS: &str = "batter_retry_attempts_total";
/// Terminal results of whole retry executions.
pub const RETRY_EXECUTIONS: &str = "batter_retry_executions_total";
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
pub const MAX_NAME_LEN: usize = 96;
/// Label value recorded for a name outside the vocabulary. Its `<` and `>`
/// bytes are outside that vocabulary, so it cannot collide with a real name.
pub const INVALID_NAME: &str = "<invalid>";
/// Label value recorded once a name table is full; also collision-free.
pub const OVERFLOW_NAME: &str = "<overflow>";

/// Values of the `outcome` label on operation and attempt metrics.
pub const OUTCOMES: [&str; 5] = [
    "succeeded",
    "failed",
    "cancelled",
    "deadline_exceeded",
    "dropped",
];
/// Values of the `result` label on [`RETRY_EXECUTIONS`].
pub const RETRY_RESULTS: [&str; 9] = [
    "succeeded",
    "not_retryable",
    "replay_forbidden",
    "attempts_exhausted",
    "insufficient_budget",
    "cancelled",
    "deadline_exceeded",
    "attempt_deadline_exceeded",
    "dropped",
];
/// `(admission, decision)` pairs on [`ADMISSION_DECISIONS`]. A bulkhead
/// `dropped` decision is a waiting `enter` future dropped before a decision.
pub const ADMISSION_LABELS: [(&str, &str); 12] = [
    ("bulkhead", "admitted"),
    ("bulkhead", "overloaded"),
    ("bulkhead", "closed"),
    ("bulkhead", "cancelled"),
    ("bulkhead", "deadline_exceeded"),
    ("bulkhead", "dropped"),
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
/// Values of the `outcome` label on [`CLEANUP_HOOKS`]. `dropped` counts hooks
/// abandoned by dropping an unclosed stack or an in-flight close driver.
pub const CLEANUP_OUTCOMES: [&str; 8] = [
    "succeeded",
    "failed",
    "panicked",
    "cancelled",
    "timed_out",
    "unjoined",
    "skipped",
    "dropped",
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
    + NAME_VALUES * RETRY_RESULTS.len()
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
        RETRY_EXECUTIONS,
        Unit::Count,
        "Terminal Batter retry execution results"
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
    /// Slots are write-once, filled in order and lock-free to read, so the
    /// first empty slot ends the lookup. Validation runs only before admitting.
    /// Concurrent admission of one name may occupy two slots; the distinct
    /// label values stay bounded.
    fn label(&self, name: &'static str) -> &'static str {
        let mut first_empty = self.slots.len();
        for (index, slot) in self.slots.iter().enumerate() {
            match slot.get() {
                Some(existing) if std::ptr::eq(*existing, name) || *existing == name => {
                    return existing;
                }
                Some(_) => {}
                None => {
                    first_empty = index;
                    break;
                }
            }
        }
        if crate::validation::name(name).is_err() {
            self.coalesced("invalid");
            return INVALID_NAME;
        }
        for slot in &self.slots[first_empty..] {
            match slot.set(name) {
                Ok(()) => return name,
                Err(_) if slot.get() == Some(&name) => return name,
                Err(_) => {}
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

fn seconds(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64()
}

pub(crate) fn operation(
    boundary: Boundary,
    name: &'static str,
    outcome: Outcome,
    elapsed: Duration,
) {
    let metric = match boundary {
        Boundary::Operation => OPERATION_COMPLETIONS,
        Boundary::RetryAttempt => RETRY_ATTEMPTS,
        Boundary::Internal => return,
    };
    let operation = OPERATION_NAMES.label(name);
    let outcome = outcome.as_str();
    counter!(metric, "operation" => operation, "outcome" => outcome).increment(1);
    if matches!(boundary, Boundary::Operation) {
        histogram!(OPERATION_DURATION, "operation" => operation, "outcome" => outcome)
            .record(seconds(elapsed));
    }
}

/// Records one terminal result when finished, or `dropped` when destroyed first.
pub(crate) struct Terminal {
    kind: TerminalKind,
    result: &'static str,
}

enum TerminalKind {
    Retry(&'static str),
    Bulkhead,
}

impl Terminal {
    pub(crate) fn retry(operation: &'static str) -> Self {
        Self {
            kind: TerminalKind::Retry(operation),
            result: "dropped",
        }
    }

    pub(crate) fn bulkhead() -> Self {
        Self {
            kind: TerminalKind::Bulkhead,
            result: "dropped",
        }
    }

    pub(crate) fn finish_retry<T, E>(&mut self, result: &Result<T, RetryExecutionError<E>>) {
        self.result = match result {
            Ok(_) => "succeeded",
            Err(RetryExecutionError::Stopped { reason, .. }) => match reason {
                StopReason::NotRetryable => "not_retryable",
                StopReason::ReplayForbidden => "replay_forbidden",
                StopReason::AttemptsExhausted => "attempts_exhausted",
                StopReason::InsufficientBudget => "insufficient_budget",
            },
            Err(RetryExecutionError::Interrupted { reason, .. }) => interruption(*reason),
            Err(RetryExecutionError::AttemptDeadlineExceeded { .. }) => "attempt_deadline_exceeded",
        };
    }

    pub(crate) fn finish_bulkhead<T>(&mut self, result: &Result<T, AdmissionError>) {
        self.result = match result {
            Ok(_) => "admitted",
            Err(AdmissionError::Overloaded) => "overloaded",
            Err(AdmissionError::Closed) => "closed",
            Err(AdmissionError::Interrupted(reason)) => interruption(*reason),
        };
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        match self.kind {
            TerminalKind::Retry(name) => {
                let operation = OPERATION_NAMES.label(name);
                counter!(RETRY_EXECUTIONS, "operation" => operation, "result" => self.result)
                    .increment(1);
            }
            TerminalKind::Bulkhead => admission("bulkhead", self.result),
        }
    }
}

fn interruption(reason: Interruption) -> &'static str {
    match reason {
        Interruption::Cancelled => "cancelled",
        Interruption::DeadlineExceeded => "deadline_exceeded",
    }
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

/// Count hooks abandoned without an observed or skipped outcome.
pub(crate) fn cleanup_dropped(hooks: usize) {
    counter!(CLEANUP_HOOKS, "outcome" => "dropped").increment(hooks as u64);
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
mod tests;
