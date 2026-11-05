//! Release ordering is enforced under the ledger lock, including legacy adoption.

use super::{Fixture, Role, catalog_state, pid};
use crate::support::{Result, require};
use batter_sqlx::managed::{
    ManagedApplyError, ManagedApplyWithError, ManagedObject, ManagedObjectManifest,
    ManagedReleaseError, ManagedRevision, PublicExecute,
};
use batter_sqlx::verification::{FindingKind, QualifiedName, RoutineSignature, VerificationPlan};
use sqlx::{AssertSqlSafe, PgPool};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::Notify;

fn manifest(fixture: &Fixture, revision: u64, value: i32) -> Result<ManagedObjectManifest> {
    let schema = fixture.schema();
    Ok(ManagedObjectManifest::new(
        ManagedRevision::new(revision).expect("test revision"),
        QualifiedName::new(schema, "managed_objects")?,
        PublicExecute::Revoke,
        [ManagedObject::function(
            RoutineSignature::new(schema, "release_value", [])?,
            format!(
                "CREATE OR REPLACE FUNCTION {schema}.release_value() RETURNS int LANGUAGE sql RETURN {value}"
            ),
        )?],
        [],
    )?)
}

async fn ledger(pool: &PgPool, schema: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT row_to_json(m)::text FROM {schema}.managed_objects m ORDER BY identity"
    )))
    .fetch_all(pool)
    .await?)
}

async fn column_count(pool: &PgPool, schema: &str) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM information_schema.columns WHERE table_schema=$1 AND table_name='managed_objects'")
        .bind(schema).fetch_one(pool).await?)
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_release_refuses_downgrades_and_revision_conflicts() -> Result {
    Fixture::run(async |fixture| {
        let pool = fixture.pool(Role::Migrator).await?;
        let old = manifest(fixture, 1, 10)?;
        let current = manifest(fixture, 2, 20)?;
        let conflict = manifest(fixture, 2, 30)?;
        let _ = fixture.apply(&pool, &old).await?;
        let _ = fixture.apply(&pool, &current).await?;
        let before = (
            catalog_state(&pool, fixture.schema()).await?,
            ledger(&pool, fixture.schema()).await?,
        );
        let called = AtomicBool::new(false);
        let error = super::application_sql::apply(fixture, &pool, &old, async |_| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
        require!(
            matches!(error, ManagedApplyWithError::Apply(ManagedApplyError::Release(
            ManagedReleaseError::Downgrade { requested, installed }
        )) if requested == old.revision() && installed == current.revision()),
            "stale release was not refused"
        )?;
        require!(
            !called.load(Ordering::SeqCst),
            "stale release invoked application SQL"
        )?;
        require!(
            matches!(
                fixture.apply(&pool, &conflict).await,
                Err(ManagedApplyError::Release(
                    ManagedReleaseError::RevisionConflict { .. }
                ))
            ),
            "same revision accepted different source"
        )?;
        require!(
            before
                == (
                    catalog_state(&pool, fixture.schema()).await?,
                    ledger(&pool, fixture.schema()).await?
                ),
            "refusal mutated state"
        )?;
        let reapplied = fixture.apply(&pool, &current).await?;
        require!(
            reapplied.revision() == current.revision()
                && reapplied.created_or_replaced() == 0
                && reapplied.unchanged() == 1,
            "same release was not idempotent"
        )?;
        let report = fixture
            .verify(&pool, VerificationPlan::managed_objects(&old))
            .await?;
        require!(
            report
                .findings()
                .iter()
                .any(|f| f.kind == FindingKind::ManagedObjectRevisionMismatch),
            "verification missed a stale revision"
        )?;
        // A higher revision may carry identical content, including an explicit
        // rollback release later choosing an earlier function body.
        let next = manifest(fixture, 3, 20)?;
        require!(
            next.digest() == current.digest(),
            "revision changed the content digest"
        )?;
        require!(
            fixture.apply(&pool, &next).await?.created_or_replaced() == 0,
            "revision-only advance ran DDL"
        )?;
        let report = fixture
            .verify(&pool, VerificationPlan::managed_objects(&current))
            .await?;
        require!(
            report.findings().len() == 1
                && report.findings()[0].kind == FindingKind::ManagedObjectRevisionMismatch,
            "identical content hid a revision-only mismatch"
        )?;
        let rollback = manifest(fixture, 4, 10)?;
        let _ = fixture.apply(&pool, &rollback).await?;
        let value: i32 = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT {}.release_value()",
            fixture.schema()
        )))
        .fetch_one(&pool)
        .await?;
        require!(value == 10, "explicit newer rollback release failed")
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_release_waiting_older_writer_cannot_revert_newer_commit() -> Result {
    Fixture::run(async |fixture| {
        let first = fixture.pool(Role::Migrator).await?;
        let second = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let backend = pid(&second).await?;
        let _ = fixture.apply(&first, &manifest(fixture, 1, 10)?).await?;
        let newer = manifest(fixture, 3, 30)?;
        let older = manifest(fixture, 2, 20)?;
        let entered = Notify::new();
        let release = Notify::new();
        let applying = super::application_sql::apply(fixture, &first, &newer, async |_| {
            entered.notify_one();
            release.notified().await;
            Ok(())
        });
        let contender = async {
            entered.notified().await;
            let waiting = fixture.apply(&second, &older);
            let observe = async {
                let result = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')")
                            .bind(backend).fetch_one(&observer).await?;
                        if blocked { return Ok::<_, sqlx::Error>(()); }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await;
                release.notify_one();
                result??;
                Ok::<_, batter_core::BoxError>(())
            };
            let (outcome, observed) = tokio::join!(waiting, observe);
            observed?;
            require!(matches!(outcome, Err(ManagedApplyError::Release(ManagedReleaseError::Downgrade { .. }))), "waiting old writer reverted new release")
        };
        let (new_result, old_result) = tokio::join!(applying, contender);
        let _ = new_result?;
        old_result?;
        require!(fixture.apply(&observer, &newer).await?.unchanged() == 1, "newer release did not remain installed")
    }).await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_release_legacy_adoption_requires_matching_baseline_and_is_atomic() -> Result {
    Fixture::run(async |fixture| {
        let pool = fixture.pool(Role::Migrator).await?;
        let baseline = manifest(fixture, 7, 10)?;
        let next = manifest(fixture, 8, 20)?;
        let _ = fixture.apply(&pool, &baseline).await?;
        // Recreate the exact v1 ledger shape and retained content.
        fixture
            .admin(&format!(
                "ALTER TABLE {}.managed_objects DROP COLUMN release_revision",
                fixture.schema()
            ))
            .await?;
        require!(
            column_count(&pool, fixture.schema()).await? == 5,
            "not a legacy ledger"
        )?;
        let before = (
            catalog_state(&pool, fixture.schema()).await?,
            ledger(&pool, fixture.schema()).await?,
        );
        require!(
            matches!(
                fixture.apply(&pool, &next).await,
                Err(ManagedApplyError::Release(
                    ManagedReleaseError::LegacyBaselineRequired
                ))
            ),
            "changed content established an unversioned baseline"
        )?;
        let refused = super::application_sql::apply(fixture, &pool, &baseline, async |_| {
            Err(sqlx::Error::RowNotFound)
        })
        .await;
        require!(
            matches!(refused, Err(ManagedApplyWithError::Application(_))),
            "baseline callback refusal misclassified"
        )?;
        require!(
            column_count(&pool, fixture.schema()).await? == 5,
            "failed adoption changed the legacy shape"
        )?;
        require!(
            before
                == (
                    catalog_state(&pool, fixture.schema()).await?,
                    ledger(&pool, fixture.schema()).await?
                ),
            "failed adoption changed legacy content"
        )?;
        require!(
            fixture.apply(&pool, &baseline).await?.unchanged() == 1,
            "matching baseline ran object DDL"
        )?;
        require!(
            column_count(&pool, fixture.schema()).await? == 6,
            "successful adoption did not fence old shape readers"
        )?;
        let _ = fixture.apply(&pool, &next).await?;
        require!(
            matches!(
                fixture.apply(&pool, &baseline).await,
                Err(ManagedApplyError::Release(
                    ManagedReleaseError::Downgrade { .. }
                ))
            ),
            "adopted baseline reverted newer content"
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_release_missing_or_invalid_metadata_cannot_reset_the_fence() -> Result {
    Fixture::run(async |fixture| {
        let pool = fixture.pool(Role::Migrator).await?;
        let current = manifest(fixture, 2, 20)?;
        let old = manifest(fixture, 1, 10)?;
        let _ = fixture.apply(&pool, &current).await?;
        for revision in [0, -1] {
            fixture.admin(&format!("UPDATE {}.managed_objects SET release_revision={revision} WHERE identity='manifest'", fixture.schema())).await?;
            require!(matches!(fixture.apply(&pool, &old).await, Err(ManagedApplyError::Release(ManagedReleaseError::InvalidLedger))), "invalid revision reset the fence")?;
        }
        fixture.admin(&format!("DELETE FROM {}.managed_objects", fixture.schema())).await?;
        require!(matches!(fixture.apply(&pool, &old).await, Err(ManagedApplyError::Release(ManagedReleaseError::InvalidLedger))), "empty existing ledger was treated as a fresh install")
    }).await
}
