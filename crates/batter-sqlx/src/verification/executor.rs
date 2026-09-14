use super::*;
use sqlx::{Connection, PgConnection};

#[derive(Clone, Copy)]
pub(super) enum Inspection<'a> {
    Combined(&'a VerificationPolicy),
    Migrations(&'a MigrationPolicy),
    Authority(&'a AuthorityPolicy),
    SqlxMigrations(&'a SqlxMigrationManifest),
    ExactRole(&'a CompiledExactRole),
    Protected(VerificationRequest<'a>),
}

impl<'a> Inspection<'a> {
    fn validate(self) -> Result<(), PolicyError> {
        match self {
            Self::Combined(policy) => policy.validate(),
            Self::Migrations(policy) => policy.validate(),
            Self::Authority(policy) => policy.validate(),
            Self::SqlxMigrations(manifest) => manifest.validate(),
            Self::ExactRole(role) => role.authority_policy().validate(),
            Self::Protected(request) => request.validate(),
        }
    }

    fn requests_temporary_namespace(self) -> bool {
        let authority = match self {
            Self::Combined(policy) => Some(&policy.authority),
            Self::Authority(policy) => Some(policy),
            Self::Migrations(_) | Self::SqlxMigrations(_) => None,
            Self::ExactRole(role) => Some(role.authority_policy()),
            Self::Protected(request) => request.exact_role.map(CompiledExactRole::authority_policy),
        };
        authority.is_some_and(AuthorityPolicy::requests_temporary_namespace)
            || self
                .migration()
                .is_some_and(|policy| policy::coverage::temporary_namespace(policy.ledger.schema()))
            || self.sqlx_migration().is_some_and(|manifest| {
                policy::coverage::temporary_namespace(manifest.ledger().schema())
            })
            || self
                .schema()
                .is_some_and(SchemaInspectionPolicy::requests_temporary_namespace)
    }

    fn migration(self) -> Option<&'a MigrationPolicy> {
        match self {
            Self::Combined(policy) => Some(&policy.migration),
            Self::Migrations(policy) => Some(policy),
            _ => None,
        }
    }

    fn sqlx_migration(self) -> Option<&'a SqlxMigrationManifest> {
        match self {
            Self::SqlxMigrations(manifest) => Some(manifest),
            Self::Protected(request) => request.sqlx_migrations,
            _ => None,
        }
    }

    fn schema(self) -> Option<&'a SchemaInspectionPolicy> {
        match self {
            Self::Protected(request) => request.schema,
            _ => None,
        }
    }

    fn exact_role(self) -> Option<&'a CompiledExactRole> {
        match self {
            Self::ExactRole(role) => Some(role),
            Self::Protected(request) => request.exact_role,
            _ => None,
        }
    }

    fn is_protected(self) -> bool {
        matches!(
            self,
            Self::SqlxMigrations(_) | Self::ExactRole(_) | Self::Protected(_)
        )
    }
}

pub(super) fn execute<'a>(
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
    let inspection = inspect_transaction(&mut transaction, inspection).await;
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

async fn inspect_transaction(
    transaction: &mut PgTransaction<'_>,
    inspection: Inspection<'_>,
) -> Result<VerificationReport, VerificationError> {
    sqlx::query("SET LOCAL search_path = pg_catalog, pg_temp")
        .execute(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    if inspection.requests_temporary_namespace() {
        let (session_user, current_user) = authority::inspect_identities(transaction).await?;
        return Ok(VerificationReport::incomplete(
            vec![UnsupportedSurface::TemporaryNamespaces],
            session_user,
            current_user,
        ));
    }
    let legacy_ledger = if let Some(policy) = inspection.migration() {
        Some((
            policy,
            migration::lock_before_snapshot(transaction, &policy.ledger).await?,
        ))
    } else {
        None
    };
    let sqlx_ledger = if let Some(manifest) = inspection.sqlx_migration() {
        let lock = migration::lock_before_snapshot(transaction, manifest.ledger()).await?;
        Some((manifest, lock))
    } else {
        None
    };
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::integer")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    if !authority::supports_postgres_18(version) {
        return Ok(VerificationReport::incomplete(
            vec![UnsupportedSurface::PostgresVersion],
            String::new(),
            String::new(),
        ));
    }
    if has_unprotected_ledger(legacy_ledger, sqlx_ledger) {
        let (session_user, current_user) = authority::inspect_identities(transaction).await?;
        return Ok(VerificationReport::incomplete(
            vec![UnsupportedSurface::UnprotectedMigrationLedger],
            session_user,
            current_user,
        ));
    }
    if has_inherited_ledger(transaction, legacy_ledger, sqlx_ledger).await? {
        let (session_user, current_user) = authority::inspect_identities(transaction).await?;
        return Ok(VerificationReport::incomplete(
            vec![UnsupportedSurface::InheritedMigrationLedgers],
            session_user,
            current_user,
        ));
    }
    sqlx::query("SET LOCAL row_security = off")
        .execute(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    match inspection {
        Inspection::Combined(policy) => {
            authority::inspect(transaction, &policy.authority, legacy_ledger).await
        }
        Inspection::Migrations(policy) => {
            let (_, lock) = legacy_ledger.expect("migration inspection acquired its ledger lock");
            authority::inspect_migrations(transaction, policy, lock).await
        }
        Inspection::Authority(policy) => authority::inspect(transaction, policy, None).await,
        protected if protected.is_protected() => {
            inspect_protected(transaction, protected, sqlx_ledger).await
        }
        _ => unreachable!("all verification inspection variants are dispatched"),
    }
}

fn has_unprotected_ledger(
    legacy: Option<(&MigrationPolicy, migration::LedgerLock)>,
    sqlx: Option<(&SqlxMigrationManifest, migration::LedgerLock)>,
) -> bool {
    legacy.is_some_and(|(_, lock)| lock == migration::LedgerLock::Unprotected)
        || sqlx.is_some_and(|(_, lock)| lock == migration::LedgerLock::Unprotected)
}

async fn has_inherited_ledger(
    transaction: &mut PgTransaction<'_>,
    legacy: Option<(&MigrationPolicy, migration::LedgerLock)>,
    sqlx: Option<(&SqlxMigrationManifest, migration::LedgerLock)>,
) -> Result<bool, VerificationError> {
    if let Some((_, migration::LedgerLock::Ready(protected))) = legacy {
        migration::has_inheritance(transaction, protected).await
    } else if let Some((_, migration::LedgerLock::Ready(protected))) = sqlx {
        migration::has_inheritance(transaction, protected).await
    } else {
        Ok(false)
    }
}

async fn inspect_protected(
    transaction: &mut PgTransaction<'_>,
    inspection: Inspection<'_>,
    sqlx_ledger: Option<(&SqlxMigrationManifest, migration::LedgerLock)>,
) -> Result<VerificationReport, VerificationError> {
    let exact_role = inspection.exact_role();
    let mut evaluation = authority::Evaluation::new();
    let (mut report, required): (VerificationReport, &[RequiredSurface]) = if let Some(role) =
        exact_role
    {
        let policy = role.authority_policy();
        (
            authority::inspect_with_evaluation(transaction, policy, None, &mut evaluation).await?,
            policy.required_surfaces.as_slice(),
        )
    } else {
        let (session_user, current_user) = authority::inspect_identities(transaction).await?;
        (
            VerificationReport::new(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                session_user,
                current_user,
                &[],
            ),
            &[],
        )
    };
    let mut fragments = Vec::new();
    if let Some((manifest, lock)) = sqlx_ledger {
        fragments.push(sqlx_migration::inspect(transaction, manifest, lock).await?);
    }
    if let Some(policy) = inspection.schema() {
        fragments.push(schema_inspection::inspect(transaction, policy).await?);
    }
    if exact_role.is_some_and(CompiledExactRole::denies_current_database_ownership) {
        fragments.push(authority::inspect_ownership(transaction, report.session_user()).await?);
    }
    for mut fragment in fragments {
        let evaluated_items = fragment.evaluated_items;
        report.findings.append(&mut fragment.findings);
        report.supported.append(&mut fragment.supported);
        report.unsupported.append(&mut fragment.unsupported);
        evaluation
            .checkpoint_many(evaluated_items, &report.findings)
            .await?;
    }
    report.supported.sort_by_key(|surface| *surface as u8);
    report.supported.dedup();
    report.unsupported.sort_by_key(|surface| *surface as u8);
    report.unsupported.dedup();
    Ok(VerificationReport::new(
        report.findings,
        report.supported,
        report.unsupported,
        report.session_user,
        report.current_user,
        required,
    ))
}
