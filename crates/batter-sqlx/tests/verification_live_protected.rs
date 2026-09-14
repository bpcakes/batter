use super::support::{Result, require};
use super::{AuthorityFixture, Names, exec, quote};
use batter::operation::{OperationContext, OperationError};
use batter_sqlx::verification::{
    AllowedPrivilege, AuthorityPolicyBuilder, DatabaseGrantSpec, DatabasePolicy,
    DeclarationPurpose, DiscoveryScope, ExactRoleManifest, FindingKind, Identifier,
    MigrationExpectation, MigrationPolicy, ObjectPrivilege, PublicDelivery, PublicGrant,
    PublicObject, QualifiedName, RolePolicy, SchemaInspectionPolicy, SqlxLedgerMode,
    SqlxMigrationManifest, SupportedSurface, UnsupportedSurface, VerificationError,
    VerificationPlan, VerificationStatus, verify, verify_exact_role, verify_migrations,
    verify_sqlx_migrations,
};
use std::time::Duration;

mod concurrency;

pub(super) async fn create_sqlx_ledger(connection: &mut sqlx::PgConnection, table: &str) -> Result {
    exec(
        connection,
        format!(
            "CREATE TABLE {table} (
             version BIGINT PRIMARY KEY,
             description TEXT NOT NULL,
             installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
             success BOOLEAN NOT NULL,
             checksum BYTEA NOT NULL,
             execution_time BIGINT NOT NULL)"
        ),
    )
    .await
}

pub(super) fn context() -> Result<OperationContext> {
    Ok(OperationContext::new(Duration::from_secs(10))?)
}

async fn require_missing_select_is_native(
    admin: &mut sqlx::PgConnection,
    pool: &sqlx::PgPool,
    names: &Names,
    table: &str,
    manifest: &SqlxMigrationManifest,
) -> Result {
    exec(
        &mut *admin,
        format!("GRANT INSERT ON {table} TO {}", quote(&names.login_a)),
    )
    .await?;
    exec(
        &mut *admin,
        format!("REVOKE SELECT ON {table} FROM {}", quote(&names.login_a)),
    )
    .await?;
    let history = batter_sqlx::test_support::pause_next_verification_ledger_history();
    let context = context()?;
    let denied = verify_sqlx_migrations(pool, &context, manifest);
    tokio::pin!(denied);
    tokio::select! {
        () = history.wait_until_reached() => {}
        _ = &mut denied => return Err(std::io::Error::other(
            "missing SELECT failed before protected ledger history setup",
        ).into()),
    }
    history.release();
    let denied = denied.await;
    require(
        matches!(
            denied,
            Err(OperationError::Failed(VerificationError::Native(ref error)))
                if error.native().as_database_error()
                    .and_then(|database| database.code())
                    .as_deref() == Some("42501")
        ),
        "a locked ledger without SELECT was misreported as a replacement race",
    )?;
    exec(
        &mut *admin,
        format!("GRANT SELECT ON {table} TO {}", quote(&names.login_a)),
    )
    .await
}

async fn require_checksum_length_guard(
    admin: &mut sqlx::PgConnection,
    pool: &sqlx::PgPool,
    names: &Names,
    table_name: &str,
    table: &str,
) -> Result {
    let checksum_prefix = vec![7_u8; 1_024];
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {table} SET checksum = $1 WHERE version = 1"
    )))
    .bind(&checksum_prefix)
    .execute(&mut *admin)
    .await?;
    let bounded_checksum = SqlxMigrationManifest::new(
        QualifiedName::new(&names.schema_a, table_name)?,
        SqlxLedgerMode::Exact,
        [
            MigrationExpectation::new(1, checksum_prefix),
            MigrationExpectation::new(3, [3]),
        ],
    )?;
    require(
        verify_sqlx_migrations(pool, &context()?, &bounded_checksum)
            .await?
            .is_within_declared_policy(),
        "a checksum at the declared byte bound did not pass",
    )?;
    exec(
        admin,
        format!("UPDATE {table} SET checksum = checksum || '\\x00'::bytea WHERE version = 1"),
    )
    .await?;
    require(
        verify_sqlx_migrations(pool, &context()?, &bounded_checksum)
            .await?
            .findings()
            .iter()
            .any(|finding| finding.kind == FindingKind::MigrationChecksumMismatch),
        "a checksum longer than the retained prefix matched its prefix",
    )
}

async fn require_identifier_literal_guard(
    admin: &mut sqlx::PgConnection,
    pool: &sqlx::PgPool,
    names: &Names,
) -> Result {
    let unusual_name = "sqlx\\'history";
    let unusual_table = format!("{}.{}", quote(&names.schema_a), quote(unusual_name));
    create_sqlx_ledger(&mut *admin, &unusual_table).await?;
    exec(
        &mut *admin,
        format!(
            "INSERT INTO {unusual_table} VALUES
             (9, 'unusual', now(), true, '\\x09', 1)"
        ),
    )
    .await?;
    exec(
        &mut *admin,
        format!(
            "GRANT SELECT ON {unusual_table} TO {}",
            quote(&names.login_a)
        ),
    )
    .await?;
    sqlx::query("SET standard_conforming_strings = off")
        .execute(pool)
        .await?;
    let unusual = SqlxMigrationManifest::new(
        QualifiedName::new(&names.schema_a, unusual_name)?,
        SqlxLedgerMode::Exact,
        [MigrationExpectation::new(9, [9])],
    )?;
    require(
        verify_sqlx_migrations(pool, &context()?, &unusual)
            .await?
            .is_within_declared_policy(),
        "a quoted ledger name depended on standard_conforming_strings",
    )
}

async fn require_catalog_type_identity_guard(
    admin: &mut sqlx::PgConnection,
    pool: &sqlx::PgPool,
    names: &Names,
) -> Result {
    for type_name in ["int8", "bytea", "bool"] {
        exec(
            &mut *admin,
            format!(
                "CREATE TYPE {}.{} AS (value bigint)",
                quote(&names.schema_a),
                quote(type_name)
            ),
        )
        .await?;
    }
    let shadow_name = "shadow_type_history";
    let shadow_table = format!("{}.{}", quote(&names.schema_a), quote(shadow_name));
    exec(
        &mut *admin,
        format!(
            "CREATE TABLE {shadow_table} (
             version {}.{} NOT NULL,
             description TEXT NOT NULL,
             installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
             success {}.{} NOT NULL,
             checksum {}.{} NOT NULL,
             execution_time BIGINT NOT NULL)",
            quote(&names.schema_a),
            quote("int8"),
            quote(&names.schema_a),
            quote("bool"),
            quote(&names.schema_a),
            quote("bytea")
        ),
    )
    .await?;
    exec(
        &mut *admin,
        format!(
            "GRANT SELECT ON {shadow_table} TO {}",
            quote(&names.login_a)
        ),
    )
    .await?;
    let shadow = SqlxMigrationManifest::new(
        QualifiedName::new(&names.schema_a, shadow_name)?,
        SqlxLedgerMode::Exact,
        Vec::<MigrationExpectation>::new(),
    )?;
    require(
        verify_sqlx_migrations(pool, &context()?, &shadow)
            .await?
            .findings()
            .iter()
            .any(|finding| finding.kind == FindingKind::SqlxLedgerShape),
        "user-defined types with built-in names reached history decoding",
    )?;
    let legacy = MigrationPolicy::new(QualifiedName::new(&names.schema_a, shadow_name)?, [])?;
    let legacy_report = verify_migrations(pool, &context()?, &legacy).await?;
    require(
        legacy_report
            .findings()
            .iter()
            .any(|finding| finding.kind == FindingKind::LedgerColumnShape)
            && !legacy_report
                .unsupported()
                .contains(&UnsupportedSurface::UnprotectedMigrationLedger),
        "legacy verification misreported user-defined type identities as a ledger race",
    )
}

async fn install_legacy_holdable_history_cursor(pool: &sqlx::PgPool) -> Result {
    let mut connection = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *connection).await?;
    sqlx::query(
        "DECLARE batter_verification_ledger_history
         NO SCROLL CURSOR WITH HOLD FOR SELECT 1 AS value",
    )
    .persistent(false)
    .execute(&mut *connection)
    .await?;
    sqlx::query("COMMIT").execute(&mut *connection).await?;
    Ok(())
}

async fn require_combined_protected_request(
    pool: &sqlx::PgPool,
    names: &Names,
    policy: &SchemaInspectionPolicy,
) -> Result {
    let mut role =
        ExactRoleManifest::new(Identifier::new(&names.schema_a)?, DiscoveryScope::Declared)?;
    role.set_role_policy(RolePolicy {
        allowed_admin_roles: vec![Identifier::new(&names.admin_target)?],
        ..RolePolicy::default()
    })?;
    role.add_database(
        DatabaseGrantSpec::new(
            [ObjectPrivilege::Connect, ObjectPrivilege::Temporary],
            DeclarationPurpose::AllowedOnly,
        )?
        .public_delivery(PublicDelivery::AllowDeclared),
    )?;
    role.deny_current_database_ownership(true);
    let compiled = role.compile()?;
    let absent_subset = SqlxMigrationManifest::new(
        QualifiedName::new(&names.schema_a, "absent_combined_history")?,
        SqlxLedgerMode::InstalledSubset,
        Vec::<MigrationExpectation>::new(),
    )?;
    let report = verify(
        pool,
        &context()?,
        VerificationPlan::exact_role(&compiled)
            .with_sqlx_migrations(&absent_subset)?
            .with_schema_inspection(policy)?,
    )
    .await?;
    require(
        [
            SupportedSurface::RoleMembership,
            SupportedSurface::SqlxMigrationLedger,
            SupportedSurface::SecurityDefinerConfiguration,
        ]
        .iter()
        .all(|surface| report.supported().contains(surface))
            && !report
                .unsupported()
                .contains(&UnsupportedSurface::UnprotectedMigrationLedger),
        "one protected request did not merge exact-role, absent-subset and schema evidence",
    )?;
    require(
        report.is_within_declared_policy(),
        "the canonical combined protected request did not pass cleanly",
    )?;

    require_exact_generic_schema(pool, names, policy, &compiled).await?;
    require_generic_authority_sqlx_schema(pool, names, policy, &absent_subset).await?;
    require_generic_migration_schema(pool, names, policy).await
}

async fn require_exact_generic_schema(
    pool: &sqlx::PgPool,
    names: &Names,
    policy: &SchemaInspectionPolicy,
    compiled: &batter_sqlx::verification::CompiledExactRole,
) -> Result {
    let missing_generic = MigrationPolicy::new(
        QualifiedName::new(&names.schema_a, "absent_generic_history")?,
        [],
    )?;
    let exact_generic = verify(
        pool,
        &context()?,
        VerificationPlan::exact_role(compiled)
            .with_migrations(&missing_generic)?
            .with_schema_inspection(policy)?,
    )
    .await?;
    require(
        [
            SupportedSurface::MigrationLedger,
            SupportedSurface::SecurityDefinerConfiguration,
            SupportedSurface::CurrentDatabaseOwnership,
        ]
        .iter()
        .all(|surface| exact_generic.supported().contains(surface))
            && exact_generic
                .findings()
                .iter()
                .any(|finding| finding.kind == FindingKind::MissingMigration),
        "exact-role, generic-ledger and schema evidence was not retained together",
    )
}

async fn require_generic_authority_sqlx_schema(
    pool: &sqlx::PgPool,
    names: &Names,
    policy: &SchemaInspectionPolicy,
    absent_subset: &SqlxMigrationManifest,
) -> Result {
    let public_database = [ObjectPrivilege::Connect, ObjectPrivilege::Temporary]
        .into_iter()
        .map(|privilege| PublicGrant {
            object: PublicObject::Database,
            privilege: AllowedPrivilege::new(privilege, false),
        })
        .collect();
    let authority = AuthorityPolicyBuilder {
        roles: RolePolicy {
            allowed_admin_roles: vec![Identifier::new(&names.admin_target)?],
            ..RolePolicy::default()
        },
        database: DatabasePolicy {
            privileges: vec![
                AllowedPrivilege::new(ObjectPrivilege::Connect, false),
                AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
            ],
            allow_owner: false,
        },
        public_grants: public_database,
        ..AuthorityPolicyBuilder::default()
    }
    .build()?;
    let generic_sqlx = verify(
        pool,
        &context()?,
        VerificationPlan::authority(&authority)
            .with_sqlx_migrations(absent_subset)?
            .with_schema_inspection(policy)?,
    )
    .await?;
    require(
        [
            SupportedSurface::RoleMembership,
            SupportedSurface::DatabaseAcl,
            SupportedSurface::SqlxMigrationLedger,
            SupportedSurface::SecurityDefinerConfiguration,
        ]
        .iter()
        .all(|surface| generic_sqlx.supported().contains(surface))
            && generic_sqlx.is_within_declared_policy(),
        "generic authority, SQLx migration and schema evidence was not retained together",
    )
}

async fn require_generic_migration_schema(
    pool: &sqlx::PgPool,
    names: &Names,
    policy: &SchemaInspectionPolicy,
) -> Result {
    let missing_generic = MigrationPolicy::new(
        QualifiedName::new(&names.schema_a, "absent_generic_history")?,
        [],
    )?;
    let generic_schema = verify(
        pool,
        &context()?,
        VerificationPlan::migrations(&missing_generic).with_schema_inspection(policy)?,
    )
    .await?;
    require(
        [
            SupportedSurface::MigrationLedger,
            SupportedSurface::SecurityDefinerConfiguration,
        ]
        .iter()
        .all(|surface| generic_schema.supported().contains(surface))
            && generic_schema
                .findings()
                .iter()
                .any(|finding| finding.kind == FindingKind::MissingMigration),
        "generic migration and schema evidence was not retained together",
    )
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_sqlx_ledger_checks_exact_shape_and_nonprefix_subset() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let table_name = "sqlx_history";
                let table = format!("{}.{}", quote(&names.schema_a), quote(table_name));
                create_sqlx_ledger(&mut fixture.admin, &table).await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "INSERT INTO {table} VALUES
                     (1, 'one', now(), true, '\\x01', 1),
                     (3, 'three', now(), true, '\\x03', 1)"
                    ),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!("GRANT SELECT ON {table} TO {}", quote(&names.login_a)),
                )
                .await?;
                let pool = fixture.login(&names.login_a).await?;
                install_legacy_holdable_history_cursor(&pool).await?;
                let ledger = QualifiedName::new(&names.schema_a, table_name)?;
                let exact = SqlxMigrationManifest::new(
                    ledger.clone(),
                    SqlxLedgerMode::Exact,
                    [
                        MigrationExpectation::new(1, [1]),
                        MigrationExpectation::new(3, [3]),
                    ],
                )?;
                let report = verify_sqlx_migrations(&pool, &context()?, &exact).await?;
                require(
                    report.status() == VerificationStatus::WithinDeclaredPolicy
                        && report
                            .supported()
                            .contains(&SupportedSurface::SqlxMigrationLedger),
                    "canonical SQLx ledger did not pass exact verification",
                )?;

                let subset = SqlxMigrationManifest::new(
                    ledger,
                    SqlxLedgerMode::InstalledSubset,
                    [
                        MigrationExpectation::new(1, [1]),
                        MigrationExpectation::new(2, [2]),
                        MigrationExpectation::new(3, [3]),
                        MigrationExpectation::new(4, [4]),
                    ],
                )?;
                require(
                    verify_sqlx_migrations(&pool, &context()?, &subset)
                        .await?
                        .is_within_declared_policy(),
                    "installed subset was treated as a prefix",
                )?;

                require_missing_select_is_native(&mut fixture.admin, &pool, &names, &table, &exact)
                    .await?;

                require_checksum_length_guard(
                    &mut fixture.admin,
                    &pool,
                    &names,
                    table_name,
                    &table,
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!("ALTER TABLE {table} ADD COLUMN extra integer NOT NULL DEFAULT 0"),
                )
                .await?;
                require(
                    verify_sqlx_migrations(&pool, &context()?, &subset)
                        .await?
                        .findings()
                        .iter()
                        .any(|finding| finding.kind == FindingKind::SqlxLedgerShape),
                    "extra SQLx ledger column escaped exact shape verification",
                )?;

                require_identifier_literal_guard(&mut fixture.admin, &pool, &names).await?;
                require_catalog_type_identity_guard(&mut fixture.admin, &pool, &names).await?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_schema_checks_all_definers_and_exact_stored_search_path() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let schema = quote(&names.schema_a);
                exec(
                    &mut fixture.admin,
                    format!(
                        "CREATE FUNCTION {schema}.configured_definer() RETURNS integer
                     LANGUAGE sql SECURITY DEFINER SET search_path TO pg_catalog, pg_temp
                     SET statement_timeout TO '1s' AS 'SELECT 1'"
                    ),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "CREATE FUNCTION {schema}.configured_trigger() RETURNS trigger
                     LANGUAGE plpgsql SECURITY DEFINER
                     SET search_path TO pg_catalog, pg_temp
                     AS 'BEGIN RETURN NEW; END'"
                    ),
                )
                .await?;
                for routine in ["configured_definer()", "configured_trigger()"] {
                    exec(
                        &mut fixture.admin,
                        format!("REVOKE ALL ON FUNCTION {schema}.{routine} FROM PUBLIC"),
                    )
                    .await?;
                }
                let pool = fixture.login(&names.login_a).await?;
                let policy =
                    SchemaInspectionPolicy::canonical([Identifier::new(&names.schema_a)?])?;
                let plan = VerificationPlan::schema_inspection(&policy);
                let report = verify(&pool, &context()?, plan).await?;
                require(
                    report.is_within_declared_policy()
                        && report.supported() == [SupportedSurface::SecurityDefinerConfiguration]
                        && report.unsupported() == [UnsupportedSurface::SecurityDefinerBody]
                        && report
                            .supported()
                            .contains(&SupportedSurface::SecurityDefinerConfiguration),
                    "exact ungranted and trigger definer settings did not pass",
                )?;

                require_combined_protected_request(&pool, &names, &policy).await?;
                let missing_schema = format!("{}_missing", names.schema_a);
                let missing_policy = SchemaInspectionPolicy::canonical([
                    Identifier::new(&names.schema_a)?,
                    Identifier::new(&missing_schema)?,
                ])?;
                let missing_report = verify(
                    &pool,
                    &context()?,
                    VerificationPlan::schema_inspection(&missing_policy),
                )
                .await?;
                require(
                    missing_report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::MissingObject
                            && finding.object.as_deref()
                                == Some(format!("schema:{}", quote(&missing_schema)).as_str())
                    }),
                    "an absent selected schema was treated as an empty schema",
                )?;
                exec(
                    &mut fixture.admin,
                    format!("ALTER FUNCTION {schema}.configured_trigger() RESET search_path"),
                )
                .await?;
                require(
                    verify(&pool, &context()?, plan)
                        .await?
                        .findings()
                        .iter()
                        .any(|finding| finding.kind == FindingKind::SecurityDefinerSearchPath),
                    "missing trigger-function search_path escaped schema verification",
                )?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_exact_role_rejects_set_reachable_ownership() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let table = format!("{}.reachable_owner", quote(&names.schema_a));
                exec(
                    &mut fixture.admin,
                    format!("CREATE TABLE {table} (id integer)"),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!("ALTER TABLE {table} OWNER TO {}", quote(&names.settable)),
                )
                .await?;
                let pool = fixture.login(&names.login_a).await?;
                let mut manifest = ExactRoleManifest::new(
                    Identifier::new(&names.schema_a)?,
                    DiscoveryScope::Declared,
                )?;
                manifest.deny_current_database_ownership(true);
                let compiled = manifest.compile()?;
                let report = verify_exact_role(&pool, &context()?, &compiled).await?;
                require(
                    report
                        .supported()
                        .contains(&SupportedSurface::CurrentDatabaseOwnership)
                        && report.findings().iter().any(|finding| {
                            finding.kind == FindingKind::ReachableOwnership
                                && finding.subject.as_deref() == Some(names.settable.as_str())
                                && finding
                                    .object
                                    .as_deref()
                                    .is_some_and(|object| object.contains("objsubid=0"))
                        }),
                    "SET-reachable current-database ownership escaped the protected role",
                )?;
                Ok(())
            })
        })
        .await
}
