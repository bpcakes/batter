//! Bounded metrics are recorded once per outcome and never alter results.
#![cfg(feature = "metrics")]

#[path = "metrics/capture.rs"]
mod capture;
#[path = "metrics/ordering.rs"]
mod ordering;
#[path = "metrics/retry_destruction.rs"]
mod retry_destruction;
#[path = "metrics/shutdown_destruction.rs"]
mod shutdown_destruction;
#[path = "metrics/terminal.rs"]
mod terminal;

use batter_core::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity},
    cleanup::CleanupBudget,
    lifecycle::{Fatal, ProcessAdmissionError, ProcessCapacity, ShutdownBudget, Supervisor},
    operation::{Interruption, OperationError, OperationOwner},
    retry::{self, ReplaySafety, RetryDecision, RetryPolicy},
    telemetry::metrics::*,
};
use capture::Capture;
use std::{convert::Infallible, time::Duration};

pub(crate) fn owner() -> OperationOwner {
    OperationOwner::new(Duration::from_secs(5)).unwrap()
}

/// The shared registration vocabulary, or a documented placeholder.
fn name_ok(value: &str) -> bool {
    value == INVALID_NAME
        || value == OVERFLOW_NAME
        || (!value.is_empty()
            && value.len() <= MAX_NAME_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
}

/// Sorted label keys of each metric and whether their values are documented.
fn documented(name: &str, values: &[&str]) -> Option<(&'static [&'static str], bool)> {
    Some(match (name, values) {
        (OPERATION_COMPLETIONS | OPERATION_DURATION | RETRY_ATTEMPTS, [operation, outcome]) => (
            &["operation", "outcome"],
            name_ok(operation) && OUTCOMES.contains(outcome),
        ),
        (RETRY_EXECUTIONS, [operation, result]) => (
            &["operation", "result"],
            name_ok(operation) && RETRY_RESULTS.contains(result),
        ),
        (ADMISSION_DECISIONS, [admission, decision]) => (
            &["admission", "decision"],
            match *admission {
                "bulkhead" => BULKHEAD_DECISIONS.contains(decision),
                "process" => PROCESS_DECISIONS.contains(decision),
                "root" => ROOT_DECISIONS.contains(decision),
                _ => false,
            },
        ),
        (TASK_EXITS, [kind, outcome, task]) => (
            &["kind", "outcome", "task"],
            TASK_KINDS.contains(kind) && TASK_OUTCOMES.contains(outcome) && name_ok(task),
        ),
        (CLEANUP_HOOKS, [outcome]) => (&["outcome"], CLEANUP_OUTCOMES.contains(outcome)),
        (SHUTDOWNS, [cause, result]) => (
            &["cause", "result"],
            SHUTDOWN_CAUSES.contains(cause) && SHUTDOWN_RESULTS.contains(result),
        ),
        (SHUTDOWN_DURATION, [result]) => (&["result"], SHUTDOWN_RESULTS.contains(result)),
        (LABELS_COALESCED, [domain, reason]) => (
            &["domain", "reason"],
            COALESCE_DOMAINS.contains(domain) && COALESCE_REASONS.contains(reason),
        ),
        _ => return None,
    })
}

/// Every emitted label belongs to its documented closed domain or name table.
pub(crate) fn assert_catalog(capture: &Capture) {
    for (name, labels) in capture.series() {
        let keys: Vec<&str> = labels.iter().map(|(k, _)| k.as_str()).collect();
        let values: Vec<&str> = labels.iter().map(|(_, v)| v.as_str()).collect();
        let (expected, valid) = documented(&name, &values)
            .unwrap_or_else(|| panic!("undocumented series {name} {labels:?}"));
        assert_eq!(keys, expected, "{name}");
        assert!(valid, "{name} {labels:?}");
    }
    for name in [OPERATION_DURATION, SHUTDOWN_DURATION] {
        for sample in capture.samples(name) {
            assert!(
                sample.value.is_finite() && sample.value >= 0.0,
                "{sample:?}"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn each_operation_outcome_records_one_completion_and_duration() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();

    let ok = context
        .run("example.read", |_| async { Ok::<_, &str>(1) })
        .await;
    assert_eq!(ok.unwrap(), 1);
    let failed = context
        .run("example.read", |_| async { Err::<(), _>("secret detail") })
        .await;
    assert!(matches!(
        failed,
        Err(OperationError::Failed("secret detail"))
    ));

    let cancelled_owner = owner();
    cancelled_owner.cancel();
    let cancelled = cancelled_owner
        .context()
        .run("example.read", |_| async { Ok::<_, &str>(()) })
        .await;
    assert!(matches!(
        cancelled,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));

    let short = OperationOwner::new(Duration::from_millis(10)).unwrap();
    let expired = short
        .context()
        .run("example.read", |_| {
            std::future::pending::<Result<(), &str>>()
        })
        .await;
    assert!(matches!(
        expired,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));

    let dropped = tokio::time::timeout(
        Duration::from_millis(1),
        context.run("example.read", |_| {
            std::future::pending::<Result<(), &str>>()
        }),
    )
    .await;
    assert!(dropped.is_err());

    // `panicked` is covered separately; every other outcome occurred once.
    for outcome in OUTCOMES
        .into_iter()
        .filter(|outcome| *outcome != "panicked")
    {
        let labels = [("operation", "example.read"), ("outcome", outcome)];
        assert_eq!(
            capture.count(OPERATION_COMPLETIONS, &labels),
            1.0,
            "{outcome}"
        );
        let durations: Vec<_> = capture
            .samples(OPERATION_DURATION)
            .into_iter()
            .filter(|s| s.labels.iter().any(|(k, v)| k == "outcome" && v == outcome))
            .collect();
        assert_eq!(durations.len(), 1, "{outcome}");
    }
    assert_eq!(capture.samples(RETRY_ATTEMPTS), []);
    assert_catalog(&capture);
}

#[tokio::test]
async fn raw_identifiers_urls_and_errors_never_become_labels() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();
    let raw: [&'static str; 4] = [
        "https://example.test/items/7?token=abc",
        "/items/7",
        "user@example.test",
        "connection refused: peer reset",
    ];
    for name in raw {
        let result = context
            .run(name, |_| async { Ok::<_, Infallible>(()) })
            .await;
        assert!(result.is_ok());
    }
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("operation", INVALID_NAME)]),
        4.0
    );
    assert_eq!(
        capture.count(
            LABELS_COALESCED,
            &[("domain", "operation"), ("reason", "invalid")]
        ),
        4.0
    );
    for (_, labels) in capture.series() {
        for (_, value) in labels {
            assert!(!raw.contains(&value.as_str()), "{value}");
        }
    }
    assert_catalog(&capture);
}

#[tokio::test]
async fn retry_attempts_are_counted_separately_from_operations() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let owner = owner();
    let policy = RetryPolicy::new(3, Duration::from_millis(1), Duration::from_millis(1)).unwrap();
    let result = retry::execute(
        owner.context(),
        "provider.read",
        ReplaySafety::Idempotent,
        &policy,
        |attempt| async move {
            if attempt.number < 2 {
                Err("transient")
            } else {
                Ok(7)
            }
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert_eq!(result.unwrap(), 7);
    let attempt = |outcome| {
        capture.count(
            RETRY_ATTEMPTS,
            &[("operation", "provider.read"), ("outcome", outcome)],
        )
    };
    assert_eq!(attempt("failed"), 1.0);
    assert_eq!(attempt("succeeded"), 1.0);
    assert_eq!(
        capture.count(
            RETRY_EXECUTIONS,
            &[("operation", "provider.read"), ("result", "succeeded")]
        ),
        1.0
    );
    // Neither attempts nor foundation-owned backoff waits are operations.
    assert_eq!(capture.samples(OPERATION_COMPLETIONS), []);
    assert_eq!(capture.samples(OPERATION_DURATION), []);
    assert_catalog(&capture);
}

async fn bulkhead_storm(capture: &Capture, storm: usize) -> Vec<String> {
    let _recorder = facade::set_default_local_recorder(capture);
    let context = owner().into_context();
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let held = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    let mut results = Vec::with_capacity(storm);
    for _ in 0..storm {
        let result = bulkhead.enter(&context, Admission::Reject).await;
        results.push(format!("{:?}", result.map(|_| ())));
    }
    drop(held);
    bulkhead.close();
    let closed = bulkhead.enter(&context, Admission::Wait).await;
    assert!(matches!(closed, Err(AdmissionError::Closed)));
    results
}

#[tokio::test]
async fn rejection_storm_under_a_saturated_recorder_keeps_decisions_and_bounds() {
    const STORM: usize = 10_000;
    let complete = Capture::unbounded();
    let saturated = Capture::saturating_after(5);
    let observed = bulkhead_storm(&complete, STORM).await;
    let degraded = bulkhead_storm(&saturated, STORM).await;

    assert_eq!(
        observed, degraded,
        "diagnostics changed admission decisions"
    );
    assert!(observed.iter().all(|r| r == "Err(Overloaded)"));
    let decision = |capture: &Capture, decision| {
        capture.count(
            ADMISSION_DECISIONS,
            &[("admission", "bulkhead"), ("decision", decision)],
        )
    };
    assert_eq!(decision(&complete, "admitted"), 1.0);
    assert_eq!(decision(&complete, "overloaded"), STORM as f64);
    assert_eq!(decision(&complete, "closed"), 1.0);
    // The waiting admission is not also recorded as an operation.
    assert_eq!(complete.samples(OPERATION_COMPLETIONS), []);
    // Rejection by the recorder is neither retried nor reported to Batter:
    // both recorders received exactly one registration per decision.
    assert_eq!(complete.registrations(), saturated.registrations());
    assert_eq!(saturated.discarded(), saturated.registrations() - 5);
    // A storm adds samples to existing series, never new series.
    assert_eq!(
        complete
            .series()
            .iter()
            .filter(|(name, _)| name == ADMISSION_DECISIONS)
            .count(),
        3
    );
    assert_catalog(&complete);
}

pub(crate) fn shutdown_budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        second,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn process_tasks_cleanup_and_shutdown_record_bounded_outcomes() {
    const STORM: usize = 1_000;
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let mut supervisor =
        Supervisor::with_process_capacity(shutdown_budget(), ProcessCapacity::new(1).unwrap());
    let process = supervisor.process_handle().unwrap();
    supervisor
        .register("service-component", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    supervisor
        .on_cleanup("dependency.close", || async { Ok(()) })
        .unwrap();
    let early = process.try_spawn("job.run", |_| async { Ok::<_, Fatal<Infallible>>(()) });
    assert!(matches!(early, Err(ProcessAdmissionError::NotRunning)));

    let handle = supervisor.handle();
    let running = supervisor.start();
    handle.status().wait_ready().await.unwrap();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let held = process
        .try_spawn("job.run", |_| async move {
            let _ = released.await;
            Ok::<_, Fatal<Infallible>>(())
        })
        .unwrap();
    for _ in 0..STORM {
        let rejected = process.try_spawn("job.run", |_| async { Ok::<_, Fatal<Infallible>>(()) });
        assert!(matches!(rejected, Err(ProcessAdmissionError::Full)));
    }
    release.send(()).unwrap();
    held.wait().await.unwrap();
    let report = running.shutdown_checked().await.unwrap();
    assert_eq!(report.report().completed_process_tasks, 1);

    let process_decision = |decision| {
        capture.count(
            ADMISSION_DECISIONS,
            &[("admission", "process"), ("decision", decision)],
        )
    };
    assert_eq!(process_decision("not_running"), 1.0);
    assert_eq!(process_decision("admitted"), 1.0);
    assert_eq!(process_decision("full"), STORM as f64);
    let exits = |labels: &[(&str, &str)]| capture.count(TASK_EXITS, labels);
    assert_eq!(
        exits(&[
            ("kind", "process"),
            ("task", "job.run"),
            ("outcome", "completed")
        ]),
        1.0
    );
    assert_eq!(
        exits(&[
            ("kind", "component"),
            ("task", "service-component"),
            ("outcome", "stopped")
        ]),
        1.0
    );
    assert_eq!(
        capture.count(CLEANUP_HOOKS, &[("outcome", "succeeded")]),
        1.0
    );
    assert_eq!(
        capture.count(SHUTDOWNS, &[("cause", "requested"), ("result", "success")]),
        1.0
    );
    assert_eq!(capture.samples(SHUTDOWN_DURATION).len(), 1);
    // A process admission is recorded before its task's exit, and both before
    // the shutdown that an application root may use as its flush signal.
    let admitted = capture
        .first(
            ADMISSION_DECISIONS,
            &[("admission", "process"), ("decision", "admitted")],
        )
        .unwrap();
    let exited = capture
        .first(TASK_EXITS, &[("task", "job.run"), ("outcome", "completed")])
        .unwrap();
    let shutdown = capture.first(SHUTDOWNS, &[]).unwrap();
    assert!(admitted < exited && exited < shutdown);
    assert!(capture.series().len() <= MAX_SERIES);
    assert_catalog(&capture);
}

#[tokio::test]
async fn skipped_cleanup_and_failed_shutdown_are_distinct_outcomes() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let mut supervisor = Supervisor::new(shutdown_budget());
    supervisor
        .register("service.component", |_startup| async move {
            panic!("component failed")
        })
        .unwrap();
    supervisor
        .on_cleanup("dependency.close", || async { Ok(()) })
        .unwrap();
    let running = supervisor.start();
    assert!(running.wait_checked().await.is_err());
    assert_eq!(
        capture.count(
            TASK_EXITS,
            &[("task", "service.component"), ("outcome", "panicked")]
        ),
        1.0
    );
    assert_eq!(capture.count(CLEANUP_HOOKS, &[("outcome", "skipped")]), 1.0);
    assert_eq!(
        capture.count(
            SHUTDOWNS,
            &[("cause", "component_exit"), ("result", "failure")]
        ),
        1.0
    );
    assert_catalog(&capture);
}

#[tokio::test]
async fn results_are_identical_without_any_recorder() {
    let context = owner().into_context();
    let result = context
        .run("example.read", |_| async { Ok::<_, Infallible>(3) })
        .await;
    assert_eq!(result.unwrap(), 3);
}
