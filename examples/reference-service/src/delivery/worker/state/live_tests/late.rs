//! Actual final-state SQL and native handler outcomes for late retained actions.
use super::super::super::phased::{Action, Invocation, Progress};
use super::*;
use batter::runledger::{LateJobFinalization, LateJobSettlement, test_support::HandlerExit};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Probe<'a> {
    invocation: Invocation<'a>,
    action: Mutex<Option<Action>>,
    late: Arc<AtomicBool>,
}
#[async_trait]
impl PhasedJobHandler for Probe<'_> {
    type Output = Action;
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }
    async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<Action, JobFailure> {
        // Deterministically cross the work deadline inside its returning poll.
        // No network timing assumption; production settlement and SQL stay real.
        std::thread::sleep(
            scope
                .context()
                .deadline()
                .saturating_duration_since(tokio::time::Instant::now())
                + Duration::from_millis(2),
        );
        Ok(self.action.lock().unwrap().take().unwrap())
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, Action, JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        self.invocation.finalize(scope).await
    }
    fn settle_late<'a>(
        &'a self,
        scope: LateJobFinalization<'a, Action>,
    ) -> LateJobSettlement<'a, JobFailure> {
        self.late.store(true, Ordering::SeqCst);
        self.invocation.settle_late(scope)
    }
    fn failed(&self, failure: PhasedJobFailure<Action, JobFailure>) -> JobFailure {
        self.invocation.failed(failure)
    }
}

#[derive(Clone, Copy, Debug)]
enum Case {
    Confirm,
    Deny,
    Undispatched,
    Manual,
    Stale,
    LeaseLost,
    CommitFailed,
}

pub(super) async fn settlement(worker: &DeliveryWorker) -> Result<(), BoxError> {
    sqlx::raw_sql(
        "CREATE FUNCTION reference_test_reject_commit() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'fixture commit failure'; END $$;
         CREATE CONSTRAINT TRIGGER reference_test_reject_commit
         AFTER UPDATE ON reference_delivery_effects DEFERRABLE INITIALLY DEFERRED
         FOR EACH ROW WHEN (NEW.outcome_code = 'fixture_commit_failure')
         EXECUTE FUNCTION reference_test_reject_commit();",
    )
    .execute(&worker.pool)
    .await?;
    for case in [
        Case::Confirm,
        Case::Deny,
        Case::Undispatched,
        Case::Manual,
        Case::Stale,
        Case::LeaseLost,
        Case::CommitFailed,
    ] {
        check_case(worker, case).await?;
    }
    sqlx::raw_sql("DROP TRIGGER reference_test_reject_commit ON reference_delivery_effects; DROP FUNCTION reference_test_reject_commit();")
        .execute(&worker.pool).await?;
    Ok(())
}

async fn prepare_case(
    worker: &DeliveryWorker,
    case: Case,
) -> Result<(JobContext, DeliveryJobPayload), BoxError> {
    let (mut context, payload) = claimed(&worker.pool).await?;
    // A confirmed/denied last attempt must return success, not consume a
    // nonexistent retry to discover the terminal application state.
    if matches!(case, Case::Confirm | Case::Deny) {
        context.attempt = sqlx::query_scalar(
            "UPDATE job_queue SET attempt = max_attempts WHERE id = $1 RETURNING attempt",
        )
        .bind(context.job_id)
        .fetch_one(&worker.pool)
        .await?;
        assert!(context.attempt > 1);
    }
    in_invocation(PROBE_BUDGET, context.clone(), {
        let (worker, payload) = (worker.clone(), payload.clone());
        move |execution| {
            Box::pin(async move {
                worker
                    .mark_dispatch_possible(
                        execution,
                        &payload,
                        ProviderEffectState::AwaitingAttempt,
                    )
                    .await
            })
        }
    })
    .await
    .map_err(|_| "late fixture could not record dispatch fence")?;
    if matches!(case, Case::LeaseLost) {
        sqlx::query("UPDATE job_queue SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
                .bind(context.job_id).execute(&worker.pool).await?;
    }
    if matches!(case, Case::Stale) {
        sqlx::query("UPDATE reference_records SET generation = generation + 1 WHERE id = $1")
            .bind(payload.record_id)
            .execute(&worker.pool)
            .await?;
    }
    Ok((context, payload))
}

fn action(case: Case) -> Action {
    match case {
        Case::Confirm | Case::Stale => Action::Confirm("fixture-accepted".into()),
        Case::Deny | Case::LeaseLost => Action::Deny {
            code: "fixture_denied",
            expected: ProviderEffectState::ReconcileNeeded,
        },
        Case::CommitFailed => Action::Deny {
            code: "fixture_commit_failure",
            expected: ProviderEffectState::ReconcileNeeded,
        },
        Case::Undispatched => Action::Undispatched(Duration::from_secs(60)),
        Case::Manual => Action::Manual {
            code: "fixture_conflict",
            message: "Conflicting retained truth.",
            expected: ProviderEffectState::ReconcileNeeded,
        },
    }
}

async fn check_case(worker: &DeliveryWorker, case: Case) -> Result<(), BoxError> {
    let (context, payload) = prepare_case(worker, case).await?;
    let invocation = Invocation::new(worker, &payload);
    invocation.progress(Progress::Dispatch);
    let late = Arc::new(AtomicBool::new(false));
    let probe = Probe {
        invocation,
        action: Mutex::new(Some(action(case))),
        late: late.clone(),
    };
    let handler = PhasedJob::new(probe, Duration::from_millis(2900))?;
    let run = HandlerInvocation::for_context(Duration::from_secs(3), context)?
        .execute(&handler, Value::Null)
        .await;
    assert!(
        late.load(Ordering::SeqCst),
        "{case:?}: explicit late settlement selected"
    );
    let retained: RetainedOutcome = sqlx::query_as(
            "SELECT state, provider_effect_id, acceptance_possible,
             retry_not_before IS NOT NULL AS retry_eligible FROM reference_delivery_effects WHERE delivery_id = $1",
        )
        .bind(payload.delivery_id)
        .fetch_one(&worker.pool)
        .await?;
    assert_outcome(case, run, retained);
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RetainedOutcome {
    state: String,
    provider_effect_id: Option<String>,
    acceptance_possible: bool,
    retry_eligible: bool,
}

fn assert_completed(
    case: Case,
    run: &batter::runledger::test_support::HandlerRun,
    retained: &RetainedOutcome,
) {
    assert!(
        matches!(run.exit, HandlerExit::Returned(Ok(ref completion))
        if completion.disposition() == runledger_core::jobs::JobCompletionDisposition::Succeed),
        "{case:?}: {:?}",
        run.exit
    );
    assert_eq!(
        retained.state,
        if matches!(case, Case::Confirm) {
            "CONFIRMED"
        } else {
            "BUSINESS_DENIED"
        }
    );
    assert!(!retained.acceptance_possible);
    assert_eq!(
        retained.provider_effect_id.as_deref(),
        matches!(case, Case::Confirm).then_some("fixture-accepted")
    );
}

fn assert_outcome(
    case: Case,
    run: batter::runledger::test_support::HandlerRun,
    retained: RetainedOutcome,
) {
    match case {
        Case::Confirm | Case::Deny => assert_completed(case, &run, &retained),
        Case::Undispatched => {
            let failure = returned_failure(run);
            assert_eq!(
                failure.retry_timing(),
                Some(runledger_core::jobs::JobRetryTiming::After(
                    Duration::from_secs(60)
                ))
            );
            assert_eq!(retained.state, "RETRYABLE_UNDISPATCHED");
            assert!(!retained.acceptance_possible);
            assert!(retained.retry_eligible);
        }
        Case::Manual | Case::Stale => {
            assert_eq!(returned_failure(run).code, CODE_MANUAL_RESOLUTION);
            assert_eq!(retained.state, "MANUAL_RESOLUTION");
        }
        Case::LeaseLost | Case::CommitFailed => {
            assert_eq!(
                returned_failure(run).code,
                if matches!(case, Case::LeaseLost) {
                    CODE_LEASE_LOST
                } else {
                    CODE_STATE_UNAVAILABLE
                }
            );
            assert_eq!(
                retained.state, "RECONCILE_NEEDED",
                "failed acknowledgement cannot publish success"
            );
            assert!(retained.acceptance_possible);
        }
    }
}
