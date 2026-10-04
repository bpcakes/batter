//! Controls that detect under-grant, over-grant and shadowed objects.

use super::policy::{APPLICATION_RELATION, APPLICATION_ROUTINE, Composition, PublicPolicy};
use super::schema;
use super::support::{
    Fixture, PgAtomicFailure, Result, exec, quote, require, require_denied, require_permitted,
    require_violation, require_within_policy,
};
use batter::runledger::grants::RunledgerOperation;
use batter::runledger::native::core::jobs::JobType;
use batter::runledger::native::postgres::jobs::{
    self, JobDefinitionUpsert, JobEnqueue, JobEnqueueIntent, JobEnqueueIntentDisposition,
};
use batter::runledger::run_atomic;
use batter::runlimit::grants::RunlimitOperation;
use batter::runlimit::native::{
    Check, FixedWindowPolicy, GcraPolicy, KeyHasher, PolicyId, ScopeId,
};
use batter::runlimit::postgres::{PostgresGcraLimiter, PostgresLimiter};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
async fn removing_a_required_privilege_fails_the_native_operation_and_the_verifier() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs = fixture.names.jobs.clone();
        let role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs, &[RunledgerOperation::IntentSubmission])
            .compile()?;
        let login = fixture.login("drifting", &role).await?;
        let probe = fixture.pool(&login, &[&jobs]).await?;
        require_within_policy(&probe, &role).await?;

        // The controls drive the native API, not a copy of its SQL. A copied
        // statement would keep passing after a native query change that altered
        // the privileges the real entrypoint needs, which is the drift these
        // controls exist to catch. The raw statements below stay only as the
        // narrower evidence of which privilege PostgreSQL refused.
        let database = schema::runledger_database(fixture, &login, &jobs, 2)?;
        let payload = serde_json::json!({"control": true});
        require(
            record_intent(&database, &payload, "control-first").await.is_ok(),
            "the native intent recording was refused under its own selection",
        )?;

        // Revoking one required column privilege must fail both the native
        // operation and verification of the same compiled role.
        let revoke_insert = format!(
            "REVOKE INSERT (enqueue_request) ON {}.job_enqueue_intents FROM {}",
            quote(&jobs),
            quote(&login)
        );
        exec(&mut fixture.admin, revoke_insert).await?;
        let refused = record_intent(&database, &payload, "control-revoked").await;
        require(
            refused.is_err(),
            &format!("the native intent recording survived a revoked column: {refused:?}"),
        )?;
        let insert = format!(
            "INSERT INTO {}.job_enqueue_intents \
             (job_type, payload, idempotency_key, stage, enqueue_request_version, enqueue_request) \
             VALUES ('batter.grants.control', '{{}}'::jsonb, 'control-raw', 'queued', 1, '{{}}'::jsonb)",
            quote(&jobs)
        );
        require_denied(&probe, &insert).await?;
        require_violation(&probe, &role).await?;
        exec(
            &mut fixture.admin,
            format!(
                "GRANT INSERT (enqueue_request) ON {}.job_enqueue_intents TO {}",
                quote(&jobs),
                quote(&login)
            ),
        )
        .await?;
        require_within_policy(&probe, &role).await?;
        require_permitted(&probe, &insert).await?;

        // Revoking the row-lock UPDATE breaks duplicate resolution's FOR KEY
        // SHARE read, which PostgreSQL refuses without UPDATE on some column.
        // An exact duplicate is what reaches that read, so the first recording
        // has to succeed before the revocation.
        require(
            record_intent(&database, &payload, "control-duplicate")
                .await
                .is_ok(),
            "the native recording that sets up duplicate resolution was refused",
        )?;
        exec(
            &mut fixture.admin,
            format!(
                "REVOKE UPDATE (id) ON {}.job_enqueue_intents FROM {}",
                quote(&jobs),
                quote(&login)
            ),
        )
        .await?;
        let duplicate = record_intent(&database, &payload, "control-duplicate").await;
        require(
            duplicate.is_err(),
            &format!("native duplicate resolution survived a revoked row lock: {duplicate:?}"),
        )?;
        let locking_read = format!(
            "SELECT id FROM {}.job_enqueue_intents WHERE idempotency_key = 'control-duplicate' \
             LIMIT 1 FOR KEY SHARE",
            quote(&jobs)
        );
        require_denied(&probe, &locking_read).await?;
        require_violation(&probe, &role).await?;
        Ok(())
    })
    .await
}

/// Record one required intent through the native entrypoint under this login.
///
/// The error is flattened to a string because these controls only distinguish
/// acceptance from refusal; which privilege PostgreSQL named is established by
/// the raw statement beside each revocation.
async fn record_intent(
    database: &batter::runledger::RunledgerDatabase,
    payload: &serde_json::Value,
    key: &str,
) -> Result {
    run_atomic(database, async |mut scope| {
        let intent = JobEnqueueIntent::new(JobType::new("batter.grants.control"), payload, key);
        scope
            .record_required_job_enqueue_intent(&intent)
            .await
            .map(|_| ())
    })
    .await
    .map_err(PgAtomicFailure::into_error)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn excessive_table_update_grant_options_and_shadow_schemas_are_rejected() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs = fixture.names.jobs.clone();
        let shadow = fixture.names.shadow.clone();
        let role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs, &[RunledgerOperation::IntentSubmission])
            .compile()?;
        let login = fixture.login("excess", &role).await?;
        let probe = fixture.pool(&login, &[&jobs]).await?;
        require_within_policy(&probe, &role).await?;

        for (grant, revoke) in [
            (
                format!(
                    "GRANT UPDATE ON {}.job_enqueue_intents TO {}",
                    quote(&jobs),
                    quote(&login)
                ),
                format!(
                    "REVOKE UPDATE ON {}.job_enqueue_intents FROM {} \
                     ; GRANT UPDATE (id) ON {}.job_enqueue_intents TO {}",
                    quote(&jobs),
                    quote(&login),
                    quote(&jobs),
                    quote(&login)
                ),
            ),
            (
                format!(
                    "GRANT SELECT (id) ON {}.job_enqueue_intents TO {} WITH GRANT OPTION",
                    quote(&jobs),
                    quote(&login)
                ),
                format!(
                    "REVOKE GRANT OPTION FOR SELECT (id) ON {}.job_enqueue_intents FROM {}",
                    quote(&jobs),
                    quote(&login)
                ),
            ),
            (
                format!(
                    "GRANT USAGE ON SCHEMA {} TO {}",
                    quote(&shadow),
                    quote(&login)
                ),
                format!(
                    "REVOKE USAGE ON SCHEMA {} FROM {}",
                    quote(&shadow),
                    quote(&login)
                ),
            ),
        ] {
            exec(&mut fixture.admin, grant.clone()).await?;
            if grant.contains("SCHEMA") {
                // An out-of-scope schema is not audited, so verification still
                // passes; the selected schema's requirement is what matters.
                require_within_policy(&probe, &role).await?;
            } else {
                require_violation(&probe, &role).await?;
            }
            exec(&mut fixture.admin, revoke).await?;
            require_within_policy(&probe, &role).await?;
        }

        // Same-named relations and permissive grants in a shadow schema cannot
        // satisfy a requirement on the selected schema.
        exec(
            &mut fixture.admin,
            format!(
                "GRANT USAGE ON SCHEMA {} TO {}; \
                 GRANT ALL ON ALL TABLES IN SCHEMA {} TO {}",
                quote(&shadow),
                quote(&login),
                quote(&shadow),
                quote(&login)
            ),
        )
        .await?;
        exec(
            &mut fixture.admin,
            format!(
                "REVOKE INSERT (enqueue_request) ON {}.job_enqueue_intents FROM {}",
                quote(&jobs),
                quote(&login)
            ),
        )
        .await?;
        require_violation(&probe, &role).await?;
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
async fn one_login_composes_both_adapters_with_its_own_application_objects() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs = fixture.names.jobs.clone();
        let quotas = fixture.names.quotas.clone();
        let application = fixture.names.application.clone();
        let owner = fixture.names.owner.clone();
        let owner_database = schema::runledger_database(fixture, &owner, &jobs, 2)?;
        let payload = serde_json::json!({"composed": true});

        // Consumer shape A: narrow intent submission, fixed-window quotas,
        // attempts, its own application objects, and a policy that explicitly
        // permits PUBLIC delivery of SELECT and routine EXECUTE. The privileged
        // full-schema snapshot is a separate login, as in the audited shape.
        schema::permit_public_application_delivery(fixture).await?;
        let shape_a = Composition::new(PublicPolicy::PermitSelectedDelivery)
            .with_jobs(&jobs, &[RunledgerOperation::IntentSubmission])
            .with_quotas(
                &quotas,
                &[
                    RunlimitOperation::FixedWindowAdmission,
                    RunlimitOperation::FixedWindowExpiryCleanup,
                    RunlimitOperation::AuthenticationAttempts,
                ],
            )
            .with_application(&application)
            .compile()?;
        let inspector_role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs, &[RunledgerOperation::SchemaSnapshot])
            .compile()?;

        // Consumer shape B: intent submission and direct workers under the
        // native supervisor's default loops, which include the scheduler, with
        // GCRA quotas, attempts, and every PUBLIC delivery denied.
        let shape_b = Composition::new(PublicPolicy::Deny)
            .with_jobs(
                &jobs,
                &[
                    RunledgerOperation::DirectJobExecution,
                    RunledgerOperation::IntentPromotion,
                    RunledgerOperation::IntentSubmission,
                    RunledgerOperation::ScheduledDispatch,
                ],
            )
            .with_quotas(
                &quotas,
                &[
                    RunlimitOperation::GcraAdmission,
                    RunlimitOperation::GcraExpiryCleanup,
                    RunlimitOperation::AuthenticationAttempts,
                ],
            )
            .compile()?;

        let login_a = fixture.login("shape_a", &shape_a).await?;
        let inspector = fixture.login("shape_a_inspector", &inspector_role).await?;
        let login_b = fixture.login("shape_b", &shape_b).await?;
        let pool_a = fixture
            .pool(&login_a, &[&jobs, &quotas, &application])
            .await?;
        let pool_b = fixture.pool(&login_b, &[&jobs, &quotas]).await?;
        require_within_policy(&pool_a, &shape_a).await?;
        require_within_policy(&fixture.pool(&inspector, &[&jobs]).await?, &inspector_role).await?;
        require_within_policy(&pool_b, &shape_b).await?;

        // Each composed login executes the native work its own shape selected.
        let shape_a_database = schema::runledger_database(fixture, &login_a, &jobs, 2)?;
        let recorded = run_atomic(&shape_a_database, async |mut scope| {
            let intent = JobEnqueueIntent::new(
                JobType::new(COMPOSED_JOB_TYPE),
                &payload,
                "composed-shape-a",
            );
            scope.record_required_job_enqueue_intent(&intent).await
        })
        .await
        .map_err(PgAtomicFailure::into_error)?;
        require(
            recorded.disposition() == JobEnqueueIntentDisposition::Inserted,
            &format!("shape A did not record a new intent: {recorded:?}"),
        )?;
        require(
            PostgresLimiter::new(pool_a.clone())
                .check(&Check::new(
                    KeyHasher::new([23; 32])?.hash_for(&fixed_window_policy()?, "shape-a"),
                ))
                .await?
                .permits_request(),
            "shape A was denied its own fixed-window admission",
        )?;

        let inspection = schema::runledger_database(fixture, &inspector, &jobs, 1)?;
        let snapshot = batter::runledger::verify_schema(&inspection).await?;
        require(
            snapshot.schema() == jobs,
            "the inspector read another schema",
        )?;
        fixture.track(inspection.pool().clone());

        // Shape B claims and completes a direct job the owner enqueued, and
        // admits its own GCRA quota.
        let mut transaction = owner_database.pool().begin().await?;
        jobs::upsert_job_definition_tx(
            &mut transaction,
            &JobDefinitionUpsert {
                job_type: JobType::new(COMPOSED_JOB_TYPE),
                version: 1,
                max_attempts: 2,
                default_timeout_seconds: 60,
                default_priority: 100,
                is_enabled: true,
            },
        )
        .await?;
        transaction.commit().await?;
        let job = jobs::enqueue_job(
            owner_database.pool(),
            &JobEnqueue {
                job_type: JobType::new(COMPOSED_JOB_TYPE),
                organization_id: None,
                payload: &payload,
                priority: None,
                max_attempts: None,
                timeout_seconds: None,
                next_run_at: None,
                idempotency_key: None,
                stage: None,
            },
        )
        .await?;
        let shape_b_database = schema::runledger_database(fixture, &login_b, &jobs, 2)?;
        let claimed = jobs::claim_jobs(shape_b_database.pool(), "shape-b", 60, 10).await?;
        let claim = claimed
            .iter()
            .find(|record| record.id == job)
            .ok_or_else(|| super::support::fail("shape B could not claim the enqueued job"))?;
        jobs::complete_job_success(
            shape_b_database.pool(),
            claim.id,
            claim.run_number,
            claim.attempt,
            "shape-b",
            None,
        )
        .await?;
        require(
            PostgresGcraLimiter::new(pool_b.clone())
                .check(&Check::new(
                    KeyHasher::new([29; 32])?.hash_for(&gcra_policy()?, "shape-b"),
                ))
                .await?
                .permits_request(),
            "shape B was denied its own GCRA admission",
        )?;

        // The application's own relation and routine are reachable through the
        // same compiled role that carries shape A's native requirements.
        require_permitted(
            &pool_a,
            &format!(
                "INSERT INTO {}.{} (note) VALUES ({}.{}('Composed'))",
                quote(&application),
                quote(APPLICATION_RELATION),
                quote(&application),
                quote(APPLICATION_ROUTINE)
            ),
        )
        .await?;

        // Neither serving shape reaches the other's store, the privileged
        // snapshot, or authority its own selection excluded.
        for (pool, forbidden) in [
            (
                &pool_a,
                format!(
                    "DELETE FROM {}.{}",
                    quote(&application),
                    quote(APPLICATION_RELATION)
                ),
            ),
            (
                &pool_a,
                format!("SELECT 1 FROM {}.runlimit_gcra", quote(&quotas)),
            ),
            (
                &pool_a,
                format!("SELECT 1 FROM {}._sqlx_migrations", quote(&jobs)),
            ),
            (&pool_a, format!("SELECT 1 FROM {}.job_queue", quote(&jobs))),
            (
                &pool_b,
                format!("SELECT 1 FROM {}.runlimit_fixed_windows", quote(&quotas)),
            ),
            (
                &pool_b,
                format!("SELECT 1 FROM {}._sqlx_migrations", quote(&jobs)),
            ),
            (
                &pool_b,
                format!(
                    "SELECT 1 FROM {}.{}",
                    quote(&application),
                    quote(APPLICATION_RELATION)
                ),
            ),
        ] {
            require_denied(pool, &forbidden).await?;
        }
        require(
            !shape_a
                .grant_plan()
                .render(
                    &batter::sqlx::verification::Identifier::new(login_a.clone())?,
                    Some(&batter::sqlx::verification::Identifier::new(
                        current_database(&pool_a).await?,
                    )?),
                )?
                .contains("WITH GRANT OPTION"),
            "a rendered composition provisioned grant options",
        )?;
        fixture.track(owner_database.pool().clone());
        fixture.track(shape_a_database.pool().clone());
        fixture.track(shape_b_database.pool().clone());
        Ok(())
    })
    .await
}

/// The job type both composed shapes use.
const COMPOSED_JOB_TYPE: &str = "batter.grants.composed";

fn fixed_window_policy() -> Result<FixedWindowPolicy> {
    Ok(FixedWindowPolicy::new(
        PolicyId::new("batter.grants.composed.fixed")?,
        ScopeId::new("subject")?,
        4,
        Duration::from_secs(3_600),
    )?)
}

fn gcra_policy() -> Result<GcraPolicy> {
    Ok(GcraPolicy::new(
        PolicyId::new("batter.grants.composed.gcra")?,
        ScopeId::new("subject")?,
        4,
        Duration::from_secs(3_600),
        4,
    )?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn native_public_delivery_is_accepted_only_where_the_application_permits_it() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let quotas = fixture.names.quotas.clone();
        let selection = [RunlimitOperation::GcraAdmission];

        // Two roles over the same native objects, differing only in whether the
        // application permits PUBLIC to deliver SELECT.
        let strict = Composition::new(PublicPolicy::Deny)
            .with_quotas(&quotas, &selection)
            .compile()?;
        let permissive = Composition::new(PublicPolicy::PermitSelectedDelivery)
            .with_quotas(&quotas, &selection)
            .compile()?;
        let login = fixture.login("delivery", &strict).await?;
        let probe = fixture.pool(&login, &[&quotas]).await?;
        require_within_policy(&probe, &strict).await?;
        require_within_policy(&probe, &permissive).await?;

        // The permitted set is per privilege and per declared object. A
        // column-level PUBLIC grant on a column the fragment declares SELECT on
        // is accepted under the permitting policy and a violation under the
        // denying one. Rendering cannot distinguish the two policies, because a
        // PUBLIC allowance never becomes a grant.
        let relation = format!("{}.runlimit_gcra", quote(&quotas));
        exec(
            &mut fixture.admin,
            format!("GRANT SELECT (tat_scaled) ON {relation} TO PUBLIC"),
        )
        .await?;
        require_violation(&probe, &strict).await?;
        require_within_policy(&probe, &permissive).await?;
        exec(
            &mut fixture.admin,
            format!("REVOKE SELECT (tat_scaled) ON {relation} FROM PUBLIC"),
        )
        .await?;

        // A whole-relation PUBLIC grant reaches columns the fragment never
        // declared, so permitting PUBLIC delivery of SELECT does not accept it.
        exec(
            &mut fixture.admin,
            format!("GRANT SELECT ON {relation} TO PUBLIC"),
        )
        .await?;
        require_violation(&probe, &strict).await?;
        require_violation(&probe, &permissive).await?;
        exec(
            &mut fixture.admin,
            format!("REVOKE SELECT ON {relation} FROM PUBLIC"),
        )
        .await?;

        // A column outside the selection, and a privilege outside the permitted
        // set, both stay violations under either policy.
        for grant in [
            format!("GRANT SELECT (capacity_shard) ON {relation} TO PUBLIC"),
            format!("GRANT DELETE ON {relation} TO PUBLIC"),
        ] {
            let revoke = grant
                .replacen("GRANT", "REVOKE", 1)
                .replacen("TO", "FROM", 1);
            exec(&mut fixture.admin, grant).await?;
            require_violation(&probe, &strict).await?;
            require_violation(&probe, &permissive).await?;
            exec(&mut fixture.admin, revoke).await?;
        }
        require_within_policy(&probe, &strict).await?;
        require_within_policy(&probe, &permissive).await?;
        Ok(())
    })
    .await
}

async fn current_database(pool: &sqlx::PgPool) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT pg_catalog.current_database()")
        .fetch_one(pool)
        .await?)
}
