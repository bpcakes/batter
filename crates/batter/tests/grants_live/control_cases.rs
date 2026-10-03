//! Controls that detect under-grant, over-grant and shadowed objects.

use super::policy::{APPLICATION_RELATION, APPLICATION_ROUTINE, Composition, PublicPolicy};
use super::schema;
use super::support::{
    Fixture, Result, exec, quote, require, require_denied, require_permitted, require_violation,
    require_within_policy,
};
use batter::runledger::grants::RunledgerOperation;
use batter::runlimit::grants::RunlimitOperation;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
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

        let insert = format!(
            "INSERT INTO {}.job_enqueue_intents \
             (job_type, payload, idempotency_key, stage, enqueue_request_version, enqueue_request) \
             VALUES ('batter.grants.control', '{{}}'::jsonb, 'control-key', 'queued', 1, '{{}}'::jsonb)",
            quote(&jobs)
        );
        require_permitted(&probe, &insert).await?;

        // Revoking one required column privilege must fail both the native
        // operation and verification of the same compiled role.
        exec(
            &mut fixture.admin,
            format!(
                "REVOKE INSERT (enqueue_request) ON {}.job_enqueue_intents FROM {}",
                quote(&jobs),
                quote(&login)
            ),
        )
        .await?;
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

        // Revoking the row-lock UPDATE breaks duplicate resolution's FOR KEY
        // SHARE read, which PostgreSQL refuses without UPDATE on some column.
        let locking_read = format!(
            "SELECT id FROM {}.job_enqueue_intents WHERE idempotency_key = 'control-key' \
             LIMIT 1 FOR KEY SHARE",
            quote(&jobs)
        );
        require_permitted(&probe, &locking_read).await?;
        exec(
            &mut fixture.admin,
            format!(
                "REVOKE UPDATE (id) ON {}.job_enqueue_intents FROM {}",
                quote(&jobs),
                quote(&login)
            ),
        )
        .await?;
        require_denied(&probe, &locking_read).await?;
        require_violation(&probe, &role).await?;
        Ok(())
    })
    .await
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
async fn one_login_composes_both_adapters_with_its_own_application_objects() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs = fixture.names.jobs.clone();
        let quotas = fixture.names.quotas.clone();
        let application = fixture.names.application.clone();

        // Consumer shape A: narrow intent submission, a separately selected full
        // schema snapshot, fixed-window quotas, attempts, and a policy that
        // explicitly permits PUBLIC delivery of SELECT and routine EXECUTE.
        schema::permit_public_application_delivery(fixture).await?;
        let shape_a = Composition::new(PublicPolicy::PermitSelectedDelivery)
            .with_jobs(
                &jobs,
                &[
                    RunledgerOperation::IntentSubmission,
                    RunledgerOperation::SchemaSnapshot,
                ],
            )
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
        let login_a = fixture.login("shape_a", &shape_a).await?;
        let pool_a = fixture
            .pool(&login_a, &[&jobs, &quotas, &application])
            .await?;
        require_within_policy(&pool_a, &shape_a).await?;

        // The application's own relation and routine are reachable through the
        // same compiled role that carries the native requirements.
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
        require_denied(
            &pool_a,
            &format!(
                "DELETE FROM {}.{}",
                quote(&application),
                quote(APPLICATION_RELATION)
            ),
        )
        .await?;
        require_denied(
            &pool_a,
            &format!("SELECT 1 FROM {}.runlimit_gcra", quote(&quotas)),
        )
        .await?;

        // Consumer shape B: intent submission, direct workers with promotion,
        // GCRA quotas, attempts, and a policy that denies every PUBLIC delivery.
        let shape_b = Composition::new(PublicPolicy::Deny)
            .with_jobs(
                &jobs,
                &[
                    RunledgerOperation::DirectJobExecution,
                    RunledgerOperation::IntentPromotion,
                    RunledgerOperation::IntentSubmission,
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
        let login_b = fixture.login("shape_b", &shape_b).await?;
        let pool_b = fixture.pool(&login_b, &[&jobs, &quotas]).await?;
        require_within_policy(&pool_b, &shape_b).await?;
        require_denied(
            &pool_b,
            &format!("SELECT 1 FROM {}.runlimit_fixed_windows", quote(&quotas)),
        )
        .await?;
        require_denied(
            &pool_b,
            &format!("SELECT 1 FROM {}.{}", quote(&jobs), "_sqlx_migrations"),
        )
        .await?;
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
        Ok(())
    })
    .await
}

async fn current_database(pool: &sqlx::PgPool) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT pg_catalog.current_database()")
        .fetch_one(pool)
        .await?)
}
