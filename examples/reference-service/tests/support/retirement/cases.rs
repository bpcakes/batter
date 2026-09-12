use super::*;
use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};

pub async fn history_and_disable(pool: PgPool) -> ProbeResult {
    let (options, identity) = setup(&pool).await?;
    let terminal = enqueue(&pool).await?;
    runledger_postgres::jobs::cancel_job_with_scope(
        &pool,
        runledger_postgres::jobs::JobCancellationScope::Global,
        terminal,
        None,
    )
    .await?;
    let first = enqueue(&pool).await?;
    let second = enqueue(&pool).await?;
    crate::support::worker::catalog(None)
        .sync_definitions(&pool)
        .await?;
    let domain_job = crate::support::native_hosted::enqueue(&pool).await?;
    let domain_before = job_row(&pool, domain_job).await?;
    sqlx::query("INSERT INTO compatibility_probe(value) VALUES(77)")
        .execute(&pool)
        .await?;
    let history = job_row(&pool, terminal).await?;
    let migrations: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await?;
    let sequence: (i64, bool) =
        sqlx::query_as("SELECT last_value, is_called FROM startup_control_owner_epoch")
            .fetch_one(&pool)
            .await?;
    pool.close().await;
    wait_idle(&options).await?;
    let retired = retire(options.clone(), identity).await?;
    let repeated = retire(options.clone(), identity).await?;
    let checking = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let closing = checking.clone();
    let body = tokio::spawn(async move {
        assert!(retired.is_success());
        assert_eq!(retired.work.as_ref().unwrap().cancelled, 2);
        assert!(repeated.is_success());
        assert_eq!(repeated.work.as_ref().unwrap().cancelled, 0);
        let actual = job_row(&checking, terminal).await?;
        assert_eq!(actual, history);
        let domain_after = job_row(&checking, domain_job).await?;
        assert_eq!(
            domain_after, domain_before,
            "retirement must not mutate another job type"
        );
        let actual: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&checking)
                .await?;
        assert_eq!(actual, migrations);
        let actual: (i64, bool) =
            sqlx::query_as("SELECT last_value, is_called FROM startup_control_owner_epoch")
                .fetch_one(&checking)
                .await?;
        assert_eq!(actual, sequence);
        let domain: i64 = sqlx::query_scalar("SELECT value FROM compatibility_probe")
            .fetch_one(&checking)
            .await?;
        assert_eq!(domain, 77);
        let cancelled: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM job_queue WHERE id=ANY($1) AND status='CANCELED'",
        )
        .bind(vec![first, second])
        .fetch_one(&checking)
        .await?;
        assert_eq!(cancelled, 2);
        catalog().sync_definitions(&checking).await?;
        let enabled: bool =
            sqlx::query_scalar("SELECT is_enabled FROM job_definitions WHERE job_type=$1")
                .bind(LEGACY)
                .fetch_one(&checking)
                .await?;
        assert!(!enabled, "old additive catalog must preserve disable");
        assert!(
            enqueue(&checking).await.is_err(),
            "old native enqueue must stay disabled"
        );
        Ok::<_, BoxError>(())
    })
    .await;
    closing.close().await;
    body?
}

pub async fn wrong_identity(pool: PgPool) -> ProbeResult {
    let (options, _) = setup(&pool).await?;
    let report = retire(options, DatabaseIdentity::new(1, 1)?).await?;
    assert!(matches!(
        failure(&report),
        RetirementError::WrongDatabase { .. }
    ));
    let enabled: bool =
        sqlx::query_scalar("SELECT is_enabled FROM job_definitions WHERE job_type=$1")
            .bind(LEGACY)
            .fetch_one(&pool)
            .await?;
    assert!(enabled, "wrong target must not be mutated");
    Ok(())
}

pub async fn restricted_visibility(pool: PgPool) -> ProbeResult {
    let (options, identity) = setup(&pool).await?;
    let report = retire(
        options.options([("role", "pg_read_all_settings")]),
        identity,
    )
    .await?;
    assert!(
        matches!(failure(&report), RetirementError::NotOffline { other_backends, .. } if *other_backends > 0)
    );
    let enabled: bool =
        sqlx::query_scalar("SELECT is_enabled FROM job_definitions WHERE job_type=$1")
            .bind(LEGACY)
            .fetch_one(&pool)
            .await?;
    assert!(
        enabled,
        "unknown activity must refuse before native mutation"
    );
    Ok(())
}

pub async fn pending_transaction(pool: PgPool) -> ProbeResult {
    let (options, identity) = setup(&pool).await?;
    let mut tx = transactions::read_committed(&pool).await?;
    let job = enqueue_tx(&mut tx).await?;
    let rejected = retire(options.clone(), identity).await;
    tx.commit().await?;
    pool.close().await;
    wait_idle(&options).await?;
    let retired = retire(options.clone(), identity).await?;
    let mut connection = PgConnection::connect_with(&options).await?;
    let state = sqlx::query_scalar::<_, String>("SELECT status::text FROM job_queue WHERE id=$1")
        .bind(job)
        .fetch_one(&mut connection)
        .await;
    connection.close().await?;
    assert!(
        matches!(failure(&rejected?), RetirementError::NotOffline { other_backends, .. } if *other_backends > 0)
    );
    assert!(retired.is_success());
    assert_eq!(state?, "CANCELED");
    Ok(())
}

pub async fn prepared_transaction(pool: PgPool) -> ProbeResult {
    let (options, identity) = setup(&pool).await?;
    let (job, gid) = prepare_enqueue(&pool).await?;
    pool.close().await;
    wait_idle(&options).await?;
    let rejected = retire(options.clone(), identity).await;
    let mut connection = PgConnection::connect_with(&options).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("COMMIT PREPARED '{gid}'")))
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    let retired = retire(options.clone(), identity).await?;
    let mut connection = PgConnection::connect_with(&options).await?;
    let state = sqlx::query_scalar::<_, String>("SELECT status::text FROM job_queue WHERE id=$1")
        .bind(job)
        .fetch_one(&mut connection)
        .await;
    connection.close().await?;
    assert!(matches!(
        failure(&rejected?),
        RetirementError::NotOffline {
            prepared_transactions: 1,
            ..
        }
    ));
    assert!(retired.is_success());
    assert_eq!(state?, "CANCELED");
    Ok(())
}

async fn prepare_enqueue(pool: &PgPool) -> Result<(Uuid, String), BoxError> {
    let enabled: i32 =
        sqlx::query_scalar("SELECT current_setting('max_prepared_transactions')::int")
            .fetch_one(pool)
            .await?;
    assert!(
        enabled > 0,
        "explicit retirement acceptance requires prepared transactions enabled"
    );
    let gid = format!("retirement_{}", Uuid::now_v7().simple());
    let mut tx = transactions::read_committed(pool).await?;
    let job = enqueue_tx(&mut tx).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("PREPARE TRANSACTION '{gid}'")))
        .execute(&mut *tx)
        .await?;
    // SQLx's guard queues an ordinary ROLLBACK on drop. It cannot resolve the
    // detached prepared transaction; close the actual producer pool before probing.
    drop(tx);
    Ok((job, gid))
}

async fn job_row(pool: &PgPool, id: Uuid) -> Result<serde_json::Value, sqlx::Error> {
    sqlx::query_scalar("SELECT to_jsonb(q) FROM job_queue q WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
}
