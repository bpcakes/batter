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
//! race first is not an attempt; neither counts as an operation.
//! Foundation-owned waits (bulkhead waiting, retry backoff) are recorded only
//! by their owning decision. Adapter boundaries are ordinary
//! operations, for example `http.response_construction` for each admitted
//! Axum request, and their names share the operation table. A polled
//! supervisor drive records one shutdown: its report's result before it
//! publishes `Stopped`, or `dropped`/`panicked` (with cause `none` before a
//! cause was selected) when the driver is destroyed first. Its duration runs
//! from the lifecycle stop instant that every drain budget uses to the final
//! report; abandoned drivers have no report and record no duration. A startup
//! that fails before its running driver records its cleanup hooks, not a
//! shutdown.
//!
//! Recording follows the boundary's own ordering. Task exits are recorded
//! after a failure closes admission. An accepted process admission is recorded
//! when the supervisor takes ownership of the queued task, so it precedes that
//! task's exit and the shutdown; rejections are recorded after the admission
//! lock is released. Root admission is recorded after its decision is
//! complete. Root-admitted work is not supervised, so its decision and
//! operations are not ordered with shutdown and may be recorded after
//! `Stopped`. Histograms receive one sample per finished boundary. Aggregation, buckets,
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
//! [`MAX_SERIES`], independent of traffic. Batter retains `2 * NAME_CAPACITY`
//! write-once name slots and at most [`MAX_SERIES`] pre-built series keys,
//! and no other observation storage. A series' key is allocated once, on its
//! first observation; later observations of it do not allocate.
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

mod keys;
mod names;
mod terminal;
mod vocabulary;

pub use ::metrics as facade;
pub use vocabulary::{
    ADMISSIONS, BULKHEAD_DECISIONS, CLEANUP_OUTCOMES, COALESCE_DOMAINS, COALESCE_REASONS, OUTCOMES,
    PROCESS_DECISIONS, RETRY_RESULTS, ROOT_DECISIONS, SHUTDOWN_CAUSES, SHUTDOWN_RESULTS,
    TASK_KINDS, TASK_OUTCOMES,
};

use super::{Boundary, Outcome, record::CleanupHook};
use crate::lifecycle::{ProcessAdmissionError, Readiness, ShutdownCause, TaskOutcome};
use keys::{KeyCache, increment, index_of, sample};
use metrics::{Unit, describe_counter, describe_histogram};
use names::{NameLabel, OPERATION_NAMES, TASK_NAMES};
use std::time::Duration;

pub(crate) use terminal::{AdmittedDecision, BulkheadTerminal, RetryTerminal, ShutdownTerminal};

/// Completed operation boundaries, including dropped ones.
pub const OPERATION_COMPLETIONS: &str = "batter_operation_completions_total";
/// Elapsed time of each operation boundary in seconds.
pub const OPERATION_DURATION: &str = "batter_operation_duration_seconds";
/// Finished retry attempts whose factory was invoked; not counted as operations.
pub const RETRY_ATTEMPTS: &str = "batter_retry_attempts_total";
/// Terminal results of whole retry executions.
pub const RETRY_EXECUTIONS: &str = "batter_retry_executions_total";
/// Bulkhead, process and root lifecycle admission decisions.
pub const ADMISSION_DECISIONS: &str = "batter_admission_decisions_total";
/// Observed direct task exits, including successful finite completion.
pub const TASK_EXITS: &str = "batter_task_exits_total";
/// Cleanup hook outcomes, including skipped and abandoned hooks.
pub const CLEANUP_HOOKS: &str = "batter_cleanup_hooks_total";
/// Supervisor drives, completed or abandoned.
pub const SHUTDOWNS: &str = "batter_shutdowns_total";
/// Time from the lifecycle stop instant to the final shutdown report in
/// seconds; abandoned drivers have no report and record no sample.
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

/// Admitted slots plus the two placeholders.
const NAME_VALUES: usize = NAME_CAPACITY + 2;

const OPERATION_SERIES: usize = NAME_VALUES * OUTCOMES.len();
const RETRY_SERIES: usize = NAME_VALUES * RETRY_RESULTS.len();
const TASK_SERIES: usize = TASK_KINDS.len() * NAME_VALUES * TASK_OUTCOMES.len();
const SHUTDOWN_SERIES: usize = SHUTDOWN_CAUSES.len() * SHUTDOWN_RESULTS.len();
const COALESCE_SERIES: usize = COALESCE_DOMAINS.len() * COALESCE_REASONS.len();

static COMPLETION_KEYS: KeyCache<OPERATION_SERIES> = KeyCache::new(OPERATION_COMPLETIONS);
static DURATION_KEYS: KeyCache<OPERATION_SERIES> = KeyCache::new(OPERATION_DURATION);
static ATTEMPT_KEYS: KeyCache<OPERATION_SERIES> = KeyCache::new(RETRY_ATTEMPTS);
static RETRY_KEYS: KeyCache<RETRY_SERIES> = KeyCache::new(RETRY_EXECUTIONS);
static BULKHEAD_KEYS: KeyCache<{ BULKHEAD_DECISIONS.len() }> = KeyCache::new(ADMISSION_DECISIONS);
static PROCESS_KEYS: KeyCache<{ PROCESS_DECISIONS.len() }> = KeyCache::new(ADMISSION_DECISIONS);
static ROOT_KEYS: KeyCache<{ ROOT_DECISIONS.len() }> = KeyCache::new(ADMISSION_DECISIONS);
static TASK_KEYS: KeyCache<TASK_SERIES> = KeyCache::new(TASK_EXITS);
static CLEANUP_KEYS: KeyCache<{ CLEANUP_OUTCOMES.len() }> = KeyCache::new(CLEANUP_HOOKS);
static SHUTDOWN_KEYS: KeyCache<SHUTDOWN_SERIES> = KeyCache::new(SHUTDOWNS);
static SHUTDOWN_DURATION_KEYS: KeyCache<{ SHUTDOWN_RESULTS.len() }> =
    KeyCache::new(SHUTDOWN_DURATION);
static COALESCE_KEYS: KeyCache<COALESCE_SERIES> = KeyCache::new(LABELS_COALESCED);

/// Upper bound on distinct series across the whole catalog. It is also the
/// total size of Batter's pre-built key caches, its only other retained
/// observation storage besides the name tables.
pub const MAX_SERIES: usize = OPERATION_SERIES * 3
    + RETRY_SERIES
    + BULKHEAD_DECISIONS.len()
    + PROCESS_DECISIONS.len()
    + ROOT_DECISIONS.len()
    + TASK_SERIES
    + CLEANUP_OUTCOMES.len()
    + SHUTDOWN_SERIES
    + SHUTDOWN_RESULTS.len()
    + COALESCE_SERIES;

/// How a guarded boundary ended.
pub(crate) enum End<T> {
    Finished(T),
    Dropped,
    Panicked,
}

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
    describe_counter!(SHUTDOWNS, Unit::Count, "Batter supervisor drive results");
    describe_histogram!(
        SHUTDOWN_DURATION,
        Unit::Seconds,
        "Batter stop instant to shutdown-report time"
    );
    describe_counter!(
        LABELS_COALESCED,
        Unit::Count,
        "Metric names replaced by a bounded placeholder"
    );
}

/// Series index of a name label combined with one closed-domain value.
fn named(name: NameLabel, values: &[&str], value: &str) -> usize {
    name.index * values.len() + index_of(values, value)
}

pub(crate) fn operation(
    boundary: Boundary,
    factory_invoked: bool,
    name: &'static str,
    outcome: Outcome,
    elapsed: Duration,
) {
    let cache = match boundary {
        Boundary::Operation => &COMPLETION_KEYS,
        Boundary::RetryAttempt if factory_invoked => &ATTEMPT_KEYS,
        Boundary::RetryAttempt | Boundary::Internal => return,
    };
    let operation = OPERATION_NAMES.label(name);
    let outcome = vocabulary::outcome(outcome, std::thread::panicking());
    let index = named(operation, &OUTCOMES, outcome);
    let labels = [("operation", operation.text), ("outcome", outcome)];
    increment(cache.key(index, labels), 1);
    if matches!(boundary, Boundary::Operation) {
        sample(DURATION_KEYS.key(index, labels), elapsed.as_secs_f64());
    }
}

fn retry_execution(name: &'static str, result: &'static str) {
    let operation = OPERATION_NAMES.label(name);
    let index = named(operation, RETRY_RESULTS, result);
    let labels = [("operation", operation.text), ("result", result)];
    increment(RETRY_KEYS.key(index, labels), 1);
}

fn admission(kind: AdmissionKind, decision: &'static str) {
    let admission = vocabulary::admission_kind(kind);
    let labels = [("admission", admission), ("decision", decision)];
    let key = match kind {
        AdmissionKind::Bulkhead => {
            BULKHEAD_KEYS.key(index_of(BULKHEAD_DECISIONS, decision), labels)
        }
        AdmissionKind::Process => PROCESS_KEYS.key(index_of(PROCESS_DECISIONS, decision), labels),
        AdmissionKind::Root => ROOT_KEYS.key(index_of(ROOT_DECISIONS, decision), labels),
    };
    increment(key, 1);
}

/// A rejected process admission, recorded after the admission lock and any
/// rejected captures are released. Accepted decisions travel with the task.
pub(crate) fn process_rejected(error: &ProcessAdmissionError) {
    admission(AdmissionKind::Process, vocabulary::process(Err(error)));
}

pub(crate) fn root(observed: Readiness) {
    admission(AdmissionKind::Root, vocabulary::root(observed));
}

pub(crate) fn task(finite: bool, name: &'static str, outcome: TaskOutcome) {
    let kind = vocabulary::task_kind(finite);
    let outcome = vocabulary::task_outcome(outcome);
    let task = TASK_NAMES.label(name);
    let index = index_of(TASK_KINDS, kind) * NAME_VALUES * TASK_OUTCOMES.len()
        + named(task, TASK_OUTCOMES, outcome);
    let labels = [("kind", kind), ("task", task.text), ("outcome", outcome)];
    increment(TASK_KEYS.key(index, labels), 1);
}

/// Count `hooks` cleanup hooks with one outcome in a single increment.
pub(crate) fn cleanup(hook: CleanupHook, hooks: usize) {
    if hooks == 0 {
        return;
    }
    let outcome = vocabulary::cleanup(hook);
    let key = CLEANUP_KEYS.key(index_of(CLEANUP_OUTCOMES, outcome), [("outcome", outcome)]);
    increment(key, hooks as u64);
}

fn shutdown(cause: Option<ShutdownCause>, result: &'static str, elapsed: Option<Duration>) {
    let cause = vocabulary::shutdown_cause(cause);
    let index = index_of(SHUTDOWN_CAUSES, cause) * SHUTDOWN_RESULTS.len()
        + index_of(SHUTDOWN_RESULTS, result);
    increment(
        SHUTDOWN_KEYS.key(index, [("cause", cause), ("result", result)]),
        1,
    );
    if let Some(elapsed) = elapsed {
        let key =
            SHUTDOWN_DURATION_KEYS.key(index_of(SHUTDOWN_RESULTS, result), [("result", result)]);
        sample(key, elapsed.as_secs_f64());
    }
}

fn coalesced(domain: NameDomain, reason: Coalesce) {
    let domain = vocabulary::name_domain(domain);
    let reason = vocabulary::coalesce_reason(reason);
    let index = index_of(COALESCE_DOMAINS, domain) * COALESCE_REASONS.len()
        + index_of(COALESCE_REASONS, reason);
    increment(
        COALESCE_KEYS.key(index, [("domain", domain), ("reason", reason)]),
        1,
    );
}

#[cfg(test)]
mod tests;
