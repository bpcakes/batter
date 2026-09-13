//! Read-only PostgreSQL schema and runtime-authority verification.
//!
//! Verification acquires and owns a [`crate::PgLease`] from the serving pool
//! inside the supplied operation boundary. It acknowledges transaction rollback
//! before reuse and retires the lease on failure or interruption. It does not
//! run migrations, repair ACLs, or acknowledge server-session termination.

mod authority;
mod manifest;
mod migration;
mod policy;
mod report;
mod work;

pub use manifest::{
    ColumnGrantGroup, CompiledExactRole, DatabaseGrantSpec, DeclarationPurpose, ExactRoleManifest,
    GrantPlan, GrantRenderError, ManifestError, PublicDelivery, RelationGrantGroup,
    RoutineGrantSpec, SchemaGrantSpec,
};
pub use policy::{
    AdditionalMigrations, AllowedPrivilege, AuthorityPolicy, ColumnPolicy, DatabasePolicy,
    DiscoveryDefaults, DiscoveryScope, Identifier, MigrationExpectation, MigrationPolicy,
    ObjectDefaults, ObjectPrivilege, ParameterName, ParameterPolicy, PolicyError, PublicAllowance,
    PublicGrant, PublicObject, QualifiedName, RelationPolicy, RequiredPrivilege, RequiredSurface,
    RolePolicy, RoutinePolicy, RoutineSignature, RoutineType, SchemaPolicy, SequencePolicy,
    TypePolicy, VerificationPolicy,
};
pub use report::{
    Finding, FindingKind, RoleAttribute, SupportedSurface, UnsupportedSurface, VerificationError,
    VerificationReport, VerificationStatus,
};

use batter::operation::{OperationContext, OperationError};
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};

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
/// use batter::operation::OperationContext;
/// use batter_sqlx::verification::{verify, VerificationPolicy, VerificationStatus};
/// # async fn check(pool: &sqlx::PgPool, context: &OperationContext,
/// # policy: &VerificationPolicy) -> Result<(), Box<dyn std::error::Error>> {
/// let report = verify(pool, context, policy).await?;
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
    policy: &VerificationPolicy,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    execute(pool, context, Inspection::Combined(policy)).await
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
/// # async fn check(pool: &sqlx::PgPool, context: &batter::operation::OperationContext,
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
/// # async fn check(pool: &sqlx::PgPool, context: &batter::operation::OperationContext,
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
/// # async fn check(pool: &sqlx::PgPool, context: &batter::operation::OperationContext)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::verification::{AuthorityPolicy, DiscoveryScope, Identifier,
///     UnsupportedSurface, VerificationStatus, verify_authority};
/// let policy = AuthorityPolicy {
///     discovery: DiscoveryScope::Schemas(vec![Identifier::new("pg_temp")?]),
///     ..AuthorityPolicy::default()
/// };
/// let report = verify_authority(pool, context, &policy).await?;
/// assert_eq!(report.status(), VerificationStatus::Incomplete);
/// assert!(report.unsupported().contains(&UnsupportedSurface::TemporaryNamespaces));
/// # Ok(()) }
/// ```
///
/// # Errors
/// Returns the typed policy, database, rollback and interruption errors
/// described by [`verify`].
pub async fn verify_migrations(
    pool: &PgPool,
    context: &OperationContext,
    policy: &MigrationPolicy,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    execute(pool, context, Inspection::Migrations(policy)).await
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
/// # async fn check(pool: &sqlx::PgPool, context: &batter::operation::OperationContext,
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
/// # async fn check(pool: &sqlx::PgPool, context: &batter::operation::OperationContext)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::verification::{AuthorityPolicy, DiscoveryScope, Identifier,
///     UnsupportedSurface, VerificationStatus, verify_authority};
/// let policy = AuthorityPolicy {
///     discovery: DiscoveryScope::Schemas(vec![Identifier::new("pg_temp")?]),
///     ..AuthorityPolicy::default()
/// };
/// let report = verify_authority(pool, context, &policy).await?;
/// assert_eq!(report.status(), VerificationStatus::Incomplete);
/// assert!(report.unsupported().contains(&UnsupportedSurface::TemporaryNamespaces));
/// # Ok(()) }
/// ```
///
/// # Errors
/// Returns the typed policy, database, rollback and interruption errors
/// described by [`verify`].
pub async fn verify_authority(
    pool: &PgPool,
    context: &OperationContext,
    policy: &AuthorityPolicy,
) -> Result<VerificationReport, OperationError<VerificationError>> {
    execute(pool, context, Inspection::Authority(policy)).await
}

#[derive(Clone, Copy)]
enum Inspection<'a> {
    Combined(&'a VerificationPolicy),
    Migrations(&'a MigrationPolicy),
    Authority(&'a AuthorityPolicy),
}

impl<'a> Inspection<'a> {
    fn validate(self) -> Result<(), PolicyError> {
        match self {
            Self::Combined(policy) => policy.validate(),
            Self::Migrations(policy) => policy.validate(),
            Self::Authority(policy) => policy.validate(),
        }
    }

    fn requests_temporary_namespace(self) -> bool {
        let authority = match self {
            Self::Combined(policy) => Some(&policy.authority),
            Self::Authority(policy) => Some(policy),
            Self::Migrations(_) => None,
        };
        authority.is_some_and(AuthorityPolicy::requests_temporary_namespace)
            || self
                .migration()
                .is_some_and(|policy| policy::coverage::temporary_namespace(policy.ledger.schema()))
    }

    fn migration(self) -> Option<&'a MigrationPolicy> {
        match self {
            Self::Combined(policy) => Some(&policy.migration),
            Self::Migrations(policy) => Some(policy),
            Self::Authority(_) => None,
        }
    }
}

fn execute<'a>(
    pool: &'a PgPool,
    context: &'a OperationContext,
    inspection: Inspection<'a>,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<VerificationReport, OperationError<VerificationError>>,
            > + Send
            + 'a,
    >,
> {
    // A single erased future keeps SQLx's nested acquisition/transaction types
    // out of downstream async layouts, including strict Clippy configurations.
    Box::pin(async move {
        let outcome = context
            .run("postgres.verification", |scope| async move {
                inspection.validate().map_err(|error| {
                    OperationError::Failed(VerificationError::InvalidPolicy(error))
                })?;
                let mut lease = crate::PgLease::acquire(pool, &scope)
                    .await
                    .map_err(|error| match error {
                        OperationError::Failed(error) => {
                            OperationError::Failed(VerificationError::Native(error))
                        }
                        OperationError::Interrupted(reason) => OperationError::Interrupted(reason),
                    })?;
                let report = inspect_checkout(lease.connection(), inspection)
                    .await
                    .map_err(OperationError::Failed)?;
                Ok((report, lease))
            })
            .await;
        match outcome {
            Ok((report, lease)) => {
                lease.return_to_pool();
                Ok(report)
            }
            Err(OperationError::Failed(error)) => Err(error),
            Err(OperationError::Interrupted(reason)) => Err(OperationError::Interrupted(reason)),
        }
    })
}

async fn inspect_checkout(
    connection: &mut PgConnection,
    inspection: Inspection<'_>,
) -> Result<VerificationReport, VerificationError> {
    // SQLx's depth does not detect raw BEGIN. It only rejects inconsistent
    // managed state; the acknowledged ROLLBACK below clears raw state on this
    // exclusively owned checkout, never on a borrowed caller transaction.
    if connection.is_in_transaction() {
        return Err(VerificationError::ConnectionState);
    }
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    let mut transaction = connection
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    let inspection = async {
        // Serving paths can contain user-defined overloads even when pg_catalog
        // appears first. Catalog inspection uses only trusted resolution; the
        // acknowledged rollback restores the application's path.
        sqlx::query("SET LOCAL search_path = pg_catalog, pg_temp")
            .execute(&mut *transaction)
            .await
            .map_err(|error| VerificationError::Native(error.into()))?;
        if inspection.requests_temporary_namespace() {
            let (session_user, current_user) =
                authority::inspect_identities(&mut transaction).await?;
            return Ok(VerificationReport::incomplete(
                vec![UnsupportedSurface::TemporaryNamespaces],
                session_user,
                current_user,
            ));
        }
        let ledger = if let Some(policy) = inspection.migration() {
            let lock = migration::lock_before_snapshot(&mut transaction, policy).await?;
            Some((policy, lock))
        } else {
            None
        };
        // This SELECT fixes the transaction snapshot after ledger locking and
        // before any metadata-dependent classification or planner expansion.
        let version: i32 =
            sqlx::query_scalar("SELECT current_setting('server_version_num')::integer")
                .fetch_one(&mut *transaction)
                .await
                .map_err(|error| VerificationError::Native(error.into()))?;
        if !authority::supports_postgres_18(version) {
            return Ok(VerificationReport::incomplete(
                vec![UnsupportedSurface::PostgresVersion],
                String::new(),
                String::new(),
            ));
        }
        if let Some((policy, migration::LedgerLock::Ready)) = &ledger
            && migration::has_inheritance(&mut transaction, policy).await?
        {
            let (session_user, current_user) =
                authority::inspect_identities(&mut transaction).await?;
            return Ok(VerificationReport::incomplete(
                vec![UnsupportedSurface::InheritedMigrationLedgers],
                session_user,
                current_user,
            ));
        }
        sqlx::query("SET LOCAL row_security = off")
            .execute(&mut *transaction)
            .await
            .map_err(|error| VerificationError::Native(error.into()))?;
        match inspection {
            Inspection::Combined(policy) => {
                authority::inspect(&mut transaction, &policy.authority, ledger).await
            }
            Inspection::Migrations(policy) => {
                let (_, lock) = ledger.expect("migration inspection acquired its ledger lock");
                authority::inspect_migrations(&mut transaction, policy, lock).await
            }
            Inspection::Authority(policy) => {
                authority::inspect(&mut transaction, policy, None).await
            }
        }
    }
    .await;
    let rollback = transaction.rollback().await;
    match (inspection, rollback) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(error)) => Err(VerificationError::Rollback {
            primary: None,
            source: error.into(),
        }),
        (Err(primary), Err(error)) => Err(VerificationError::Rollback {
            primary: Some(Box::new(primary)),
            source: error.into(),
        }),
    }
}

pub(crate) type PgTransaction<'c> = Transaction<'c, Postgres>;
