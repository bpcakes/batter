use super::*;
use sqlx::Connection;

impl VerificationPlan<'_> {
    fn requests_temporary_namespace(self) -> bool {
        self.authority_policy()
            .is_some_and(AuthorityPolicy::requests_temporary_namespace)
            || self
                .migration_policy()
                .is_some_and(|policy| policy::coverage::temporary_namespace(policy.ledger.schema()))
            || self.sqlx_migration().is_some_and(|manifest| {
                policy::coverage::temporary_namespace(manifest.ledger().schema())
            })
            || self
                .schema()
                .is_some_and(SchemaInspectionPolicy::requests_temporary_namespace)
    }
}

pub(super) fn execute<'a>(
    pool: &'a PgPool,
    context: &'a OperationContext,
    plan: VerificationPlan<'a>,
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
                inspect_owned_checkout(pool, scope, plan).await
            })
            .await;
        match outcome {
            Ok(report) => Ok(report),
            Err(OperationError::Failed(error)) => Err(error),
            Err(OperationError::Interrupted(reason)) => Err(OperationError::Interrupted(reason)),
        }
    })
}

fn inspect_owned_checkout<'a>(
    pool: &'a PgPool,
    scope: OperationContext,
    plan: VerificationPlan<'a>,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<VerificationReport, OperationError<VerificationError>>,
            > + Send
            + 'a,
    >,
> {
    // Keep the lease-owned query graph behind its own allocation boundary. If
    // this is inlined into `execute`, the unit-test harness's additional
    // monomorphizations push rustc's async layout query past the default depth.
    Box::pin(async move {
        let lease = crate::PgLease::acquire(pool, &scope)
            .await
            .map_err(|error| match error {
                OperationError::Failed(error) => {
                    OperationError::Failed(VerificationError::Native(error))
                }
                OperationError::Interrupted(reason) => OperationError::Interrupted(reason),
            })?;
        // Ok plus successful idle-state cleanup returns the checkout;
        // Err, interruption, or cleanup failure retires it.
        lease
            .with_connection(async |connection| inspect_checkout(connection, plan).await)
            .await
            .map_err(OperationError::Failed)
    })
}

async fn inspect_checkout(
    session: &mut crate::PgSession<'_>,
    plan: VerificationPlan<'_>,
) -> Result<VerificationReport, VerificationError> {
    // Verification is adapter-owned and needs native connection state and a
    // custom BEGIN mode. This private access is not part of the public lease
    // closure capability.
    let connection = session.native_connection();
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
    let inspection = inspect_transaction(&mut transaction, plan).await;
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
    plan: VerificationPlan<'_>,
) -> Result<VerificationReport, VerificationError> {
    sqlx::query("SET LOCAL search_path = pg_catalog, pg_temp")
        .execute(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    if plan.requests_temporary_namespace() {
        let (session_user, current_user) = authority::inspect_identities(transaction).await?;
        return Ok(VerificationReport::incomplete(
            vec![UnsupportedSurface::TemporaryNamespaces],
            session_user,
            current_user,
        ));
    }
    let legacy_ledger = if let Some(policy) = plan.migration_policy() {
        Some((
            policy,
            migration::lock_before_snapshot(transaction, &policy.ledger).await?,
        ))
    } else {
        None
    };
    let sqlx_ledger = if let Some(manifest) = plan.sqlx_migration() {
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
    inspect_plan(transaction, plan, legacy_ledger, sqlx_ledger).await
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

async fn inspect_plan(
    transaction: &mut PgTransaction<'_>,
    plan: VerificationPlan<'_>,
    legacy_ledger: Option<(&MigrationPolicy, migration::LedgerLock)>,
    sqlx_ledger: Option<(&SqlxMigrationManifest, migration::LedgerLock)>,
) -> Result<VerificationReport, VerificationError> {
    let exact_role = plan.selected_exact_role();
    let mut evaluation = authority::Evaluation::new();
    let authority_policy = plan.authority_policy();
    let (mut report, required): (VerificationReport, &[RequiredSurface]) = if let Some(policy) =
        authority_policy
    {
        (
            authority::inspect_with_evaluation(transaction, policy, legacy_ledger, &mut evaluation)
                .await?,
            policy.required_surfaces.as_slice(),
        )
    } else if let Some((policy, lock)) = legacy_ledger {
        (
            authority::inspect_migrations(transaction, policy, lock).await?,
            &[],
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
    if let Some(policy) = plan.schema() {
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
    Ok(VerificationReport::new(
        report.findings,
        report.supported,
        report.unsupported,
        report.session_user,
        report.current_user,
        required,
    ))
}
