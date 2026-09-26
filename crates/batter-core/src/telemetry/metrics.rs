//! Optional bounded-cardinality metrics through the [`metrics`] facade.
//!
//! Enable the `metrics` feature to record foundation outcomes into whichever
//! recorder the application root installs. Batter never installs a recorder,
//! exporter or global subscriber. Without a recorder, the facade's no-op
//! recorder discards every observation.
//!
//! Batter records through `metrics` 0.24, re-exported as [`facade`]. A recorder
//! or exporter built on another major version of `metrics` is a different
//! crate instance and silently receives nothing. Install it through
//! [`facade::set_global_recorder`] or choose an exporter that depends on the
//! same major version.
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
//! Each boundary that has been polled increments its counter by exactly one
//! when it finishes, including `dropped` when its future is destroyed first and
//! `panicked` when it is destroyed while its thread unwinds. An operation,
//! retry, admission or supervisor-drive future dropped before its first poll
//! does no work and records nothing. Registered cleanup hooks are different:
//! a `CleanupStack` destroyed without closing, including one moved into a
//! never-polled `close` future, records each hook as `dropped`, matching its
//! existing abandonment warning. A whole retry execution is one
//! [`RETRY_EXECUTIONS`] result. An attempt counts in [`RETRY_ATTEMPTS`] only
//! once the boundary invokes its factory, so an interruption that wins the
//! race first is not an attempt; neither counts as an operation. Foundation-owned waits (bulkhead waiting, retry backoff) are
//! recorded only by their owning decision. Adapter boundaries are ordinary
//! operations, for example `http.response_construction` for each admitted
//! Axum request, and their names share the operation table. A polled
//! supervisor drive records one shutdown: its report's result before it
//! publishes `Stopped`, or `dropped`/`panicked` (with cause `none` before a
//! cause was selected) when the driver is destroyed first. A startup that fails
//! before its running driver records its cleanup hooks, not a shutdown. Task
//! exits are recorded after a failure closes admission, and root admission
//! after the admission decision is complete.
//! Histograms receive one sample per finished boundary. Aggregation, buckets,
//! temporality and export are the recorder's policy.
//!
//! # Label domains
//!
//! Every label value is either a closed foundation vocabulary ([`OUTCOMES`],
//! [`RETRY_RESULTS`], [`ADMISSIONS`] with [`BULKHEAD_DECISIONS`],
//! [`PROCESS_DECISIONS`] and [`ROOT_DECISIONS`], [`TASK_KINDS`],
//! [`TASK_OUTCOMES`], [`CLEANUP_OUTCOMES`], [`SHUTDOWN_CAUSES`],
//! [`SHUTDOWN_RESULTS`], [`COALESCE_REASONS`]) or an operation/task name
//! admitted through a fixed-capacity table. Names use the component
//! registration vocabulary: 1 to [`MAX_NAME_LEN`] ASCII alphanumeric, `.`, `_`
//! or `-` bytes. Other names record [`INVALID_NAME`]; valid names beyond
//! [`NAME_CAPACITY`] distinct values per domain record [`OVERFLOW_NAME`].
//! Coalescing is reported through [`LABELS_COALESCED`], never by logging.
//!
//! The vocabulary structurally rejects URLs, paths, queries, e-mail addresses
//! and error text, but it cannot recognize an identifier embedded in an
//! otherwise valid name such as `order.12345`. Pass developer-controlled
//! constants. Each distinct name occupies one slot; slots are first-come and
//! never evicted, so names admitted early (including adapter names or leaked
//! runtime names) can force later names into [`OVERFLOW_NAME`] for the life of
//! the process, while the series count stays bounded. Cleanup names are not
//! emitted, and shutdown causes omit their component names.
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
//! // The application root installs a recorder once, for example through
//! // `batter_core::telemetry::metrics::facade::set_global_recorder` or an
//! // exporter on the same `metrics` major version, then may publish the
//! // catalog descriptions.
//! batter_core::telemetry::metrics::describe();
//! let context = OperationOwner::new(Duration::from_secs(1))?.into_context();
//! context
//!     .run("example.read", |_| async { Ok::<_, std::io::Error>(()) })
//!     .await?;
//! # Ok(())
//! # }
//! ```

mod names;
mod vocabulary;

pub use ::metrics as facade;
pub use vocabulary::{
    ADMISSIONS, BULKHEAD_DECISIONS, CLEANUP_OUTCOMES, COALESCE_DOMAINS, COALESCE_REASONS, OUTCOMES,
    PROCESS_DECISIONS, RETRY_RESULTS, ROOT_DECISIONS, SHUTDOWN_CAUSES, SHUTDOWN_RESULTS,
    TASK_KINDS, TASK_OUTCOMES,
};

use super::{Boundary, Outcome};
use crate::{
    admission::AdmissionError,
    cleanup::CleanupOutcome,
    lifecycle::{ProcessAdmissionError, Readiness, ShutdownCause, TaskOutcome},
    retry::RetryExecutionError,
};
use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};
use names::{OPERATION_NAMES, TASK_NAMES};
use std::time::Duration;
use tokio::time::Instant;

/// Completed operation boundaries, including dropped ones.
pub const OPERATION_COMPLETIONS: &str = "batter_operation_completions_total";
/// Elapsed time of each operation boundary in seconds.
pub const OPERATION_DURATION: &str = "batter_operation_duration_seconds";
/// Finished retry attempts whose factory started; not counted as operations.
pub const RETRY_ATTEMPTS: &str = "batter_retry_attempts_total";
/// Terminal results of whole retry executions.
pub const RETRY_EXECUTIONS: &str = "batter_retry_executions_total";
/// Bulkhead, process and root lifecycle admission decisions.
pub const ADMISSION_DECISIONS: &str = "batter_admission_decisions_total";
/// Observed direct task exits, including successful finite completion.
pub const TASK_EXITS: &str = "batter_task_exits_total";
/// Cleanup hook outcomes, including skipped and abandoned hooks.
pub const CLEANUP_HOOKS: &str = "batter_cleanup_hooks_total";
/// Completed supervisor drives.
pub const SHUTDOWNS: &str = "batter_shutdowns_total";
/// Time from drain start to the final shutdown report in seconds.
pub const SHUTDOWN_DURATION: &str = "batter_shutdown_duration_seconds";
/// Names replaced by [`INVALID_NAME`] or [`OVERFLOW_NAME`].
pub const LABELS_COALESCED: &str = "batter_metric_labels_coalesced_total";

/// Distinct admitted names per domain (operations and tasks).
pub const NAME_CAPACITY: usize = 64;
/// Longest admitted operation or task name in bytes, shared with registration.
pub const MAX_NAME_LEN: usize = crate::validation::NAME_MAX_LEN;
/// Label value recorded for a name outside the vocabulary. Its `<` and `>`
/// bytes are outside that vocabulary, so it cannot collide with a real name.
pub const INVALID_NAME: &str = "<invalid>";
/// Label value recorded once a name table is full; also collision-free.
pub const OVERFLOW_NAME: &str = "<overflow>";

const NAME_VALUES: usize = NAME_CAPACITY + 2;

/// Upper bound on distinct series across the whole catalog.
pub const MAX_SERIES: usize = NAME_VALUES * OUTCOMES.len() * 3
    + NAME_VALUES * RETRY_RESULTS.len()
    + BULKHEAD_DECISIONS.len()
    + PROCESS_DECISIONS.len()
    + ROOT_DECISIONS.len()
    + TASK_KINDS.len() * NAME_VALUES * TASK_OUTCOMES.len()
    + CLEANUP_OUTCOMES.len()
    + SHUTDOWN_CAUSES.len() * SHUTDOWN_RESULTS.len()
    + SHUTDOWN_RESULTS.len()
    + COALESCE_DOMAINS.len() * COALESCE_REASONS.len();

/// Which admission gate made a decision.
#[derive(Clone, Copy)]
pub(crate) enum AdmissionKind {
    Bulkhead,
    Process,
    Root,
}

/// Which name table coalesced a label.
#[derive(Clone, Copy)]
pub(crate) enum NameDomain {
    Operation,
    Task,
}

/// Why a name was replaced by a placeholder.
#[derive(Clone, Copy)]
pub(crate) enum Coalesce {
    Invalid,
    Capacity,
}

/// How a supervisor drive ended.
#[derive(Clone, Copy)]
pub(crate) enum ShutdownEnd {
    Success,
    Failure,
    Dropped,
    Panicked,
}

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
        "Finished Batter retry attempts by outcome"
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

pub(crate) fn operation(
    boundary: Boundary,
    factory_invoked: bool,
    name: &'static str,
    outcome: Outcome,
    elapsed: Duration,
) {
    let metric = match boundary {
        Boundary::Operation => OPERATION_COMPLETIONS,
        Boundary::RetryAttempt if factory_invoked => RETRY_ATTEMPTS,
        Boundary::RetryAttempt | Boundary::Internal => return,
    };
    let operation = OPERATION_NAMES.label(name);
    let outcome = vocabulary::outcome(outcome, std::thread::panicking());
    counter!(metric, "operation" => operation, "outcome" => outcome).increment(1);
    if matches!(boundary, Boundary::Operation) {
        histogram!(OPERATION_DURATION, "operation" => operation, "outcome" => outcome)
            .record(elapsed.as_secs_f64());
    }
}

/// Records one retry execution result, or `dropped`/`panicked` when
/// destroyed before a result.
pub(crate) struct RetryTerminal {
    operation: &'static str,
    result: Option<&'static str>,
}

impl RetryTerminal {
    pub(crate) fn new(operation: &'static str) -> Self {
        Self {
            operation,
            result: None,
        }
    }

    pub(crate) fn finish<T, E>(&mut self, result: &Result<T, RetryExecutionError<E>>) {
        self.result = Some(vocabulary::retry((
            Some(result.as_ref().map(|_| ())),
            false,
        )));
    }
}

impl Drop for RetryTerminal {
    fn drop(&mut self) {
        let result = self
            .result
            .unwrap_or_else(|| vocabulary::retry::<()>((None, std::thread::panicking())));
        let operation = OPERATION_NAMES.label(self.operation);
        counter!(RETRY_EXECUTIONS, "operation" => operation, "result" => result).increment(1);
    }
}

/// Records one bulkhead decision, or `dropped` when destroyed first.
pub(crate) struct BulkheadTerminal {
    decision: &'static str,
}

impl BulkheadTerminal {
    pub(crate) fn new() -> Self {
        Self {
            decision: vocabulary::bulkhead(None),
        }
    }

    pub(crate) fn finish<T>(&mut self, result: &Result<T, AdmissionError>) {
        self.decision = vocabulary::bulkhead(Some(result.as_ref().map(|_| ())));
    }
}

impl Drop for BulkheadTerminal {
    fn drop(&mut self) {
        admission(AdmissionKind::Bulkhead, self.decision);
    }
}

pub(crate) fn process<T>(result: &Result<T, ProcessAdmissionError>) {
    let decision = vocabulary::process(result.as_ref().map(|_| ()));
    admission(AdmissionKind::Process, decision);
}

pub(crate) fn root(observed: Readiness) {
    admission(AdmissionKind::Root, vocabulary::root(observed));
}

fn admission(kind: AdmissionKind, decision: &'static str) {
    let admission = vocabulary::admission_kind(kind);
    counter!(ADMISSION_DECISIONS, "admission" => admission, "decision" => decision).increment(1);
}

pub(crate) fn task(finite: bool, name: &'static str, outcome: TaskOutcome) {
    let kind = vocabulary::task_kind(finite);
    let outcome = vocabulary::task_outcome(outcome);
    let task = TASK_NAMES.label(name);
    counter!(TASK_EXITS, "kind" => kind, "task" => task, "outcome" => outcome).increment(1);
}

/// How a registered cleanup hook ended.
pub(crate) enum CleanupHook {
    Observed(CleanupOutcome),
    Skipped,
    Dropped,
}

pub(crate) fn cleanup(hook: CleanupHook, hooks: usize) {
    counter!(CLEANUP_HOOKS, "outcome" => vocabulary::cleanup(hook)).increment(hooks as u64);
}

/// Armed from the start of a supervisor drive; records exactly one shutdown,
/// including `dropped`/`panicked` when the driver is destroyed first.
pub(crate) struct ShutdownTerminal {
    cause: Option<ShutdownCause>,
    drain_started: Option<Instant>,
    armed: bool,
}

impl ShutdownTerminal {
    pub(crate) fn new() -> Self {
        Self {
            cause: None,
            drain_started: None,
            armed: true,
        }
    }

    pub(crate) fn draining(&mut self, cause: ShutdownCause) {
        self.cause = Some(cause);
        self.drain_started = Some(Instant::now());
    }

    /// Record the completed report's result now; call before `Stopped` is
    /// published so an application root cannot flush before this sample.
    pub(crate) fn finish(mut self, success: bool) {
        self.record(if success {
            ShutdownEnd::Success
        } else {
            ShutdownEnd::Failure
        });
    }

    fn record(&mut self, end: ShutdownEnd) {
        self.armed = false;
        let cause = vocabulary::shutdown_cause(self.cause);
        let result = vocabulary::shutdown_result(end);
        counter!(SHUTDOWNS, "cause" => cause, "result" => result).increment(1);
        // Duration is defined from drain start; an earlier abandonment has none.
        if let Some(started) = self.drain_started {
            histogram!(SHUTDOWN_DURATION, "result" => result)
                .record(started.elapsed().as_secs_f64());
        }
    }
}

impl Drop for ShutdownTerminal {
    fn drop(&mut self) {
        if self.armed {
            self.record(if std::thread::panicking() {
                ShutdownEnd::Panicked
            } else {
                ShutdownEnd::Dropped
            });
        }
    }
}

#[cfg(test)]
mod tests;
