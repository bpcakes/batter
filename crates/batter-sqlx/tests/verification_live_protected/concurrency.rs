use super::super::support::{Result, require};
use super::super::{AuthorityFixture, Names, exec, quote};
use super::{context, create_sqlx_ledger};
use batter_sqlx::test_support::{
    pause_next_verification_ledger_history, pause_next_verification_ledger_lock,
};
use batter_sqlx::verification::{
    FindingKind, MigrationExpectation, QualifiedName, SqlxLedgerMode, SqlxMigrationManifest,
    UnsupportedSurface, VerificationStatus, verify_sqlx_migrations,
};

async fn replace_ledger_schema(
    admin: &mut sqlx::PgConnection,
    names: &Names,
    table_name: &str,
) -> Result<String> {
    let original_schema = quote(&names.schema_a);
    let renamed_schema_name = format!("{}_old", names.schema_a);
    let renamed_schema = quote(&renamed_schema_name);
    exec(
        &mut *admin,
        format!("ALTER SCHEMA {original_schema} RENAME TO {renamed_schema}"),
    )
    .await?;
    exec(&mut *admin, format!("CREATE SCHEMA {original_schema}")).await?;
    exec(
        &mut *admin,
        format!(
            "GRANT USAGE ON SCHEMA {original_schema} TO {}",
            quote(&names.login_a)
        ),
    )
    .await?;
    let replacement = format!("{original_schema}.{}", quote(table_name));
    create_sqlx_ledger(&mut *admin, &replacement).await?;
    exec(
        &mut *admin,
        format!("GRANT SELECT ON {replacement} TO {}", quote(&names.login_a)),
    )
    .await?;
    Ok(renamed_schema)
}

async fn restore_ledger_schema(
    admin: &mut sqlx::PgConnection,
    names: &Names,
    renamed_schema: &str,
) -> Result {
    let original_schema = quote(&names.schema_a);
    exec(
        &mut *admin,
        format!("DROP SCHEMA {original_schema} CASCADE"),
    )
    .await?;
    exec(
        admin,
        format!("ALTER SCHEMA {renamed_schema} RENAME TO {original_schema}"),
    )
    .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_sqlx_absent_lock_cannot_admit_late_ledger() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let table_name = "late_sqlx_history";
                let table = format!("{}.{}", quote(&names.schema_a), quote(table_name));
                let pool = fixture.login(&names.login_a).await?;
                let manifest = SqlxMigrationManifest::new(
                    QualifiedName::new(&names.schema_a, table_name)?,
                    SqlxLedgerMode::InstalledSubset,
                    [MigrationExpectation::new(1, [1])],
                )?;
                let pause = pause_next_verification_ledger_lock();
                let verification_context = context()?;
                let verification = verify_sqlx_migrations(&pool, &verification_context, &manifest);
                tokio::pin!(verification);
                tokio::select! {
                    result = &mut verification => {
                        // This case checks session behaviour, not the verdict.
                        let _report = result?;
                        return Err(std::io::Error::other(
                            "verifier completed before the missing-ledger barrier"
                        ).into());
                    }
                    () = pause.wait_until_reached() => {}
                }
                create_sqlx_ledger(&mut fixture.admin, &table).await?;
                exec(
                    &mut fixture.admin,
                    format!("GRANT SELECT ON {table} TO {}", quote(&names.login_a)),
                )
                .await?;
                pause.release();
                let report = verification.await?;
                require(
                    report.status() == VerificationStatus::Incomplete
                        && report
                            .unsupported()
                            .contains(&UnsupportedSurface::UnprotectedMigrationLedger),
                    "a ledger created after the absent lock was treated as protected",
                )?;
                require(
                    verify_sqlx_migrations(&pool, &context()?, &manifest)
                        .await?
                        .is_within_declared_policy(),
                    "the late-ledger control was not canonical after creation",
                )?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_sqlx_name_replacement_is_not_mistaken_for_locked_ledger() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let table_name = "replaced_sqlx_history";
                let original_schema = quote(&names.schema_a);
                let original_table = format!("{original_schema}.{}", quote(table_name));
                create_sqlx_ledger(&mut fixture.admin, &original_table).await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "GRANT SELECT ON {original_table} TO {}",
                        quote(&names.login_a)
                    ),
                )
                .await?;
                let pool = fixture.login(&names.login_a).await?;
                let manifest = SqlxMigrationManifest::new(
                    QualifiedName::new(&names.schema_a, table_name)?,
                    SqlxLedgerMode::InstalledSubset,
                    Vec::<MigrationExpectation>::new(),
                )?;
                let pause = pause_next_verification_ledger_lock();
                let verification_context = context()?;
                let verification = verify_sqlx_migrations(&pool, &verification_context, &manifest);
                tokio::pin!(verification);
                tokio::select! {
                    result = &mut verification => {
                        // This case checks session behaviour, not the verdict.
                        let _report = result?;
                        return Err(std::io::Error::other(
                            "verifier completed before the protected-ledger barrier"
                        ).into());
                    }
                    () = pause.wait_until_reached() => {}
                }

                let renamed_schema =
                    replace_ledger_schema(&mut fixture.admin, &names, table_name).await?;
                pause.release();
                let report = verification.await?;
                require(
                    report.status() == VerificationStatus::Incomplete
                        && report
                            .unsupported()
                            .contains(&UnsupportedSurface::UnprotectedMigrationLedger),
                    "an unlocked replacement relation was treated as the locked ledger",
                )?;

                restore_ledger_schema(&mut fixture.admin, &names, &renamed_schema).await?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_sqlx_history_rejects_post_snapshot_name_replacement() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let table_name = "post_snapshot_sqlx_history";
                let table = format!("{}.{}", quote(&names.schema_a), quote(table_name));
                create_sqlx_ledger(&mut fixture.admin, &table).await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "INSERT INTO {table} VALUES (7, 'unexpected', now(), true, '\\x07', 1)"
                    ),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!("GRANT SELECT ON {table} TO {}", quote(&names.login_a)),
                )
                .await?;
                let pool = fixture.login(&names.login_a).await?;
                let manifest = SqlxMigrationManifest::new(
                    QualifiedName::new(&names.schema_a, table_name)?,
                    SqlxLedgerMode::InstalledSubset,
                    Vec::<MigrationExpectation>::new(),
                )?;
                let pause = pause_next_verification_ledger_history();
                let verification_context = context()?;
                let verification = verify_sqlx_migrations(&pool, &verification_context, &manifest);
                tokio::pin!(verification);
                tokio::select! {
                    result = &mut verification => {
                        // This case checks session behaviour, not the verdict.
                        let _report = result?;
                        return Err(std::io::Error::other(
                            "verifier completed before the protected-history barrier"
                        ).into());
                    }
                    () = pause.wait_until_reached() => {}
                }
                let renamed_schema =
                    replace_ledger_schema(&mut fixture.admin, &names, table_name).await?;
                pause.release();
                let report = verification.await?;
                require(
                    report.status() == VerificationStatus::Incomplete
                        && report
                            .unsupported()
                            .contains(&UnsupportedSurface::UnprotectedMigrationLedger),
                    "post-snapshot replacement history was accepted for the locked ledger",
                )?;
                restore_ledger_schema(&mut fixture.admin, &names, &renamed_schema).await?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn protected_sqlx_guards_reject_rls_inheritance_and_overflow() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let table_name = "guarded_sqlx_history";
                let table = format!("{}.{}", quote(&names.schema_a), quote(table_name));
                create_sqlx_ledger(&mut fixture.admin, &table).await?;
                exec(
                    &mut fixture.admin,
                    format!("GRANT SELECT ON {table} TO {}", quote(&names.login_a)),
                )
                .await?;
                let pool = fixture.login(&names.login_a).await?;
                let manifest = SqlxMigrationManifest::new(
                    QualifiedName::new(&names.schema_a, table_name)?,
                    SqlxLedgerMode::InstalledSubset,
                    Vec::<MigrationExpectation>::new(),
                )?;

                exec(
                    &mut fixture.admin,
                    format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY"),
                )
                .await?;
                require(
                    verify_sqlx_migrations(&pool, &context()?, &manifest)
                        .await?
                        .findings()
                        .iter()
                        .any(|finding| finding.kind == FindingKind::LedgerRowSecurity),
                    "RLS-enabled protected ledger passed",
                )?;
                exec(
                    &mut fixture.admin,
                    format!("ALTER TABLE {table} DISABLE ROW LEVEL SECURITY"),
                )
                .await?;

                let child = format!("{}.guarded_sqlx_child", quote(&names.schema_a));
                exec(
                    &mut fixture.admin,
                    format!("CREATE TABLE {child} () INHERITS ({table})"),
                )
                .await?;
                require(
                    verify_sqlx_migrations(&pool, &context()?, &manifest)
                        .await?
                        .status()
                        == VerificationStatus::Incomplete,
                    "inherited protected ledger passed",
                )?;
                exec(&mut fixture.admin, format!("DROP TABLE {child}")).await?;

                exec(
                    &mut fixture.admin,
                    format!(
                        "INSERT INTO {table}
                         SELECT value, value::text, now(), true,
                                pg_catalog.int8send(value), 1
                         FROM pg_catalog.generate_series(1::bigint, 10001::bigint) AS value"
                    ),
                )
                .await?;
                require(
                    verify_sqlx_migrations(&pool, &context()?, &manifest)
                        .await?
                        .findings()
                        .iter()
                        .any(|finding| finding.kind == FindingKind::MigrationLedgerLimit),
                    "over-capacity protected ledger passed",
                )?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn protected_sqlx_late_attachment_cannot_supply_snapshot_rows() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                use sqlx::Connection;

                let names = fixture.names.clone();
                let table_name = "snapshot_sqlx_history";
                let table = format!("{}.{}", quote(&names.schema_a), quote(table_name));
                let child = format!("{}.snapshot_sqlx_child", quote(&names.schema_a));
                create_sqlx_ledger(&mut fixture.admin, &table).await?;
                exec(
                    &mut fixture.admin,
                    format!("CREATE TABLE {child} (LIKE {table} INCLUDING ALL)"),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!("INSERT INTO {child} VALUES (1, 'one', now(), true, '\\x01', 1)"),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "GRANT SELECT ON {table}, {child} TO {}",
                        quote(&names.login_a)
                    ),
                )
                .await?;
                let pool = fixture.login(&names.login_a).await?;
                let serving_warmup: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table}"
                )))
                .fetch_one(&pool)
                .await?;
                require(serving_warmup == 0, "protected ledger was not empty")?;
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&pool)
                    .await?;
                let namespace_catalog: i64 =
                    sqlx::query_scalar("SELECT 'pg_catalog.pg_namespace'::regclass::oid::bigint")
                        .fetch_one(&mut fixture.admin)
                        .await?;
                let mut mutator = sqlx::PgConnection::connect(&fixture.url).await?;
                let mut blocker = sqlx::PgConnection::connect(&fixture.url).await?;
                exec(&mut mutator, "SET statement_timeout = '3s'".into()).await?;
                let mutator_ledger_warmup: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table}"
                )))
                .fetch_one(&mut mutator)
                .await?;
                let mutator_child_warmup: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {child}"
                )))
                .fetch_one(&mut mutator)
                .await?;
                require(
                    mutator_ledger_warmup == 0 && mutator_child_warmup == 1,
                    "late-attachment relations were not warm before the barrier",
                )?;
                let mut barrier = blocker.begin().await?;
                sqlx::query("LOCK TABLE pg_catalog.pg_namespace IN ACCESS EXCLUSIVE MODE")
                    .execute(&mut *barrier)
                    .await?;
                let manifest = SqlxMigrationManifest::new(
                    QualifiedName::new(&names.schema_a, table_name)?,
                    SqlxLedgerMode::Exact,
                    [MigrationExpectation::new(1, [1])],
                )?;
                let verification_context = context()?;
                let verification = verify_sqlx_migrations(&pool, &verification_context, &manifest);
                tokio::pin!(verification);
                tokio::select! {
                    result = &mut verification => {
                        // This case checks session behaviour, not the verdict.
                        let _report = result?;
                        return Err(std::io::Error::other(
                            "verifier completed before the snapshot barrier"
                        ).into());
                    }
                    result = super::super::verification_live_recovery::pending_lock(
                        &mut fixture.admin,
                        pid,
                        namespace_catalog,
                    ) => result?,
                }
                let mut change = mutator.begin().await?;
                exec(&mut *change, format!("ALTER TABLE {child} INHERIT {table}")).await?;
                change.commit().await?;
                barrier.rollback().await?;
                let report = verification.await?;
                require(
                    report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::MissingMigration
                            && finding.subject.as_deref() == Some("1")
                    }),
                    "a child attached after snapshot supplied the protected ledger row",
                )?;
                mutator.close().await?;
                blocker.close().await?;
                Ok(())
            })
        })
        .await
}
