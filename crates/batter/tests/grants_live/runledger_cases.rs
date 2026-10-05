//! Native Runledger operations under their selected grants.

use super::policy::{Composition, PublicPolicy};
use super::schema;
use super::support::{
    CaseRejection, Fixture, PgAtomicFailure, Result, quote, require, require_denied,
    require_within_policy,
};
use batter::runledger::grants::RunledgerOperation;
use batter::runledger::native::core::jobs::JobType;
use batter::runledger::native::postgres::jobs::JobEnqueueIntent;
use batter::runledger::native::postgres::{Error as NativeError, RequiredIntentError};
use batter::runledger::{PgAtomicError, run_atomic};
use batter::sqlx::PgScopeError;

/// The native stable code for a differing request under an existing key.
const INTENT_CONFLICT_CODE: &str = "job.intent_idempotency_conflict";

/// The classified code a rejected required-intent recording carries, if any.
///
/// Matching the stable code rather than the message keeps the assertion
/// independent of wording, and distinguishes the conflict this case stages from
/// a permission refusal that arrives through the same variant.
fn conflict_code<T>(
    outcome: &std::result::Result<T, PgAtomicError<T, PgScopeError<RequiredIntentError>>>,
) -> Option<&'static str> {
    match outcome {
        Err(PgAtomicError::Rejected(PgScopeError::Application(RequiredIntentError::Storage(
            NativeError::QueryError(query),
        )))) => Some(query.code()),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
async fn intent_submission_records_duplicates_conflicts_and_rolls_back_in_both_scopes() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs = fixture.names.jobs.clone();
        let application = fixture.names.application.clone();
        let role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs, &[RunledgerOperation::IntentSubmission])
            .with_application(&application)
            .compile()?;
        let login = fixture.login("intent", &role).await?;
        let probe = fixture.pool(&login, &[&jobs, &application]).await?;
        require_within_policy(&probe, &role).await?;

        let database = schema::runledger_database(fixture, &login, &jobs, 2)?;
        let payload = serde_json::json!({"note": "Composed"});
        let organization = uuid::Uuid::new_v4();
        for scoped in [false, true] {
            let key = format!("composed-{scoped}");
            let build = || {
                let intent = JobEnqueueIntent::new(
                    JobType::new("batter.grants.intent"),
                    &payload,
                    key.as_str(),
                );
                if scoped {
                    intent.with_organization_id(organization)
                } else {
                    intent
                }
            };
            // A first recording, then an exact duplicate inside one application
            // transaction that also writes an application relation.
            let first = run_atomic(&database, async |mut scope| {
                let intent = build();
                scope.record_required_job_enqueue_intent(&intent).await
            })
            .await
            .map_err(PgAtomicFailure::into_error)?;
            let repeated = run_atomic(&database, async |mut scope| {
                let intent = build();
                scope.record_required_job_enqueue_intent(&intent).await
            })
            .await
            .map_err(PgAtomicFailure::into_error)?;
            require(
                first.intent_id() == repeated.intent_id(),
                "an exact duplicate request produced a second intent",
            )?;

            // A different request under the same key must conflict.
            let conflicting = serde_json::json!({"note": "Different"});
            let conflict = run_atomic(&database, async |mut scope| {
                let intent = JobEnqueueIntent::new(
                    JobType::new("batter.grants.intent"),
                    &conflicting,
                    key.as_str(),
                );
                let intent = if scoped {
                    intent.with_organization_id(organization)
                } else {
                    intent
                };
                scope.record_required_job_enqueue_intent(&intent).await
            })
            .await;
            // The rejection has to be the idempotency conflict itself. A bare
            // `Rejected` also covers a permission refusal, which would make a
            // missing privilege read as correct conflict handling, so this
            // asserts the native error's own stable code. The native layer
            // reports a differing request under an existing key as a storage
            // error carrying that classified conflict, not as
            // `RequiredIntentError::Conflict`, which is reserved for an
            // already-conflicted outcome the call returned successfully.
            require(
                conflict_code(&conflict) == Some(INTENT_CONFLICT_CODE),
                &format!(
                    "a conflicting request under one key was not rejected as {INTENT_CONFLICT_CODE}: {conflict:?}"
                ),
            )?;

            // A rejected scope rolls back both the application write and the intent.
            let rolled_back_key = format!("{key}-rolled-back");
            let rolled_back = run_atomic(&database, async |mut scope| {
                let intent = JobEnqueueIntent::new(
                    JobType::new("batter.grants.intent"),
                    &payload,
                    rolled_back_key.as_str(),
                );
                let intent = if scoped {
                    intent.with_organization_id(organization)
                } else {
                    intent
                };
                scope
                    .record_required_job_enqueue_intent(&intent)
                    .await
                    .map_err(|error| CaseRejection::Native(Box::new(error)))?;
                Err::<(), CaseRejection>(CaseRejection::Application("application rejected"))
            })
            .await;
            // Only the case's own rejection proves the rollback path. A native
            // failure recording the probe also arrives as `Rejected`, and the
            // absence check below would then pass because nothing was ever
            // inserted, reporting rollback evidence the run never reached.
            require(
                matches!(
                    rolled_back,
                    Err(PgAtomicError::Rejected(CaseRejection::Application(_)))
                ),
                &format!("the deliberate application rejection was not what rolled back: {rolled_back:?}"),
            )?;
            let retained: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT EXISTS(SELECT 1 FROM {}.job_enqueue_intents WHERE idempotency_key = $1)",
                quote(&jobs)
            )))
            .bind(&rolled_back_key)
            .fetch_one(&probe)
            .await?;
            require(!retained, "a rolled-back intent survived its transaction")?;
        }

        // An intent-submission login cannot read the decoded payload, promote,
        // claim, or synchronize the catalog.
        for forbidden in [
            format!("SELECT payload FROM {}.job_enqueue_intents", quote(&jobs)),
            format!(
                "UPDATE {}.job_enqueue_intents SET status = 'PROMOTED'",
                quote(&jobs)
            ),
            format!("SELECT 1 FROM {}.job_queue", quote(&jobs)),
            format!(
                "INSERT INTO {}.job_definitions (job_type) VALUES ('x')",
                quote(&jobs)
            ),
            format!("DELETE FROM {}.job_enqueue_intents", quote(&jobs)),
        ] {
            require_denied(&probe, &forbidden).await?;
        }
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn the_full_schema_snapshot_is_never_a_prerequisite_of_intent_submission() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs = fixture.names.jobs.clone();
        let snapshot_role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs, &[RunledgerOperation::SchemaSnapshot])
            .compile()?;
        let intent_role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs, &[RunledgerOperation::IntentSubmission])
            .compile()?;
        let snapshot_login = fixture.login("snapshot", &snapshot_role).await?;
        let intent_login = fixture.login("intent_only", &intent_role).await?;
        require_within_policy(
            &fixture.pool(&snapshot_login, &[&jobs]).await?,
            &snapshot_role,
        )
        .await?;
        require_within_policy(&fixture.pool(&intent_login, &[&jobs]).await?, &intent_role).await?;

        let selected = schema::runledger_database(fixture, &snapshot_login, &jobs, 1)?;
        let observed = batter::runledger::verify_schema(&selected).await?;
        require(
            observed.schema() == jobs,
            "the snapshot reported another schema",
        )?;
        fixture.track(selected.pool().clone());

        let unselected = schema::runledger_database(fixture, &intent_login, &jobs, 1)?;
        let denied = batter::runledger::verify_schema(&unselected).await;
        require(
            denied.is_err(),
            "intent submission alone reached the privileged full-schema snapshot",
        )?;
        fixture.track(unselected.pool().clone());
        Ok(())
    })
    .await
}
