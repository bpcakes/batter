//! Exercise the database boundary, including lock ordering, through public APIs.

use super::{Fixture, Role, pid};
use crate::support::{Result, bounded, require};
use batter_sqlx::managed::{
    ManagedObject, ManagedObjectKind, ManagedObjectManifest, PublicExecute,
};
use batter_sqlx::verification::{
    FindingKind, Identifier, MigrationExpectation, QualifiedName, RoutineSignature, SqlxLedgerMode,
    SqlxMigrationManifest, UnsupportedSurface, VerificationPlan, VerificationStatus,
};
use sqlx::{AssertSqlSafe, PgPool};
use std::time::Duration;
use tokio::time::{Instant, sleep};

fn changed_objects(fixture: &Fixture) -> Result<ManagedObjectManifest> {
    let schema = fixture.schema();
    let mut objects = fixture.standard_objects();
    objects.retain(|object| object.kind() != ManagedObjectKind::Trigger);
    objects.push(ManagedObject::function(
        RoutineSignature::new(schema, "release_number", [])?,
        format!("CREATE OR REPLACE FUNCTION {schema}.release_number() RETURNS int LANGUAGE sql RETURN 2;"),
    )?);
    objects.push(ManagedObject::trigger(
        QualifiedName::new(schema, "records")?,
        Identifier::new("records_touch")?,
        format!(
            "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE OF note
             ON {schema}.records FOR EACH ROW EXECUTE FUNCTION {schema}.touch_updated_at();"
        ),
    )?);
    Ok(fixture
        .objects_with(PublicExecute::Revoke, objects, [])?
        .manifest)
}

async fn wait_for_lock(
    observer: &PgPool,
    backend: i32,
    schema: &str,
    table: &str,
    mode: &str,
    granted: bool,
) -> Result {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed: bool = bounded(
            sqlx::query_scalar(
                "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_locks l
                JOIN pg_catalog.pg_class c ON c.oid = l.relation
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                WHERE l.pid = $1 AND n.nspname = $2 AND c.relname = $3
                  AND l.mode = $4 AND l.granted = $5)",
            )
            .bind(backend)
            .bind(schema)
            .bind(table)
            .bind(mode)
            .bind(granted)
            .fetch_one(observer),
        )
        .await??;
        if observed {
            return Ok(());
        }
        require(
            Instant::now() < deadline,
            "expected database lock was not observed",
        )?;
        sleep(Duration::from_millis(10)).await;
    }
}

/// A second migration ledger pauses the verifier after its managed-ledger lock
/// without blocking the applier's own SQLx migration phase.
async fn verification_gate(pool: &PgPool, schema: &str) -> Result<SqlxMigrationManifest> {
    bounded(
        sqlx::query(AssertSqlSafe(format!(
            "CREATE TABLE {schema}.verification_gate (LIKE {schema}._sqlx_migrations INCLUDING ALL)"
        )))
        .execute(pool),
    )
    .await??;
    Ok(SqlxMigrationManifest::new(
        QualifiedName::new(schema, "verification_gate")?,
        SqlxLedgerMode::Exact,
        Vec::<MigrationExpectation>::new(),
    )?)
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_verifier_waits_for_apply_before_snapshot() -> Result {
    Fixture::run(async |fixture| {
        let original = fixture.objects(PublicExecute::Revoke);
        let applying = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&applying, &original.manifest).await?;
        let changed = changed_objects(fixture)?;
        let reader = fixture.pool(Role::Reader).await?;
        let blocker = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let apply_pid = pid(&applying).await?;
        let verify_pid = pid(&reader).await?;
        let schema = fixture.schema();
        let mut gate = blocker.begin().await?;
        bounded(sqlx::query(AssertSqlSafe(format!(
            "LOCK TABLE {schema}.records IN SHARE MODE"
        ))).execute(&mut *gate)).await??;

        // Apply has changed its routines but cannot bind the changed trigger.
        // Its ledger lock must prevent the verifier from taking an old snapshot.
        let apply = async { Ok::<_, batter_core::BoxError>(fixture.apply(&applying, &changed).await?) };
        let inspect = async {
            wait_for_lock(&observer, apply_pid, schema, "records", "ShareRowExclusiveLock", false).await?;
            wait_for_lock(&observer, apply_pid, schema, "managed_objects", "AccessExclusiveLock", true).await?;
            let verification = fixture.verify(&reader, VerificationPlan::managed_objects(&changed));
            tokio::pin!(verification);
            tokio::select! {
                result = &mut verification => {
                    let _ = result?;
                    return Err(std::io::Error::other("verifier escaped an in-flight apply").into());
                }
                observed = wait_for_lock(&observer, verify_pid, schema, "managed_objects", "AccessShareLock", false) => observed?,
            }
            gate.commit().await?;
            let report = verification.await?;
            require(report.is_within_declared_policy(), "verifier used a snapshot from before apply committed")
        };
        let _ = tokio::try_join!(apply, inspect)?;
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_verifier_holds_apply_until_inspection_ends() -> Result {
    Fixture::run(async |fixture| {
        let original = fixture.objects(PublicExecute::Revoke);
        let applying = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&applying, &original.manifest).await?;
        let changed = changed_objects(fixture)?;
        let reader = fixture.pool(Role::Reader).await?;
        let blocker = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let apply_pid = pid(&applying).await?;
        let verify_pid = pid(&reader).await?;
        let schema = fixture.schema();
        let migration = verification_gate(&observer, schema).await?;
        let mut gate = blocker.begin().await?;
        bounded(sqlx::query(AssertSqlSafe(format!(
            "LOCK TABLE {schema}.verification_gate IN ACCESS EXCLUSIVE MODE"
        ))).execute(&mut *gate)).await??;
        let plan = VerificationPlan::sqlx_migrations(&migration).with_managed_objects(&original.manifest)?;
        let verification = fixture.verify(&reader, plan);
        tokio::pin!(verification);
        tokio::select! {
            result = &mut verification => {
                let _ = result?;
                return Err(std::io::Error::other("verifier missed the inspection gate").into());
            }
            observed = wait_for_lock(&observer, verify_pid, schema, "verification_gate", "AccessShareLock", false) => observed?,
        }
        wait_for_lock(&observer, verify_pid, schema, "managed_objects", "AccessShareLock", true).await?;

        let apply = fixture.apply(&applying, &changed);
        tokio::pin!(apply);
        tokio::select! {
            result = &mut apply => {
                let _ = result?;
                return Err(std::io::Error::other("apply escaped the verifier's managed-ledger lock").into());
            }
            observed = wait_for_lock(&observer, apply_pid, schema, "managed_objects", "AccessExclusiveLock", false) => observed?,
        }
        gate.commit().await?;
        let (report, _) = tokio::try_join!(verification, async {
            Ok::<_, batter_core::BoxError>(apply.await?)
        })?;
        require(report.is_within_declared_policy(), "apply changed the set during verification")?;
        require(
            fixture.verify(&reader, VerificationPlan::managed_objects(&changed)).await?.is_within_declared_policy(),
            "apply did not proceed after verification released its lock",
        )
    }).await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_verifier_rejects_ledger_created_after_lock_attempt() -> Result {
    Fixture::run(async |fixture| {
        let original = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&migrator, &original.manifest).await?;
        let late = fixture.objects_in_ledger(PublicExecute::Revoke, "late_objects")?;
        let reader = fixture.pool(Role::Reader).await?;
        let blocker = fixture.pool(Role::Migrator).await?;
        let verify_pid = pid(&reader).await?;
        let schema = fixture.schema();
        let migration = verification_gate(&migrator, schema).await?;
        let mut gate = blocker.begin().await?;
        bounded(sqlx::query(AssertSqlSafe(format!(
            "LOCK TABLE {schema}.verification_gate IN ACCESS EXCLUSIVE MODE"
        ))).execute(&mut *gate)).await??;
        let plan = VerificationPlan::sqlx_migrations(&migration).with_managed_objects(&late.manifest)?;
        let verification = fixture.verify(&reader, plan);
        tokio::pin!(verification);
        tokio::select! {
            result = &mut verification => {
                let _ = result?;
                return Err(std::io::Error::other("verifier missed the late-ledger gate").into());
            }
            observed = wait_for_lock(&migrator, verify_pid, schema, "verification_gate", "AccessShareLock", false) => observed?,
        }
        bounded(sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE TABLE {schema}.late_objects (LIKE {schema}.managed_objects INCLUDING ALL);
             INSERT INTO {schema}.late_objects SELECT * FROM {schema}.managed_objects"
        ))).execute(&migrator)).await??;
        gate.commit().await?;
        let report = verification.await?;
        require(
            report.status() == VerificationStatus::Incomplete
                && report.unsupported().contains(&UnsupportedSurface::UnprotectedManagedObjectLedger),
            "a late-created ledger was treated as protected",
        )?;
        require(
            fixture.verify(&reader, VerificationPlan::managed_objects(&late.manifest)).await?.is_within_declared_policy(),
            "the late ledger was not otherwise a valid managed installation",
        )
    }).await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_verifier_rejects_misshapen_row_secured_and_inherited_ledgers() -> Result {
    Fixture::run(async |fixture| {
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let schema = fixture.schema();
        let plan = || VerificationPlan::managed_objects(&objects.manifest);
        require(
            fixture
                .verify(&reader, plan())
                .await?
                .is_within_declared_policy(),
            "valid ledger failed control",
        )?;
        for (change, restore, finding) in [
            (
                "ADD COLUMN extra text",
                "DROP COLUMN extra",
                FindingKind::ManagedObjectLedgerShape,
            ),
            (
                "ENABLE ROW LEVEL SECURITY",
                "DISABLE ROW LEVEL SECURITY",
                FindingKind::LedgerRowSecurity,
            ),
        ] {
            bounded(
                sqlx::query(AssertSqlSafe(format!(
                    "ALTER TABLE {schema}.managed_objects {change}"
                )))
                .execute(&migrator),
            )
            .await??;
            let report = fixture.verify(&reader, plan()).await?;
            require(
                !report.is_within_declared_policy()
                    && report.findings().iter().any(|item| item.kind == finding),
                "unsupported managed ledger shape was not reported",
            )?;
            bounded(
                sqlx::query(AssertSqlSafe(format!(
                    "ALTER TABLE {schema}.managed_objects {restore}"
                )))
                .execute(&migrator),
            )
            .await??;
            require(
                fixture
                    .verify(&reader, plan())
                    .await?
                    .is_within_declared_policy(),
                "restored ledger failed control",
            )?;
        }
        bounded(
            sqlx::query(AssertSqlSafe(format!(
                "CREATE TABLE {schema}.ledger_child () INHERITS ({schema}.managed_objects)"
            )))
            .execute(&migrator),
        )
        .await??;
        let report = fixture.verify(&reader, plan()).await?;
        require(
            report.status() == VerificationStatus::Incomplete
                && report
                    .unsupported()
                    .contains(&UnsupportedSurface::UnprotectedManagedObjectLedger),
            "inherited managed ledger did not make verification incomplete",
        )?;
        bounded(
            sqlx::query(AssertSqlSafe(format!("DROP TABLE {schema}.ledger_child")))
                .execute(&migrator),
        )
        .await??;
        require(
            fixture
                .verify(&reader, plan())
                .await?
                .is_within_declared_policy(),
            "detached ledger failed control",
        )
    })
    .await
}
