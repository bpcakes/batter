use super::*;
use sqlx::postgres::PgPoolOptions;

pub async fn lost_commit_acknowledgement(pool: PgPool) -> ProbeResult {
    let (options, identity) = setup(&pool).await?;
    let job = enqueue(&pool).await?;
    pool.close().await;
    wait_idle(&options).await?;
    let proxy =
        super::proxy::CommitAckProxy::start(options.get_host().to_owned(), options.get_port())
            .await?;
    let result = retire(options.clone().port(proxy.port), identity).await;
    let fault = proxy.finish().await;
    assert!(
        fault?,
        "PostgreSQL must send the cancellation commit acknowledgement before it is discarded"
    );
    let primary = result?;
    let RetirementError::Cancellation(failure) = failure(&primary) else {
        panic!("retain uncertain cancellation commit")
    };
    assert_eq!(failure.job_id, job);
    assert_eq!(failure.cancelled_before_failure, 0);
    let runledger_postgres::Error::QueryError(error) = &failure.native else {
        panic!("retain original SQLx commit cause")
    };
    assert!(matches!(
        &*error.source_arc().unwrap(),
        sqlx::Error::Io(_) | sqlx::Error::Protocol(_)
    ));
    let observation = retirement::readback(
        options.clone(),
        identity,
        job,
        OperationContext::new(SECOND * 5)?,
        CleanupBudget::new(SECOND, SECOND, SECOND)?,
    )
    .start();
    let observed = batter::command::check_command(observation.wait().await)?;
    assert_eq!(observed.work.as_ref().unwrap().as_deref(), Some("CANCELED"));
    assert!(
        !primary.is_success(),
        "durable readback cannot replace the failed primary outcome"
    );
    let checking = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let events = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM job_events WHERE job_id=$1 AND event_type='CANCELED'",
    )
    .bind(job)
    .fetch_one(&checking)
    .await;
    checking.close().await;
    assert_eq!(
        events?, 1,
        "uncertain acknowledgement must not replay cancellation"
    );
    Ok(())
}

pub async fn commit_error_and_readback(pool: PgPool) -> ProbeResult {
    let (options, identity) = setup(&pool).await?;
    let job = enqueue(&pool).await?;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_retirement_commit() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'private cancellation commit' USING ERRCODE='23514'; END $$;
         CREATE CONSTRAINT TRIGGER reject_retirement_commit AFTER UPDATE ON job_queue
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.status='CANCELED')
         EXECUTE FUNCTION reject_retirement_commit();",
    )
    .execute(&pool)
    .await?;
    pool.close().await;
    wait_idle(&options).await?;
    let primary = retire(options.clone(), identity).await?;
    let retained = primary.clone();
    let checking = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await?;
    let closing = checking.clone();
    let body = tokio::spawn(async move {
        let RetirementError::Cancellation(failure) = failure(&primary) else { panic!("retain cancellation cause") };
        assert_eq!(failure.job_id, job);
        assert_eq!(failure.cancelled_before_failure, 0);
        let runledger_postgres::Error::QueryError(error) = &failure.native else { panic!("retain SQLx commit cause") };
        assert_eq!(error.sqlstate(), Some("23514"));
        assert!(!format!("{primary:?}").contains("private cancellation commit"));
        let mut blocked = checking.begin().await?;
        sqlx::query("LOCK job_queue IN ACCESS EXCLUSIVE MODE").execute(&mut *blocked).await?;
        let observation = retirement::readback(options.clone().application_name("retirement_readback_probe"), identity, job,
            OperationContext::new(SECOND * 10)?, CleanupBudget::new(SECOND * 3, SECOND * 3, SECOND)?).start();
        let witnessed = tokio::time::timeout(SECOND * 5, async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND application_name='retirement_readback_probe' AND wait_event_type='Lock')")
                    .fetch_one(&checking).await?;
                if waiting { return Ok::<_, sqlx::Error>(()); }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await;
        observation.cancel();
        let unlocked = blocked.rollback().await;
        let cancelled = observation.wait().await;
        unlocked?;
        witnessed??;
        let cancelled = cancelled?;
        assert!(matches!(&cancelled.work, Err(CommandCause::Interrupted(_))));
        assert!(cancelled.cleanup.as_ref().is_ok_and(|cleanup| cleanup.is_success()));
        let RetirementError::Cancellation(failure) = super::failure(&retained) else { panic!("primary survives readback cancellation") };
        let runledger_postgres::Error::QueryError(error) = &failure.native else { panic!("original remains concrete") };
        assert_eq!(error.source_arc().unwrap().as_database_error().unwrap().message(), "private cancellation commit");
        let observation = retirement::readback(options, identity, job, OperationContext::new(SECOND * 5)?,
            CleanupBudget::new(SECOND, SECOND, SECOND)?).start();
        let actual = batter::command::check_command(observation.wait().await)?;
        assert_eq!(actual.work.as_ref().unwrap().as_deref(), Some("PENDING"));
        assert!(!retained.is_success(), "readback never rewrites the primary failure");
        Ok::<_, BoxError>(())
    }).await;
    closing.close().await;
    body?
}
