use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver,
};
use sqlx::{Connection, PgPool, postgres::PgPoolOptions};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{
    ProbeResult,
    fixture_observation::{BOUND, admin},
    fixture_run,
    fixture_sessions::phase,
};

pub async fn restricted_observer_sees_other_role() -> ProbeResult {
    role_case(false).await
}

pub async fn role_cleanup_after_assertion() -> ProbeResult {
    role_case(true).await
}

async fn role_case(inject_panic: bool) -> ProbeResult {
    let catalog = admin().await?;
    // The SQL identifier contains only this literal prefix and decimal digits.
    let role = format!(
        "fixture_monitor_{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let password = create_role(&catalog, &role).await?;
    let result = restricted_case(&catalog, &role, &password, inject_panic).await;
    let removed = sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE {role}")))
        .execute(&catalog)
        .await;
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role)
            .fetch_one(&catalog)
            .await;
    catalog.close().await;
    if inject_panic {
        removed?;
        assert!(!exists?);
        let error = result.expect_err("injected assertion must propagate");
        assert!(
            error
                .downcast_ref::<tokio::task::JoinError>()
                .is_some_and(|e| e.is_panic())
        );
        return Ok(());
    }
    batter_test_support::finish(
        result.map_err(fixture_run::ProbeError::new),
        removed.map(|_| ()),
    )?;
    assert!(!exists?);
    Ok(())
}

async fn create_role(catalog: &PgPool, role: &str) -> Result<String, sqlx::Error> {
    // PostgreSQL quotes the identifier/literals, generates fresh random credentials
    // and sets expiry from its own clock. A killed process cannot leave a known,
    // permanently valid password; expiry does not promise role removal.
    let (password, statement): (String, String) = sqlx::query_as(
        "WITH credentials AS MATERIALIZED (SELECT gen_random_uuid()::text AS password)
         SELECT password, format(
           'CREATE ROLE %I NOSUPERUSER NOINHERIT LOGIN PASSWORD %L VALID UNTIL %L',
           $1::text, password, clock_timestamp() + interval '5 minutes')
         FROM credentials",
    )
    .bind(role)
    .fetch_one(catalog)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(catalog)
        .await?;
    Ok(password)
}

async fn restricted_case(
    catalog: &PgPool,
    role: &str,
    password: &str,
    inject_panic: bool,
) -> ProbeResult {
    let observer_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            catalog
                .connect_options()
                .as_ref()
                .clone()
                .username(role)
                .password(password),
        )
        .await?;
    // Keep the pool in this cleanup owner while the assertion-bearing task runs.
    // Its native JoinError cannot unwind past pool close and the caller's DROP ROLE.
    let body_pool = observer_pool.clone();
    let body_catalog = catalog.clone();
    let result = tokio::spawn(async move {
        assert!(
            !inject_panic,
            "deliberate restricted-role assertion failure"
        );
        observe_other_role(&body_catalog, &body_pool).await
    })
    .await;
    observer_pool.close().await;
    result?
}

async fn observe_other_role(catalog: &PgPool, pool: &PgPool) -> ProbeResult {
    let expires: bool = sqlx::query_scalar(
        "SELECT COALESCE(rolvaliduntil > clock_timestamp() AND
         rolvaliduntil <= clock_timestamp() + interval '5 minutes', false)
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    assert!(expires);
    let privileged: bool = sqlx::query_scalar(
        "SELECT rolsuper OR pg_has_role(current_user, 'pg_read_all_stats', 'USAGE') FROM pg_roles WHERE rolname = current_user"
    ).fetch_one(pool).await?;
    assert!(!privileged);
    let control = SessionObserver::new(pool.clone(), Duration::from_millis(200))?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let run = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control.clone())
        .start(move |scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 1)?;
                let db = scope.empty(&plan).await?;
                let mut detached = db.pools()[0]
                    .acquire()
                    .await
                    .map_err(FixtureError::Observe)?
                    .detach();
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut detached)
                    .await
                    .map_err(FixtureError::Observe)?;
                send.send((detached, pid, db.database_name().to_owned()))
                    .unwrap_or_else(|_| panic!("observer alive"));
                Ok::<_, FixtureError>(())
            })
        });
    let (connection, pid, name) = tokio::time::timeout(BOUND, receive).await??;
    phase(&run, CleanupPhase::AwaitingRetry).await;
    let visible: (String, bool) = sqlx::query_as(
        "SELECT datname, usename <> current_user FROM pg_stat_activity WHERE pid = $1",
    )
    .bind(pid)
    .fetch_one(pool)
    .await?;
    let retained = fixture_run::present(catalog, &name).await;
    connection.close().await?;
    super::fixture_sessions::wait_sessions_absent(catalog, &name).await?;
    control.retry_with(pool.clone());
    let report = tokio::time::timeout(BOUND, run.into_report()).await??;
    fixture_run::absent(catalog, &name).await?;
    assert_eq!(visible, (name, true));
    assert!(retained? && !report.is_ok() && report.databases[0].result.is_ok());
    assert!(matches!(
        *report.databases[0].observation_failures[0],
        FixtureError::ObservationTimeout
    ));
    Ok(())
}
