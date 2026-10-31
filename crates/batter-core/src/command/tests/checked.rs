//! `into_checked` against `is_success`, and success formatting.

use crate::{
    PanicPayload,
    cleanup::{CleanupOutcome, CleanupRecord, CleanupReport, SkipReason, SkippedCleanup},
    command::{CommandCause, CommandFailure, CommandReport, CommandSuccess},
    operation::Interruption,
};
use std::{convert::Infallible, fmt, io, sync::Arc};
use tokio::task::JoinError;

/// Deliberately no derives, so only identity can be compared.
struct Value(u32);

struct WorkError(&'static str);

const STAGE: &str = "checked.stage";

#[derive(Clone, Copy, Debug)]
enum Cleanup {
    Succeeded,
    Failed,
    Skipped,
    Coordinator,
}

#[derive(Clone, Copy, Debug)]
struct Case {
    work: Result<(), ()>,
    interruption: Option<Interruption>,
    destruction_panic: Option<()>,
    cleanup: Cleanup,
}

impl Case {
    /// The contract, stated independently of the implementation.
    fn succeeds(self) -> bool {
        self.work.is_ok()
            && self.interruption.is_none()
            && self.destruction_panic.is_none()
            && matches!(self.cleanup, Cleanup::Succeeded)
    }

    fn report(self, coordinator: &Arc<JoinError>) -> CommandReport<Value, WorkError> {
        CommandReport {
            stage: STAGE,
            work: match self.work {
                Ok(()) => Ok(Value(11)),
                Err(()) => Err(CommandCause::Failed(WorkError("work-marker"))),
            },
            interruption_after_work: self.interruption,
            destruction_panic: self
                .destruction_panic
                .map(|()| PanicPayload::new(Box::new("p"))),
            cleanup: match self.cleanup {
                Cleanup::Succeeded => Ok(cleanup_report(CleanupOutcome::Succeeded, None)),
                Cleanup::Failed => Ok(cleanup_report(CleanupOutcome::Failed, None)),
                Cleanup::Skipped => Ok(cleanup_report(
                    CleanupOutcome::Succeeded,
                    Some(SkipReason::BudgetExhausted),
                )),
                Cleanup::Coordinator => Err(coordinator.clone()),
            },
        }
    }
}

fn cleanup_report(outcome: CleanupOutcome, skipped: Option<SkipReason>) -> CleanupReport {
    let error = (outcome != CleanupOutcome::Succeeded)
        .then(|| Box::new(io::Error::other("cleanup-marker")) as crate::BoxError);
    CleanupReport {
        records: vec![CleanupRecord {
            name: "checked.hook",
            outcome,
            error,
        }],
        skipped: skipped
            .map(|reason| SkippedCleanup {
                name: "skipped.hook",
                reason,
            })
            .into_iter()
            .collect(),
    }
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for work in [Ok(()), Err(())] {
        for interruption in [None, Some(Interruption::Cancelled)] {
            for destruction_panic in [None, Some(())] {
                for cleanup in [
                    Cleanup::Succeeded,
                    Cleanup::Failed,
                    Cleanup::Skipped,
                    Cleanup::Coordinator,
                ] {
                    cases.push(Case {
                        work,
                        interruption,
                        destruction_panic,
                        cleanup,
                    });
                }
            }
        }
    }
    cases
}

async fn coordinator_loss() -> Arc<JoinError> {
    let task = tokio::spawn(std::future::pending::<()>());
    task.abort();
    Arc::new(
        task.await
            .expect_err("an aborted task reports cancellation"),
    )
}

fn record_names(cleanup: &CleanupReport) -> Vec<(&'static str, CleanupOutcome)> {
    cleanup
        .records
        .iter()
        .map(|record| (record.name, record.outcome))
        .collect()
}

fn assert_success(case: Case, success: &CommandSuccess<Value>) {
    assert_eq!(success.value().0, 11, "{case:?}");
    assert_eq!(success.stage(), STAGE, "{case:?}");
    assert_eq!(
        record_names(success.cleanup()),
        [("checked.hook", CleanupOutcome::Succeeded)],
        "{case:?}"
    );
    assert!(success.cleanup().skipped.is_empty(), "{case:?}");
}

fn assert_retained(case: Case, report: &CommandReport<Value, WorkError>, loss: &Arc<JoinError>) {
    assert_eq!(report.stage, STAGE, "{case:?}");
    match (&report.work, case.work) {
        (Ok(value), Ok(())) => assert_eq!(value.0, 11, "{case:?}"),
        (Err(CommandCause::Failed(error)), Err(())) => assert_eq!(error.0, "work-marker"),
        _ => panic!("work outcome was replaced: {case:?}"),
    }
    assert_eq!(
        report.interruption_after_work, case.interruption,
        "{case:?}"
    );
    match (&report.destruction_panic, case.destruction_panic) {
        (Some(payload), Some(())) => payload
            .try_inspect(|value| assert_eq!(value.downcast_ref::<&str>(), Some(&"p")))
            .expect("payload inspection is uncontended"),
        (None, None) => {}
        _ => panic!("destruction panic was replaced: {case:?}"),
    }
    match (&report.cleanup, case.cleanup) {
        (Err(retained), Cleanup::Coordinator) => assert!(Arc::ptr_eq(retained, loss)),
        (Ok(cleanup), Cleanup::Succeeded | Cleanup::Skipped) => {
            assert_eq!(
                record_names(cleanup),
                [("checked.hook", CleanupOutcome::Succeeded)]
            );
            let skipped: Vec<_> = cleanup
                .skipped
                .iter()
                .map(|skip| (skip.name, skip.reason))
                .collect();
            if matches!(case.cleanup, Cleanup::Skipped) {
                assert_eq!(skipped, [("skipped.hook", SkipReason::BudgetExhausted)]);
            } else {
                assert!(skipped.is_empty(), "{case:?}");
            }
        }
        (Ok(cleanup), Cleanup::Failed) => {
            assert_eq!(
                record_names(cleanup),
                [("checked.hook", CleanupOutcome::Failed)]
            );
            let error = cleanup.records[0].error.as_ref().expect("error retained");
            assert_eq!(error.to_string(), "cleanup-marker");
        }
        _ => panic!("cleanup outcome was replaced: {case:?}"),
    }
}

#[tokio::test]
async fn into_checked_agrees_with_is_success_and_retains_every_failed_field() {
    let loss = coordinator_loss().await;
    let cases = cases();
    assert_eq!(cases.len(), 32);
    let mut successes = 0;
    for case in cases {
        let expected = case.report(&loss).is_success();
        assert_eq!(expected, case.succeeds(), "{case:?}");
        match case.report(&loss).into_checked() {
            Ok(success) => {
                assert!(expected, "unsuccessful report was checked: {case:?}");
                assert_success(case, &success);
                successes += 1;
            }
            Err(CommandFailure::Report(report)) => {
                assert!(!expected, "successful report was rejected: {case:?}");
                assert_retained(case, &report, &loss);
            }
            Err(CommandFailure::Coordinator(_)) => panic!("a report was lost: {case:?}"),
        }
    }
    assert_eq!(successes, 1);
}

/// Formats as a secret in both Display and Debug.
struct Secret;

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("secret-value")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("secret-value")
    }
}

fn secret_report(interruption: Option<Interruption>) -> CommandReport<Secret, Infallible> {
    CommandReport {
        stage: "format.stage",
        work: Ok(Secret),
        interruption_after_work: interruption,
        destruction_panic: None,
        cleanup: Ok(cleanup_report(CleanupOutcome::Succeeded, None)),
    }
}

#[test]
fn success_and_failure_formatting_omit_the_work_value() {
    let Ok(success) = secret_report(None).into_checked() else {
        panic!("a successful report was rejected")
    };
    let Err(failure) = secret_report(Some(Interruption::Cancelled)).into_checked() else {
        panic!("a late interruption was checked as success")
    };
    for text in [
        format!("{success}"),
        format!("{success:?}"),
        format!("{success:#?}"),
        format!("{failure}"),
        format!("{failure:?}"),
    ] {
        assert!(!text.contains("secret-value"), "{text}");
        assert!(text.contains("format.stage"), "{text}");
    }
    // The value itself stays reachable by explicit access.
    assert_eq!(format!("{:?}", success.value()), "secret-value");
}
