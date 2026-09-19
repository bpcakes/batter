//! Read-only PostgreSQL schema and runtime-authority verification.
//!
//! Verification acquires and owns a [`crate::PgLease`] from the serving pool
//! inside the supplied operation boundary. It acknowledges transaction rollback
//! before reuse and retires the lease on failure or interruption. It does not
//! run migrations, repair ACLs, or acknowledge server-session termination.

mod authority;
mod executor;
mod manifest;
mod migration;
mod policy;
mod report;
mod request;
mod schema_inspection;
mod sqlx_migration;
mod work;

pub use manifest::{
    ColumnGrantGroup, CompiledExactRole, DatabaseGrantSpec, DeclarationPurpose, ExactRoleManifest,
    GrantPlan, GrantRenderError, ManifestError, PublicDelivery, RelationGrantGroup,
    RoutineGrantSpec, SchemaGrantSpec,
};
pub use policy::{
    AdditionalMigrations, AllowedPrivilege, AuthorityPolicy, AuthorityPolicyBuilder, ColumnPolicy,
    DatabasePolicy, DiscoveryDefaults, DiscoveryScope, Identifier, MigrationExpectation,
    MigrationPolicy, ObjectDefaults, ObjectPrivilege, ParameterName, ParameterPolicy, PolicyError,
    PublicAllowance, PublicGrant, PublicObject, QualifiedName, RelationPolicy, RequiredPrivilege,
    RequiredSurface, RolePolicy, RoutinePolicy, RoutineSignature, RoutineType, SchemaPolicy,
    SequencePolicy, TypePolicy,
};
pub use report::{
    Finding, FindingKind, RoleAttribute, SupportedSurface, UnsupportedSurface, VerificationError,
    VerificationReport, VerificationStatus,
};
pub use request::{
    PlanError, SchemaInspectionPolicy, SqlxLedgerMode, SqlxMigrationManifest, VerificationPlan,
};

use batter_core::operation::{OperationContext, OperationError};
use sqlx::{PgPool, Postgres, Transaction};

#[derive(Default)]
struct InspectionFragment {
    findings: Vec<Finding>,
    supported: Vec<SupportedSurface>,
    unsupported: Vec<UnsupportedSurface>,
    evaluated_items: usize,
}

/// Verify the serving pool under one total deadline and cancellation lineage.
///
/// This function acquires its own lease; application transactions on other
/// checkouts are never borrowed or reset. Residual raw transaction state on the
/// newly acquired checkout is rolled back before verification begins. An idle
/// checkout produces PostgreSQL's expected "there is no transaction in progress"
/// warning on `sqlx::postgres::notice`; the reset also handles raw transactions
/// invisible to SQLx's public managed-depth check. Session
/// settings and the effective serving identity are preserved. During inspection,
/// a transaction-local trusted search path prevents application overloads from
/// redirecting catalog helpers; rollback restores the serving search path.
///
/// Explicit temporary namespace selections return Incomplete with
/// [`UnsupportedSurface::TemporaryNamespaces`] before ledger locking or ACL
/// evaluation. This includes aliases, object names and routine argument types;
/// session-specific namespace authority is outside the supported model.
///
/// The verifier locks the migration ledger before taking its repeatable-read,
/// read-only snapshot. Ledgers participating in inheritance return Incomplete
/// with [`UnsupportedSurface::InheritedMigrationLedgers`]. Supported standalone
/// ledgers are read with ONLY, excluding attachments made after snapshot capture.
/// Ordinary migration-row writes remain possible. The
/// initially authenticated login roots the excess-authority audit; the current
/// identity determines required privileges and query visibility. Local identity
/// changes and row-security settings are rolled back before normal completion.
///
/// Captured evaluation cooperates every 64 object/role visits and rejects more
/// than one million visits or its report-capacity limit with
/// [`VerificationError::EvaluationCapacity`]. This bounds library evaluation
/// work, not OS scheduling, SQLx polling, allocation, or destruction latency.
///
/// A complete report, including a policy violation, permits reuse only after
/// acknowledged rollback. Native failure, timeout, cancellation or panic retires
/// the acquired client lease. This does not acknowledge remote cancellation or
/// guarantee cleanup after runtime/process death. A snapshot is historical and
/// does not certify future sessions, grants or schema changes.
///
/// ```no_run
/// use batter_core::operation::OperationContext;
/// use batter_sqlx::verification::{verify, VerificationPlan, VerificationStatus};
/// # async fn check(pool: &sqlx::PgPool, context: &OperationContext,
/// # plan: VerificationPlan<'_>) -> Result<(), Box<dyn std::error::Error>> {
/// let report = verify(pool, context, plan).await?;
/// if report.status() != VerificationStatus::WithinDeclaredPolicy {
///     return Err("database policy verification did not pass".into());
/// }
/// # Ok(()) }
/// ```
///
/// # Errors
/// Returns typed interruption or a redacted [`VerificationError`]. Native causes
/// remain available internally; inspection and rollback failures are both
/// retained. No connection acquisition occurs for an unpolled or already
/// interrupted operation. Unsupported requested surfaces cannot pass.
pub async fn verify(
    pool: &PgPool,
    context: &OperationContext,
    plan: VerificationPlan<'_>,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    executor::execute(pool, context, plan).await
}

/// Verify only migration history, using the same owned operation as [`verify`].
///
/// Use this for a separate setup/schema stage that also runs with migration
/// credentials. The report covers only [`SupportedSurface::MigrationLedger`];
/// it makes no authority claim. Inherited ledgers are explicitly unsupported.
/// Ledger locking, rollback and lease disposition
/// follow [`verify`]. Application-specific ledger shape remains a separate check.
///
/// ```no_run
/// # async fn check(pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext,
/// # policy: &batter_sqlx::verification::MigrationPolicy)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// let report = batter_sqlx::verification::verify_migrations(pool, context, policy).await?;
/// assert!(report.supported().contains(
///     &batter_sqlx::verification::SupportedSurface::MigrationLedger));
/// # Ok(()) }
/// ```
///
/// An inherited ledger is handled as an unsupported request:
/// ```no_run
/// # async fn check(pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext,
/// # policy: &batter_sqlx::verification::MigrationPolicy)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::verification::{verify_migrations, UnsupportedSurface, VerificationStatus};
/// let report = verify_migrations(pool, context, policy).await?;
/// if report.unsupported().contains(&UnsupportedSurface::InheritedMigrationLedgers) {
///     assert_eq!(report.status(), VerificationStatus::Incomplete);
/// }
/// # Ok(()) }
/// ```
///
/// Temporary namespace selection is explicitly unsupported:
/// ```no_run
/// # async fn check(pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::verification::{AuthorityPolicyBuilder, DiscoveryScope, Identifier,
///     UnsupportedSurface, VerificationStatus, verify_authority};
/// let policy = AuthorityPolicyBuilder {
///     discovery: DiscoveryScope::Schemas(vec![Identifier::new("pg_temp")?]),
///     ..AuthorityPolicyBuilder::default()
/// }.build()?;
/// let report = verify_authority(pool, context, &policy).await?;
/// assert_eq!(report.status(), VerificationStatus::Incomplete);
/// assert!(report.unsupported().contains(&UnsupportedSurface::TemporaryNamespaces));
/// # Ok(()) }
/// ```
///
/// # Errors
/// Returns the typed database, rollback and interruption errors
/// described by [`verify`].
pub async fn verify_migrations(
    pool: &PgPool,
    context: &OperationContext,
    policy: &MigrationPolicy,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    verify(pool, context, VerificationPlan::migrations(policy)).await
}

/// Verify only serving authority, using the same owned operation as [`verify`].
///
/// This entrypoint does not lock or read a migration ledger. Use it after a
/// separate schema stage when missing required ledger privileges must remain
/// structured [`FindingKind::MissingPrivilege`] findings. The report makes no
/// migration-history claim. Required privileges use the current identity;
/// excess authority remains rooted at the authenticated login.
///
/// ```no_run
/// # async fn check(pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext,
/// # policy: &batter_sqlx::verification::AuthorityPolicy)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// let report = batter_sqlx::verification::verify_authority(pool, context, policy).await?;
/// assert!(!report.supported().contains(
///     &batter_sqlx::verification::SupportedSurface::MigrationLedger));
/// # Ok(()) }
/// ```
///
/// Temporary namespace selection is explicitly unsupported:
/// ```no_run
/// # async fn check(pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::verification::{AuthorityPolicyBuilder, DiscoveryScope, Identifier,
///     UnsupportedSurface, VerificationStatus, verify_authority};
/// let policy = AuthorityPolicyBuilder {
///     discovery: DiscoveryScope::Schemas(vec![Identifier::new("pg_temp")?]),
///     ..AuthorityPolicyBuilder::default()
/// }.build()?;
/// let report = verify_authority(pool, context, &policy).await?;
/// assert_eq!(report.status(), VerificationStatus::Incomplete);
/// assert!(report.unsupported().contains(&UnsupportedSurface::TemporaryNamespaces));
/// # Ok(()) }
/// ```
///
/// # Errors
/// Returns the typed database, rollback and interruption errors
/// described by [`verify`].
pub async fn verify_authority(
    pool: &PgPool,
    context: &OperationContext,
    policy: &AuthorityPolicy,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    verify(pool, context, VerificationPlan::authority(policy)).await
}

/// Verify one SQLx 0.9 ledger's exact shape and selected history through the
/// protected verifier executor.
///
/// The ledger is locked before the repeatable-read snapshot when it exists.
/// [`SqlxLedgerMode::InstalledSubset`] permits absence only when the captured
/// snapshot also proves absence; a relation visible in that snapshot after the
/// lock attempt makes the report incomplete. A later commit is outside that
/// historical snapshot. Verification never runs migrations.
///
/// # Errors
/// Returns the same typed database, rollback, and interruption errors
/// as [`verify`].
pub async fn verify_sqlx_migrations(
    pool: &PgPool,
    context: &OperationContext,
    manifest: &SqlxMigrationManifest,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    verify(pool, context, VerificationPlan::sqlx_migrations(manifest)).await
}

/// Verify a compiled exact role, including its selected ownership safeguard.
///
/// Use this entrypoint or [`verify`] with [`VerificationPlan::exact_role`] so
/// the compiled role's ownership safeguards remain part of execution.
///
/// # Errors
/// Returns the same typed database, rollback, and interruption errors
/// as [`verify`].
pub async fn verify_exact_role(
    pool: &PgPool,
    context: &OperationContext,
    role: &CompiledExactRole,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    verify(pool, context, VerificationPlan::exact_role(role)).await
}

pub(crate) type PgTransaction<'c> = Transaction<'c, Postgres>;
