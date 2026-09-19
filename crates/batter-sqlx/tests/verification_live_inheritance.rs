use super::support::{Result, bounded, require};
use super::{AuthorityFixture, exec, quote};
use batter_core::operation::OperationContext;
use batter_sqlx::verification::*;
use sqlx::Connection;
use std::time::Duration;

async fn inspect(
    pool: &sqlx::PgPool,
    policy: &MigrationPolicy,
    combined: bool,
) -> Result<VerificationReport> {
    let context = OperationContext::new(Duration::from_secs(10))?;
    Ok(if combined {
        let authority = AuthorityPolicy::default();
        verify(
            pool,
            &context,
            VerificationPlan::migrations(policy).with_authority(&authority)?,
        )
        .await?
    } else {
        verify_migrations(pool, &context, policy).await?
    })
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_late_ledger_attachment_cannot_supply_snapshot_rows() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let ledger = format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a));
        let child = format!("{}.late_records", quote(&names.schema_a));
        exec(&mut fixture.admin, format!("CREATE TABLE {child} (LIKE {ledger})")).await?;
        exec(&mut fixture.admin, format!("DELETE FROM {ledger}")).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT ON {child} TO {}", quote(&names.login_a))).await?;
        let pool = fixture.login(&names.login_a).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {ledger}"))).execute(&pool).await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let catalog: i64 = sqlx::query_scalar("SELECT 'pg_catalog.pg_namespace'::regclass::oid::bigint")
            .fetch_one(&mut fixture.admin).await?;
        // Open and warm all connections before the catalog barrier. INHERIT
        // changes pg_inherits/pg_attribute/pg_class, not pg_namespace.
        let mut mutator = bounded(sqlx::PgConnection::connect(&fixture.url)).await??;
        let mut blocker = bounded(sqlx::PgConnection::connect(&fixture.url)).await??;
        exec(&mut mutator, "SET statement_timeout = '3s'".into()).await?;
        for combined in [false, true] {
            exec(&mut mutator, format!("INSERT INTO {child} VALUES (1, '\\x01', true)")).await?;
            // Resolve both relations in this connection before namespace locking.
            let before: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {ledger}")))
                .fetch_one(&mut mutator).await?;
            require(before == 0, "ledger was not empty before attachment")?;
            let mut barrier = blocker.begin().await?;
            sqlx::query("LOCK TABLE pg_catalog.pg_namespace IN ACCESS EXCLUSIVE MODE")
                .execute(&mut *barrier).await?;
            let policy = MigrationPolicy::new(
                QualifiedName::new(&names.schema_a, &names.ledger_a)?,
                [MigrationExpectation::new(1, [1])],
            )?;
            let verification = inspect(&pool, &policy, combined);
            tokio::pin!(verification);
            tokio::select! {
                _ = &mut verification => return Err(std::io::Error::other("verifier completed before namespace barrier").into()),
                result = super::verification_live_recovery::pending_lock(&mut fixture.admin, pid, catalog) => result?,
            }
            // The first explicit catalog query has acquired its transaction
            // snapshot, but cannot yet read namespace metadata.
            let mut change = mutator.begin().await?;
            exec(&mut *change, format!("DELETE FROM {child}")).await?;
            exec(&mut *change, format!("ALTER TABLE {child} INHERIT {ledger}")).await?;
            change.commit().await?;
            barrier.rollback().await?;
            let report = verification.await?;
            require(report.findings().iter().any(|f| f.kind == FindingKind::MissingMigration
                && f.subject.as_deref() == Some("1")),
                "late attachment supplied a migration absent from every committed ledger state")?;
            let after: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {ledger}")))
                .fetch_one(&mut mutator).await?;
            require(after == 0, "ledger was not empty after attachment")?;
            for selected in [&names.ledger_a, "late_records"] {
                let inherited = MigrationPolicy::new(
                    QualifiedName::new(&names.schema_a, selected)?,
                    policy.required().iter().cloned(),
                )?;
                let report = inspect(&pool, &inherited, combined).await?;
                require(report.status() == VerificationStatus::Incomplete
                    && report.unsupported() == [UnsupportedSurface::InheritedMigrationLedgers]
                    && report.supported().is_empty() && report.findings().is_empty(),
                    "inherited parent or child was treated as a supported ledger")?;
                require(report.session_user() == names.login_a && report.current_user() == names.login_a,
                    "unsupported ledger report lost serving identity")?;
            }
            let after_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
            require(after_pid == pid, "clean unsupported-ledger inspection retired the serving session")?;
            exec(&mut mutator, format!("ALTER TABLE {child} NO INHERIT {ledger}")).await?;
        }
        mutator.close().await?;
        blocker.close().await?;
        Ok(())
    })).await
}
