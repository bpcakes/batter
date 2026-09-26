//! Optional bounded-cardinality metrics through the [`metrics`] facade.
//!
//! Enable the `metrics` feature to record foundation outcomes into the
//! recorder the application root passes to [`install`]. Batter never installs
//! a recorder, exporter or global subscriber itself. Without a recorder, the
//! facade's no-op recorder discards every observation.
//!
//! Batter records through `metrics` 0.24, re-exported as [`facade`].
//! [`install`] accepts only a recorder built on that version, so a recorder
//! from another major version fails to compile on the canonical path, and it
//! publishes the catalog descriptions only after the recorder exists. An
//! exporter that installs itself globally bypasses both checks and must share
//! this major version; that path is a lower-level escape hatch.
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
//! [`RETRY_EXECUTIONS`] result, recorded after discarded errors and owned
//! callbacks (including jitter samplers) are destroyed. A destructor panic
//! before return records `panicked`; destruction of the returned value or error
//! belongs to the caller and does not change that result.
//! An attempt counts in [`RETRY_ATTEMPTS`] only
//! once the boundary invokes its factory, so an interruption that wins the
//! race first is not an attempt; neither counts as an operation.
//! Foundation-owned waits (bulkhead waiting, retry backoff) are recorded only
//! by their owning decision. Adapter boundaries are ordinary
//! operations, for example `http.response_construction` for each admitted
//! Axum request, and their names share the operation table. A polled
//! supervisor drive records one shutdown: its report's result after it
//! publishes `Stopped` and destroys its owned shutdown future, before the report
//! is returned, or
//! `dropped`/`panicked` (with cause `none` before a cause was selected) when
//! the driver is destroyed first, after the supervisor's own queued work and
//! cleanup are destroyed and recorded. Publishing `Stopped` never waits for
//! recorder code; flush the exporter after awaiting the driver's completion
//! (for example `wait_checked`), not on observing readiness. Its duration runs
//! from the lifecycle stop instant that every drain budget uses to the final
//! report; abandoned drivers have no report and record no duration. A startup
//! that fails before its running driver records its cleanup hooks, not a
//! shutdown.
//!
//! Recording follows the boundary's own ordering and never runs inside the
//! admission lock. A task exit is recorded after a failure closes process
//! admission; root admission closes when drain is requested, shortly after.
//! Process admission decisions, accepted or rejected, are recorded when the
//! submitter's decision is complete; an accepted task cannot start before its
//! decision is recorded. Root admission is recorded after its decision is
//! complete. Root-admitted work is not supervised, so its decision and
//! operations are not ordered with shutdown and may be recorded after
//! `Stopped`. Histograms receive one sample per finished boundary.
//! Aggregation, buckets, temporality and export are the recorder's policy.
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
//! // The application root installs one recorder built on the re-exported
//! // facade, usually an exporter's recorder, before starting the supervisor.
//! # let recorder = batter_core::telemetry::metrics::facade::NoopRecorder;
//! batter_core::telemetry::metrics::install(recorder)?;
//! let context = OperationOwner::new(Duration::from_secs(1))?.into_context();
//! context
//!     .run("example.read", |_| async { Ok::<_, std::io::Error>(()) })
//!     .await?;
//! # Ok(())
//! # }
//! ```

mod installation;
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
use metrics::Unit;
use names::{NameLabel, OPERATION_NAMES, TASK_NAMES};
use std::time::Duration;

pub(crate) use terminal::{AttemptTerminal, BulkheadTerminal, RetryTerminal, ShutdownTerminal};

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

/// Install `recorder` as the process-wide metrics recorder, then publish the
/// catalog's names, units and descriptions to it.
/// Catalog publication targets this recorder even inside a local recorder scope;
/// subsequent observations continue to follow the facade's scoped dispatch.
///
/// This is the canonical application-root setup: descriptions cannot be sent
/// before the recorder exists, and a recorder built on another `metrics` major
/// version does not implement [`facade::Recorder`] and fails to compile.
/// Install exactly once, before the supervisor starts; a second installation
/// returns the rejected recorder. Batter never calls this itself.
///
/// ```
/// use batter_core::telemetry::metrics::{facade, install};
///
/// // Any recorder built on the re-exported facade; exporters usually offer
/// // a builder that returns one instead of installing it themselves.
/// let recorder = facade::NoopRecorder;
/// install(recorder).expect("first installation");
/// ```
///
/// ```compile_fail,E0277
/// // A value that is not a `metrics` 0.24 recorder is rejected at compile time.
/// batter_core::telemetry::metrics::install(42_u8);
/// ```
pub fn install<R>(recorder: R) -> Result<(), facade::SetRecorderError<R>>
where
    R: facade::Recorder + Sync + 'static,
{
    let recorder = installation::install(recorder)?;
    describe(recorder);
    Ok(())
}

fn describe(recorder: &dyn facade::Recorder) {
    let counter = |name: &'static str, unit: Unit, description: &'static str| {
        recorder.describe_counter(name.into(), Some(unit), description.into());
    };
    let histogram = |name: &'static str, unit: Unit, description: &'static str| {
        recorder.describe_histogram(name.into(), Some(unit), description.into());
    };
    counter(
        OPERATION_COMPLETIONS,
        Unit::Count,
        "Completed Batter operation boundaries by outcome",
    );
    histogram(
        OPERATION_DURATION,
        Unit::Seconds,
        "Elapsed Batter operation boundary time",
    );
    counter(
        RETRY_ATTEMPTS,
        Unit::Count,
        "Finished Batter retry attempts by outcome",
    );
    counter(
        RETRY_EXECUTIONS,
        Unit::Count,
        "Terminal Batter retry execution results",
    );
    counter(
        ADMISSION_DECISIONS,
        Unit::Count,
        "Batter admission decisions",
    );
    counter(TASK_EXITS, Unit::Count, "Observed Batter task exits");
    counter(CLEANUP_HOOKS, Unit::Count, "Batter cleanup hook outcomes");
    counter(SHUTDOWNS, Unit::Count, "Batter supervisor drive results");
    histogram(
        SHUTDOWN_DURATION,
        Unit::Seconds,
        "Batter stop instant to shutdown-report time",
    );
    counter(
        LABELS_COALESCED,
        Unit::Count,
        "Metric names replaced by a bounded placeholder",
    );
}

/// Series index of a name label combined with one closed-domain value.
fn named(name: NameLabel, values: &[&str], value: &str) -> Option<usize> {
    Some(name.index * values.len() + index_of(values, value)?)
}

pub(crate) fn operation(
    boundary: Boundary,
    name: &'static str,
    outcome: Outcome,
    elapsed: Duration,
) {
    if matches!(boundary, Boundary::Internal) {
        return;
    }
    let operation = OPERATION_NAMES.label(name);
    let outcome = vocabulary::outcome(outcome, std::thread::panicking());
    let Some(index) = named(operation, &OUTCOMES, outcome) else {
        return;
    };
    let labels = [("operation", operation.text), ("outcome", outcome)];
    increment(COMPLETION_KEYS.key(index, labels), 1);
    sample(DURATION_KEYS.key(index, labels), elapsed.as_secs_f64());
}

fn retry_attempt(name: &'static str, outcome: &'static str) {
    let operation = OPERATION_NAMES.label(name);
    if let Some(index) = named(operation, &OUTCOMES, outcome) {
        let labels = [("operation", operation.text), ("outcome", outcome)];
        increment(ATTEMPT_KEYS.key(index, labels), 1);
    }
}

fn retry_execution(name: &'static str, result: &'static str) {
    let operation = OPERATION_NAMES.label(name);
    if let Some(index) = named(operation, RETRY_RESULTS, result) {
        let labels = [("operation", operation.text), ("result", result)];
        increment(RETRY_KEYS.key(index, labels), 1);
    }
}

fn admission(kind: AdmissionKind, decision: &'static str) {
    let admission = vocabulary::admission_kind(kind);
    let labels = [("admission", admission), ("decision", decision)];
    let key = match kind {
        AdmissionKind::Bulkhead => {
            index_of(BULKHEAD_DECISIONS, decision).map(|index| BULKHEAD_KEYS.key(index, labels))
        }
        AdmissionKind::Process => {
            index_of(PROCESS_DECISIONS, decision).map(|index| PROCESS_KEYS.key(index, labels))
        }
        AdmissionKind::Root => {
            index_of(ROOT_DECISIONS, decision).map(|index| ROOT_KEYS.key(index, labels))
        }
    };
    if let Some(key) = key {
        increment(key, 1);
    }
}

/// A rejected process admission, recorded after the admission lock and any
/// rejected captures are released.
pub(crate) fn process_rejected(error: &ProcessAdmissionError) {
    admission(AdmissionKind::Process, vocabulary::process(Err(error)));
}

/// An accepted process admission, recorded after the admission lock is
/// released and before the task's lease lets it start.
pub(crate) fn process_admitted() {
    admission(AdmissionKind::Process, vocabulary::process(Ok(())));
}

pub(crate) fn root(observed: Readiness) {
    admission(AdmissionKind::Root, vocabulary::root(observed));
}

pub(crate) fn task(finite: bool, name: &'static str, outcome: TaskOutcome) {
    let kind = vocabulary::task_kind(finite);
    let outcome = vocabulary::task_outcome(outcome);
    let task = TASK_NAMES.label(name);
    let Some(kind_index) = index_of(TASK_KINDS, kind) else {
        return;
    };
    if let Some(index) = named(task, TASK_OUTCOMES, outcome) {
        let index = kind_index * NAME_VALUES * TASK_OUTCOMES.len() + index;
        let labels = [("kind", kind), ("task", task.text), ("outcome", outcome)];
        increment(TASK_KEYS.key(index, labels), 1);
    }
}

/// Count `hooks` cleanup hooks with one outcome in a single increment.
pub(crate) fn cleanup(hook: CleanupHook, hooks: usize) {
    let outcome = vocabulary::cleanup(hook);
    if hooks == 0 {
        return;
    }
    if let Some(index) = index_of(CLEANUP_OUTCOMES, outcome) {
        increment(
            CLEANUP_KEYS.key(index, [("outcome", outcome)]),
            hooks as u64,
        );
    }
}

fn shutdown(cause: Option<ShutdownCause>, result: &'static str, elapsed: Option<Duration>) {
    let cause = vocabulary::shutdown_cause(cause);
    let (Some(cause_index), Some(result_index)) = (
        index_of(SHUTDOWN_CAUSES, cause),
        index_of(SHUTDOWN_RESULTS, result),
    ) else {
        return;
    };
    let index = cause_index * SHUTDOWN_RESULTS.len() + result_index;
    increment(
        SHUTDOWN_KEYS.key(index, [("cause", cause), ("result", result)]),
        1,
    );
    if let Some(elapsed) = elapsed {
        let key = SHUTDOWN_DURATION_KEYS.key(result_index, [("result", result)]);
        sample(key, elapsed.as_secs_f64());
    }
}

fn coalesced(domain: NameDomain, reason: Coalesce) {
    let domain = vocabulary::name_domain(domain);
    let reason = vocabulary::coalesce_reason(reason);
    if let (Some(domain_index), Some(reason_index)) = (
        index_of(COALESCE_DOMAINS, domain),
        index_of(COALESCE_REASONS, reason),
    ) {
        let index = domain_index * COALESCE_REASONS.len() + reason_index;
        increment(
            COALESCE_KEYS.key(index, [("domain", domain), ("reason", reason)]),
            1,
        );
    }
}

#[cfg(test)]
mod tests;
