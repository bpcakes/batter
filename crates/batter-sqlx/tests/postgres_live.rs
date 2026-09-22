//! Ignored in ordinary gates; scripts/test_sqlx_live.sh requires every case.
mod support;

use batter_core::{
    BoxError,
    operation::{Interruption, OperationContext, OperationError},
};
use batter_sqlx::{
    FailureClass, PgLease, SqlxFailure, probe,
    verification::{
        AdditionalMigrations, AllowedPrivilege, AuthorityPolicyBuilder, DatabasePolicy, Identifier,
        MigrationExpectation, MigrationPolicy, ObjectPrivilege, PublicGrant, PublicObject,
        RolePolicy, SchemaPolicy, VerificationPlan, VerificationStatus, verify,
    },
};
use std::time::Duration;
use support::{Fixture, Result, bounded, require};
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
enum Stop {
    Cancel,
    Deadline,
    Error,
    Panic,
    Drop,
}

async fn blocked_operation(
    pool: sqlx::PgPool,
    context: OperationContext,
    key: i64,
    pid: oneshot::Sender<i32>,
    stop: oneshot::Receiver<Stop>,
) -> std::result::Result<(), OperationError<BoxError>> {
    context.run("test.blocked", |_| async {
        let lease = PgLease::acquire(&pool, &context).await?;
        lease.with_connection(async |session| {
            let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(session.executor())
                .await?;
            let _ = pid.send(backend);
            tokio::select! {
                result = sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(session.executor()) => {
                    result?;
                    Err(std::io::Error::other("blocker unexpectedly released").into())
                }
                signal = stop => match signal {
                    Ok(Stop::Panic) => panic!("deliberate application unwind"),
                    _ => Err(std::io::Error::other("concrete application failure").into()),
                }
            }
        }).await
    }).await
}

async fn interrupt(fixture: &mut Fixture, mode: Stop) -> Result {
    let budget = if matches!(mode, Stop::Deadline) {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(30)
    };
    let owner = batter_core::operation::OperationOwner::new(budget)?;
    let context = owner.context().clone();
    let (pid_tx, pid_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let mut task = tokio::spawn(blocked_operation(
        fixture.pool.clone(),
        context.clone(),
        fixture.key,
        pid_tx,
        stop_rx,
    ));
    let body = async {
        let pid = bounded(pid_rx).await??;
        fixture.retired.push(pid);
        fixture.blocked(pid).await?;
        match mode {
            Stop::Cancel => owner.cancel(),
            Stop::Drop => task.abort(),
            Stop::Error | Stop::Panic => {
                let _ = stop_tx.send(mode);
            }
            Stop::Deadline => {}
        }
        let result = tokio::time::timeout(Duration::from_secs(7), &mut task).await?;
        match mode {
            Stop::Cancel => require(
                matches!(
                    result,
                    Ok(Err(OperationError::Interrupted(Interruption::Cancelled)))
                ),
                "cancellation classification lost",
            )?,
            Stop::Deadline => require(
                matches!(
                    result,
                    Ok(Err(OperationError::Interrupted(
                        Interruption::DeadlineExceeded
                    )))
                ),
                "deadline classification lost",
            )?,
            Stop::Panic => require(
                matches!(result, Err(error) if error.is_panic()),
                "panic did not propagate",
            )?,
            Stop::Drop => require(
                matches!(result, Err(error) if error.is_cancelled()),
                "outer drop did not cancel task",
            )?,
            Stop::Error => {
                let Ok(Err(OperationError::Failed(error))) = result else {
                    return Err(std::io::Error::other("application error lost").into());
                };
                require(
                    error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.to_string() == "concrete application failure"),
                    "original application cause lost",
                )?;
            }
        }
        // The server still owns this blocked SQL after client task destruction.
        fixture.blocked(pid).await?;
        require(
            fixture.pool.size() == 0,
            "retirement retained pool capacity",
        )?;
        Ok(())
    }
    .await;
    if !task.is_finished() {
        task.abort();
        let cleanup = bounded(&mut task).await.map(|_| ());
        return support::combine(body, cleanup);
    }
    body
}

async fn run_interruption(mode: Stop, repetitions: usize) -> Result {
    let mut fixture = Fixture::new().await?;
    let body = async {
        for _ in 0..repetitions {
            interrupt(&mut fixture, mode).await?;
        }
        require(
            fixture
                .retired
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == repetitions,
            "backend identities were not independent",
        )?;
        fixture.replacement_and_close().await
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_cancellation_releases_capacity() -> Result {
    run_interruption(Stop::Cancel, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_deadline_releases_capacity() -> Result {
    run_interruption(Stop::Deadline, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_error_releases_capacity() -> Result {
    run_interruption(Stop::Error, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_panic_releases_capacity() -> Result {
    run_interruption(Stop::Panic, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_outer_drop_releases_capacity() -> Result {
    run_interruption(Stop::Drop, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn repeated_interruptions_leave_independent_residual_sessions() -> Result {
    run_interruption(Stop::Cancel, 3).await
}

async fn raw_transaction_cleanup(
    fixture: &Fixture,
    context: &OperationContext,
    expected_pid: i32,
    abort_transaction: bool,
) -> Result {
    let lease = PgLease::acquire(&fixture.pool, context).await?;
    let raw_pid = lease
        .with_connection(async |session| {
            sqlx::raw_sql("BEGIN").execute(session.executor()).await?;
            let raw_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(session.executor())
                .await?;
            sqlx::query(
                "CREATE TEMP TABLE batter_raw_return_guard (marker integer) ON COMMIT DROP",
            )
            .execute(session.executor())
            .await?;
            sqlx::query("INSERT INTO batter_raw_return_guard VALUES (1)")
                .execute(session.executor())
                .await?;
            if abort_transaction {
                let failed = sqlx::query("SELECT 1 / 0")
                    .execute(session.executor())
                    .await;
                require(
                    failed.is_err(),
                    "raw transaction did not enter failed state",
                )?;
            }
            Ok::<_, BoxError>(raw_pid)
        })
        .await?;
    require(
        raw_pid == expected_pid,
        "raw transaction did not use the reusable backend",
    )?;
    let (reused_pid, object_absent): (i32, bool) = bounded(
        sqlx::query_as(
            "SELECT pg_backend_pid(), to_regclass('pg_temp.batter_raw_return_guard') IS NULL",
        )
        .fetch_one(&fixture.pool),
    )
    .await??;
    require(
        reused_pid == expected_pid,
        "raw transaction cleanup retired the reusable backend",
    )?;
    require(
        object_absent,
        "raw transaction state escaped the pool-return boundary",
    )
}

async fn success(fixture: &mut Fixture) -> Result {
    let context =
        batter_core::operation::OperationOwner::new(Duration::from_secs(10))?.into_context();
    let lease = PgLease::acquire(&fixture.pool, &context).await?;
    let pid: i32 = lease
        .with_connection(async |session| {
            sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(session.executor())
                .await
        })
        .await?;
    probe(&fixture.pool, &context).await?;
    let actual: i32 =
        bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool)).await??;
    require(
        actual == pid,
        "successful low-level session did not permit reuse",
    )?;
    for abort_transaction in [false, true] {
        raw_transaction_cleanup(fixture, &context, pid, abort_transaction).await?;
    }
    // Sessions no longer expose typed begin/drop/forget compositions. Their
    // absence is compile-fail tested; atomic_live covers the owned runner and
    // the single low-level owner's completion and cancellation disposition.
    bounded(fixture.pool.close()).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn acknowledged_transactions_reuse_and_abandoned_transactions_retire() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = success(&mut fixture).await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_uses_one_read_only_snapshot_and_preserves_ledger_policy() -> Result {
    use futures_util::FutureExt as _;
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| std::io::Error::other("live checks require DATABASE_URL"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let schema = format!(
        "batter_ledger_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&pool)
        .await?;
    let context =
        batter_core::operation::OperationOwner::new(Duration::from_secs(30))?.into_context();
    let body = std::panic::AssertUnwindSafe(async {
    support::bounded(
        sqlx::query(
            sqlx::AssertSqlSafe(format!("CREATE TABLE {schema}._batter_verification_migrations (
                version BIGINT NOT NULL,
                description TEXT NOT NULL,
                installed_on TIMESTAMPTZ NOT NULL,
                checksum BYTEA NOT NULL,
                execution_time BIGINT NOT NULL,
                success BOOLEAN NOT NULL,
                PRIMARY KEY (version)
            )")),
        )
        .execute(&pool),
    )
    .await??;
    support::bounded(
        sqlx::query(
            sqlx::AssertSqlSafe(format!("INSERT INTO {schema}._batter_verification_migrations
                (version, description, installed_on, checksum, execution_time, success)
             VALUES (1, 'required', now(), $1, 1, true),
                    (2, 'later', now(), $2, 1, true)")),
        )
        .bind(vec![1_u8])
        .bind(vec![2_u8])
        .execute(&pool),
    )
    .await??;

    let migration = MigrationPolicy::new(
            batter_sqlx::verification::QualifiedName::new(
                &schema,
                "_batter_verification_migrations",
            )?,
            [MigrationExpectation::new(1, [1_u8])],
        )?
        .with_additional(AdditionalMigrations::AllowSuccessful)?;
    let authority = AuthorityPolicyBuilder {
            roles: RolePolicy {
                allow_superuser: true,
                allow_create_database: true,
                allow_create_role: true,
                allow_replication: true,
                allow_bypass_rls: true,
                allowed_admin_roles: Vec::new(),
                allowed_predefined_roles: [
                    "pg_checkpoint",
                    "pg_create_subscription",
                    "pg_database_owner",
                    "pg_execute_server_program",
                    "pg_maintain",
                    "pg_monitor",
                    "pg_read_all_data",
                    "pg_read_all_settings",
                    "pg_read_all_stats",
                    "pg_read_server_files",
                    "pg_signal_autovacuum_worker",
                    "pg_signal_backend",
                    "pg_stat_scan_tables",
                    "pg_use_reserved_connections",
                    "pg_write_all_data",
                    "pg_write_server_files",
                ]
                .into_iter()
                .map(Identifier::new)
                .collect::<std::result::Result<Vec<_>, _>>()?,
            },
            relations: Vec::new(),
            sequences: Vec::new(),
            schemas: vec![SchemaPolicy {
                schema: batter_sqlx::verification::Identifier::new("public")?,
                privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
                allow_owner: true,
            }],
            routines: Vec::new(),
            types: Vec::new(),
            parameters: Vec::new(),
            database: DatabasePolicy {
                privileges: vec![
                    AllowedPrivilege::new(ObjectPrivilege::Connect, false),
                    AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
                ],
                allow_owner: true,
            },
            public_grants: vec![
                PublicGrant {
                    object: PublicObject::Database,
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Connect, false),
                },
                PublicGrant {
                    object: PublicObject::Database,
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
                },
                PublicGrant {
                    object: PublicObject::Schema(batter_sqlx::verification::Identifier::new(
                        "public",
                    )?),
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Usage, false),
                },
            ],
            required_surfaces: Vec::new(),
            ..AuthorityPolicyBuilder::default()
        }
        .build()?;
    let report = verify(
        &pool,
        &context,
        VerificationPlan::migrations(&migration).with_authority(&authority)?,
    )
    .await?;
    require(
        report.status() == VerificationStatus::WithinDeclaredPolicy,
        "successful read-only verification did not match its declared policy",
    )?;
    require(
        report
            .unsupported()
            .contains(&batter_sqlx::verification::UnsupportedSurface::SecurityDefinerBody),
        "unsupported security-definer coverage was not explicit",
    )?;
    let count: i64 = support::bounded(
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {schema}._batter_verification_migrations")))
            .fetch_one(&pool),
    )
    .await??;
    require(count == 2, "verification changed the migration ledger")?;

    let mut incomplete_draft = authority.to_builder();
    incomplete_draft.required_surfaces =
        vec![batter_sqlx::verification::RequiredSurface::SecurityDefinerBody];
    let incomplete_authority = incomplete_draft.build()?;
    let report = verify(
        &pool,
        &context,
        VerificationPlan::migrations(&migration).with_authority(&incomplete_authority)?,
    )
    .await?;
    require(
        report.status() == VerificationStatus::Incomplete,
        "required unsupported coverage did not produce an incomplete result",
    )?;

    let reject_later = MigrationPolicy::new(
        migration.ledger().clone(),
        migration.required().iter().cloned(),
    )?;
    let report = verify(
        &pool,
        &context,
        VerificationPlan::migrations(&reject_later).with_authority(&authority)?,
    )
    .await?;
    require(
        report.status() == VerificationStatus::Violations
            && report.findings().iter().any(|finding| {
                finding.kind == batter_sqlx::verification::FindingKind::UnexpectedMigration
            }),
        "an unlisted later migration was not rejected",
    )?;

    support::bounded(
        sqlx::query(
            sqlx::AssertSqlSafe(format!("UPDATE {schema}._batter_verification_migrations
             SET checksum = $1 WHERE version = 1")),
        )
        .bind(vec![9_u8])
        .execute(&pool),
    )
    .await??;
    let report = verify(&pool, &context, VerificationPlan::migrations(&migration).with_authority(&authority)?).await?;
    require(
        report.status() == VerificationStatus::Violations
            && report.findings().iter().any(|finding| {
                finding.kind == batter_sqlx::verification::FindingKind::MigrationChecksumMismatch
            }),
        "checksum mismatch was not rejected",
    )?;

    support::bounded(
        sqlx::query(
            sqlx::AssertSqlSafe(format!("UPDATE {schema}._batter_verification_migrations SET success = false WHERE version = 2")),
        )
        .execute(&pool),
    )
    .await??;
    let report = verify(&pool, &context, VerificationPlan::migrations(&migration).with_authority(&authority)?).await?;
    require(
        report.status() == VerificationStatus::Violations
            && report.findings().iter().any(|finding| {
                finding.kind == batter_sqlx::verification::FindingKind::UnexpectedMigration
            }),
        "an unsuccessful allowed-later migration was not rejected",
    )?;

    support::bounded(
        sqlx::query(
            sqlx::AssertSqlSafe(format!("UPDATE {schema}._batter_verification_migrations
             SET checksum = $1, success = false WHERE version = 1")),
        )
        .bind(vec![1_u8])
        .execute(&pool),
    )
    .await??;
    let report = verify(&pool, &context, VerificationPlan::migrations(&migration).with_authority(&authority)?).await?;
    require(
        report.status() == VerificationStatus::Violations
            && report.findings().iter().any(|finding| {
                finding.kind == batter_sqlx::verification::FindingKind::UnsuccessfulMigration
            }),
        "unsuccessful migration was not rejected",
    )?;

    support::bounded(
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {schema}._batter_verification_migrations WHERE version = 1")))
            .execute(&pool),
    )
    .await??;
    let report = verify(&pool, &context, VerificationPlan::migrations(&migration).with_authority(&authority)?).await?;
    require(
        report.status() == VerificationStatus::Violations
            && report.findings().iter().any(|finding| {
                finding.kind == batter_sqlx::verification::FindingKind::MissingMigration
            }),
        "a missing required migration was not rejected",
    )?;

    support::bounded(
        sqlx::query(
            sqlx::AssertSqlSafe(format!("INSERT INTO {schema}._batter_verification_migrations
                (version, description, installed_on, checksum, execution_time, success)
             SELECT version, 'capacity', now(), decode('01', 'hex'), 1, true
             FROM generate_series(10, 10010) AS version")),
        )
        .execute(&pool),
    )
    .await??;
    let report = verify(&pool, &context, VerificationPlan::migrations(&migration).with_authority(&authority)?).await?;
    require(
        report.status() == VerificationStatus::Violations
            && report.findings().iter().any(|finding| {
                finding.kind == batter_sqlx::verification::FindingKind::MigrationLedgerLimit
            }),
        "an oversized migration ledger was not rejected at the bounded row limit",
    )?;
    Ok(())
    }).catch_unwind().await;
    let cleanup: Result = async {
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await?;
        Ok(())
    }
    .await;
    pool.close().await;
    let body = body.unwrap_or_else(|_| {
        Err(std::io::Error::other("verification ledger control panicked").into())
    });
    support::combine(body, cleanup)
}

async fn native_failure(fixture: &mut Fixture) -> Result {
    let context =
        batter_core::operation::OperationOwner::new(Duration::from_secs(10))?.into_context();
    let lease = PgLease::acquire(&fixture.pool, &context).await?;
    let (pid, error) = lease
        .with_retiring_connection(async |session| {
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(session.executor())
                .await?;
            let error = sqlx::query("SELECT 1 / 0 /* secret-query-marker */")
                .execute(session.executor())
                .await
                .err()
                .ok_or_else(|| std::io::Error::other("expected database failure"))?;
            Ok::<_, BoxError>((pid, error))
        })
        .await?;
    fixture.retired.push(pid);
    let error = SqlxFailure::from(error);
    require(
        error.class() == FailureClass::Database,
        "database classification lost",
    )?;
    require(
        error
            .native()
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref()
            == Some("22012"),
        "SQLSTATE lost",
    )?;
    require(
        format!("{error:?} {error}") == "PostgreSQL operation failed PostgreSQL operation failed",
        "diagnostics were not fixed",
    )?;
    let next: i32 =
        bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool)).await??;
    require(next != pid, "failed connection was reused")?;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn native_database_failure_retires_and_preserves_cause() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = native_failure(&mut fixture).await;
    fixture.finish(body).await
}

async fn commit_failure(fixture: &mut Fixture) -> Result {
    let tx = batter_sqlx::low_level::PgAtomicTransaction::begin(&fixture.pool).await?;
    let (tx, pid) = tx.application(async |session| {
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(session.executor())
            .await?;
        sqlx::query("CREATE TEMP TABLE disposition_commit (value integer UNIQUE DEFERRABLE INITIALLY DEFERRED) ON COMMIT DROP")
            .execute(session.executor()).await?;
        sqlx::query("INSERT INTO disposition_commit VALUES (1), (1)")
            .execute(session.executor())
            .await?;
        Ok::<_, sqlx::Error>(pid)
    }).await?;
    let error = tx
        .commit()
        .await
        .expect_err("deferred constraint failure at commit");
    fixture.retired.push(pid);
    let batter_sqlx::PgTransactionError::Query(error) = error.into_cause() else {
        return Err(std::io::Error::other("expected native commit cause").into());
    };
    require(
        error
            .native()
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref()
            == Some("23505"),
        "commit SQLSTATE lost",
    )?;
    require(
        error.class() == FailureClass::Database,
        "commit error was reclassified as interruption",
    )?;
    let next: i32 =
        bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool)).await??;
    require(next != pid, "unsuccessful commit returned its connection")?;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn rejected_commit_preserves_native_cause_and_retires() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = bounded(commit_failure(&mut fixture))
        .await
        .and_then(|result| result);
    fixture.finish(body).await
}

async fn ordinary_return(fixture: &mut Fixture) -> Result {
    let mut connection = bounded(fixture.pool.acquire()).await??;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await?;
    fixture.retired.push(pid);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let key = fixture.key;
    let mut task = tokio::spawn(async move {
        tokio::select! {
            _ = stop_rx => {},
            _ = sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(&mut *connection) => {},
        }
        // The original SQLx pool-return behavior: asynchronous health ping.
        drop(connection);
    });
    let body = async {
        fixture.blocked(pid).await?;
        let _ = stop_tx.send(());
        bounded(&mut task).await??;
        require(
            fixture.pool.size() == 1,
            "ordinary return unexpectedly retired accounting",
        )?;
        require(
            matches!(fixture.pool.acquire().await, Err(sqlx::Error::PoolTimedOut)),
            "ordinary return did not retain blocked slot",
        )?;
        require(
            tokio::time::timeout(Duration::from_millis(200), fixture.pool.close())
                .await
                .is_err(),
            "ordinary pool close unexpectedly completed before unlock",
        )?;
        fixture.blocked(pid).await
    }
    .await;
    if !task.is_finished() {
        task.abort();
        return support::combine(body, bounded(&mut task).await.map(|_| ()));
    }
    body
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn ordinary_return_control_retains_blocked_capacity() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = ordinary_return(&mut fixture).await;
    fixture.finish(body).await
}
