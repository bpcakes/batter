use std::{
    future,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
    time::Duration,
};

use super::*;

fn rejected<T>() -> future::Ready<Result<T, sqlx::Error>> {
    future::ready(Err(sqlx::Error::RowNotFound))
}

#[tokio::test]
async fn a_client_deadline_closes_the_connection_in_both_families() {
    let outcome = before_commit::<Check, ()>(
        Instant::now(),
        CheckPhase::UpdatingCounters,
        future::pending(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ConnectionOutcome::MustClose(
            CheckError::TimedOutBeforeCommit {
                phase: CheckPhase::UpdatingCounters
            }
        ))
    ));

    let outcome = before_commit::<Maintenance, ()>(
        Instant::now(),
        CleanupPhase::DeletingExpiredWindows,
        future::pending(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ConnectionOutcome::MustClose(
            MaintenanceError::TimedOutBeforeCommit {
                phase: CleanupPhase::DeletingExpiredWindows
            }
        ))
    ));
}

#[tokio::test]
async fn a_database_error_before_commit_keeps_the_connection_in_both_families() {
    let deadline = Instant::now() + Duration::from_secs(1);

    let outcome =
        before_commit::<Check, ()>(deadline, CheckPhase::UpdatingCounters, rejected()).await;
    assert!(matches!(
        outcome,
        Err(ConnectionOutcome::Reusable(
            CheckError::DefinitelyNotConsumed(sqlx::Error::RowNotFound)
        ))
    ));

    let outcome = before_commit::<Maintenance, ()>(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        rejected(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ConnectionOutcome::Reusable(MaintenanceError::Database(
            sqlx::Error::RowNotFound
        )))
    ));
}

/// A commit stand-in that records whether it was ever polled.
fn recorded_commit(
    polled: &AtomicBool,
    outcome: Poll<Result<(), sqlx::Error>>,
) -> impl Future<Output = Result<(), sqlx::Error>> {
    let mut outcome = Some(outcome);
    future::poll_fn(move |_| {
        polled.store(true, Ordering::Relaxed);
        match outcome.take() {
            Some(Poll::Ready(result)) => Poll::Ready(result),
            Some(Poll::Pending) | None => Poll::Pending,
        }
    })
}

#[tokio::test]
async fn an_exhausted_budget_never_polls_the_commit_in_either_family() {
    // Even a commit that would succeed at once must not be attempted.
    let polled = AtomicBool::new(false);
    let outcome = attempt_commit::<Maintenance>(
        Instant::now(),
        recorded_commit(&polled, Poll::Ready(Ok(()))),
    )
    .await;
    let Err(ConnectionOutcome::Reusable(error)) = outcome else {
        panic!("an exhausted cleanup budget is a definite pre-commit timeout")
    };
    assert!(matches!(
        error,
        MaintenanceError::TimedOutBeforeCommit {
            phase: CleanupPhase::StartingCommit
        }
    ));
    assert!(!error.may_have_removed_rows());
    assert!(!polled.load(Ordering::Relaxed));

    let polled = AtomicBool::new(false);
    let outcome = attempt_commit::<Check>(
        Instant::now(),
        recorded_commit(&polled, Poll::Ready(Ok(()))),
    )
    .await;
    let Err(ConnectionOutcome::Reusable(error)) = outcome else {
        panic!("an exhausted check budget is a definite pre-commit timeout")
    };
    assert!(matches!(
        error,
        CheckError::TimedOutBeforeCommit {
            phase: CheckPhase::StartingCommit
        }
    ));
    assert_eq!(
        error.consumption(),
        runlimit_core::ConsumptionStatus::NotConsumed
    );
    assert!(!polled.load(Ordering::Relaxed));
}

#[tokio::test]
async fn an_attempted_commit_reports_its_acknowledgement_or_an_uncertain_error() {
    let budget = || Instant::now() + Duration::from_secs(5);

    let polled = AtomicBool::new(false);
    attempt_commit::<Maintenance>(budget(), recorded_commit(&polled, Poll::Ready(Ok(()))))
        .await
        .unwrap_or_else(|_| panic!("an acknowledged commit succeeds"));
    assert!(polled.load(Ordering::Relaxed));

    let polled = AtomicBool::new(false);
    let outcome = attempt_commit::<Maintenance>(
        budget(),
        recorded_commit(&polled, Poll::Ready(Err(sqlx::Error::PoolClosed))),
    )
    .await;
    let Err(ConnectionOutcome::Reusable(error)) = outcome else {
        panic!("a commit error leaves an idle connection")
    };
    assert!(matches!(error, MaintenanceError::CommitOutcomeUnknown(_)));
    assert!(error.may_have_removed_rows());
}

// The clock is paused, so the deadline cannot pass before the commit's
// first poll: it advances only once the runtime is idle on that commit.
#[tokio::test(start_paused = true)]
async fn a_deadline_during_an_attempted_commit_is_uncertain_and_closes_the_connection() {
    let polled = AtomicBool::new(false);
    let outcome = attempt_commit::<Maintenance>(
        Instant::now() + Duration::from_millis(20),
        recorded_commit(&polled, Poll::Pending),
    )
    .await;
    let Err(ConnectionOutcome::MustClose(error)) = outcome else {
        panic!("a commit that outlives the deadline leaves a busy connection")
    };
    assert!(matches!(error, MaintenanceError::CommitTimedOut));
    assert!(error.may_have_removed_rows());
    assert!(polled.load(Ordering::Relaxed));

    let polled = AtomicBool::new(false);
    let outcome = attempt_commit::<Check>(
        Instant::now() + Duration::from_millis(20),
        recorded_commit(&polled, Poll::Pending),
    )
    .await;
    let Err(ConnectionOutcome::MustClose(error)) = outcome else {
        panic!("a check commit that outlives the deadline leaves a busy connection")
    };
    assert!(matches!(error, CheckError::CommitTimedOut));
    assert_eq!(
        error.consumption(),
        runlimit_core::ConsumptionStatus::PossiblyConsumed
    );
    assert!(polled.load(Ordering::Relaxed));
}

#[test]
fn each_family_names_its_own_timeout_refresh_phase() {
    assert_eq!(
        Check::refreshing_timeouts(CheckPhase::PreflightingCounters),
        CheckPhase::PreflightingCounters
    );
    assert_eq!(
        Maintenance::refreshing_timeouts(CleanupPhase::DeletingExpiredWindows),
        CleanupPhase::ConfiguringTimeouts
    );
}

#[test]
fn only_an_attempted_commit_is_uncertain() {
    assert!(
        !Maintenance::timed_out_before_commit(Maintenance::STARTING_COMMIT).may_have_removed_rows()
    );
    assert!(!Maintenance::failed_before_commit(sqlx::Error::RowNotFound).may_have_removed_rows());
    assert!(Maintenance::commit_timed_out().may_have_removed_rows());
    assert!(Maintenance::commit_outcome_unknown(sqlx::Error::RowNotFound).may_have_removed_rows());

    let not_consumed = runlimit_core::ConsumptionStatus::NotConsumed;
    let possibly_consumed = runlimit_core::ConsumptionStatus::PossiblyConsumed;
    assert_eq!(
        Check::timed_out_before_commit(Check::STARTING_COMMIT).consumption(),
        not_consumed
    );
    assert_eq!(
        Check::failed_before_commit(sqlx::Error::RowNotFound).consumption(),
        not_consumed
    );
    assert_eq!(Check::commit_timed_out().consumption(), possibly_consumed);
    assert_eq!(
        Check::commit_outcome_unknown(sqlx::Error::RowNotFound).consumption(),
        possibly_consumed
    );
}
