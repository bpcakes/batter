//! Application-owned schema: versioned migrations plus managed objects.
//!
//! `migrations/` keeps tables and data changes as forward-only history. The
//! routine and trigger in `objects/` are edited in place; each release applies
//! them after the migrations and verifies both ledgers before readiness.

use batter::{
    BoxError,
    operation::OperationContext,
    sqlx::{
        PgLease,
        managed::{AppliedManagedObjects, ManagedObject, ManagedObjectManifest, PublicExecute},
        verification::{
            Identifier, MigrationExpectation, QualifiedName, RoutineSignature, SqlxLedgerMode,
            SqlxMigrationManifest, VerificationPlan, VerificationReport, VerificationStatus,
            verify,
        },
    },
};
use sqlx::{PgPool, migrate::Migrator};
use std::fmt;

/// Every versioned migration this release ships.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The managed routine and the trigger that binds it.
pub fn managed_objects() -> Result<ManagedObjectManifest, BoxError> {
    Ok(ManagedObjectManifest::new(
        QualifiedName::new("lifecycle", "managed_objects")?,
        PublicExecute::Revoke,
        [
            ManagedObject::function(
                RoutineSignature::new("lifecycle", "touch_updated_at", [])?,
                include_str!("../objects/touch_updated_at.sql"),
            )?,
            ManagedObject::trigger(
                QualifiedName::new("lifecycle", "records")?,
                Identifier::new("records_touch_updated_at")?,
                include_str!("../objects/records_touch_updated_at.sql"),
            )?,
        ],
        [],
    )?)
}

/// The exact SQLx ledger history this release expects. SQLx creates its
/// ledger in the connection's first search-path schema, `public` by default.
pub fn migration_manifest() -> Result<SqlxMigrationManifest, BoxError> {
    Ok(SqlxMigrationManifest::new(
        QualifiedName::new("public", "_sqlx_migrations")?,
        SqlxLedgerMode::Exact,
        MIGRATOR
            .iter()
            .filter(|migration| !migration.migration_type.is_down_migration())
            .map(|migration| {
                MigrationExpectation::new(migration.version, migration.checksum.to_vec())
            }),
    )?)
}

/// Setup: run the migrations, then apply the managed objects, on one lease.
pub async fn apply(
    pool: &PgPool,
    context: &OperationContext,
    objects: &ManagedObjectManifest,
) -> Result<AppliedManagedObjects, BoxError> {
    let lease = PgLease::acquire(pool, context).await?;
    Ok(context
        .run("postgres.schema", |_| {
            lease.migrate_and_apply(&MIGRATOR, objects)
        })
        .await?)
}

/// Readiness gate: the SQLx history and every managed object must be exactly
/// this release's, with no out-of-band drift.
pub async fn verify_release(
    pool: &PgPool,
    context: &OperationContext,
    objects: &ManagedObjectManifest,
) -> Result<(), BoxError> {
    let migrations = migration_manifest()?;
    let plan = VerificationPlan::sqlx_migrations(&migrations).with_managed_objects(objects)?;
    let report = verify(pool, context, plan).await?;
    if report.status() == VerificationStatus::WithinDeclaredPolicy {
        Ok(())
    } else {
        Err(SchemaMismatch { report }.into())
    }
}

/// The database is not this release's schema. Findings stay in the retained
/// report for a trusted sink; Display and Debug name no object.
pub struct SchemaMismatch {
    /// The complete verification report.
    pub report: VerificationReport,
}

impl fmt::Display for SchemaMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("database schema does not match this release")
    }
}

impl fmt::Debug for SchemaMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaMismatch")
            .field("report", &self.report)
            .finish()
    }
}

impl std::error::Error for SchemaMismatch {}
