use std::time::Duration;

use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver, observe_blocked,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;

use super::{ProbeResult, fixture_run, fixture_run::present};

const BOUND: Duration = Duration::from_secs(5);

pub async fn phase<T, E>(
    run: &batter_sqlx::test_support::FixtureRun<T, E>,
    expected: CleanupPhase,
) {
    tokio::time::timeout(BOUND, async {
        while !run
            .cleanup_progress()
            .iter()
            .any(|item| item.phase == expected)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cleanup phase must be witnessed within test bound");
}

pub async fn detached_sessions() -> ProbeResult {
    for native in [true, false] {
        detached_case(native).await?;
    }
    Ok(())
}

async fn detached_case(native: bool) -> ProbeResult {
    let harness = super::harness().await?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let session_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let control = SessionObserver::new(session_pool.clone(), Duration::from_secs(2))?;
    let (send, receive) = oneshot::channel();
    let run = FixtureSuite::new(harness)
        .with_session_observer(control.clone())
        .start(move |scope| Box::pin(detached_body(scope, native, send)));
    observe_detached(admin, session_pool, control, run, receive).await
}

async fn observe_detached(
    admin: PgPool,
    session_pool: PgPool,
    control: SessionObserver,
    mut run: batter_sqlx::test_support::FixtureRun<(), FixtureError>,
    receive: oneshot::Receiver<DetachedResources>,
) -> ProbeResult {
    let (mut blocker, blocker_pid, pid, operation, pools, name, sibling, cleanup) =
        tokio::time::timeout(BOUND, receive).await??;
    let mut observation = admin.acquire().await?;
    let blocked = observe_blocked(&mut observation, pid, blocker_pid, BOUND).await;
    drop(observation);
    let _ = cleanup.send(());
    operation.abort();
    let joined = operation.await;
    // Positive local-close witness, even though retired backend is still blocked.
    close_pair(&pools).await?;
    phase(&run, CleanupPhase::AwaitingRetry).await;
    let pending = run
        .wait_for(Duration::from_millis(30))
        .await
        .expect("fixture driver joined")
        .is_none();
    let retained = present(&admin, &name).await;
    let sessions: Vec<i32> =
        sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname = $1")
            .bind(&name)
            .fetch_all(&admin)
            .await?;
    let independent = fixture_run::wait_absent(&admin, &sibling).await;
    // Release and close before interpreting assertions; cleanup retries the same lease.
    let released = sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut blocker)
        .await;
    let closed = sqlx::Connection::close(blocker).await;
    // Backend exit is separately acknowledged before starting the bounded retry.
    // The retry continues on its dedicated pool, never the diagnostic pool.
    let sessions_absent = wait_sessions_absent(&admin, &name).await;
    control.retry_with(session_pool.clone());
    let report =
        super::fixture_completion::ObservedRun::new(run, control, session_pool, admin.clone())
            .finish(BOUND)
            .await;
    let absent = if report.is_ok() {
        let absent = fixture_run::absent(&admin, &name).await;
        admin.close().await;
        absent
    } else {
        Ok(())
    };
    let observations = combine_observations([
        blocked.map_err(Into::into),
        released.map(|_| ()).map_err(Into::into),
        closed.map_err(Into::into),
        independent,
        sessions_absent,
        absent,
    ]);
    let report = batter_test_support::finish(report, observations)
        .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
    assert!(joined.is_err_and(|error| error.is_cancelled()));
    assert_session_retention(pending, retained?, &sessions, [pid, blocker_pid]);
    assert_detached_report(&report, &name);
    Ok(())
}

pub async fn observer_failure() -> ProbeResult {
    let harness = super::harness().await?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let broken = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    broken.close().await;
    let control = SessionObserver::new(broken.clone(), BOUND)?;
    let mut run = FixtureSuite::new(harness)
        .with_session_observer(control.clone())
        .start(|scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                let db = scope.empty(&plan).await?;
                sqlx::query("SELECT 1 / 0")
                    .execute(&db.pools()[0])
                    .await
                    .map_err(FixtureError::Observe)?;
                Ok::<_, FixtureError>(())
            })
        });
    phase(&run, CleanupPhase::AwaitingRetry).await;
    let snapshot = run.cleanup_progress();
    let failure = snapshot[0].observation_failures[0].clone();
    let retained = present(&admin, &snapshot[0].database_name).await;
    let cancelled = tokio::time::timeout(Duration::from_millis(30), run.wait())
        .await
        .is_err();
    let wrong = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            admin
                .connect_options()
                .as_ref()
                .clone()
                .database(&snapshot[0].database_name),
        )
        .await?;
    let mut completion =
        super::fixture_completion::ObservedRun::new(run, control, broken, admin.clone());
    completion.retry_with(wrong);
    tokio::time::timeout(BOUND, async {
        while completion.run.cleanup_progress()[0]
            .observation_failures
            .len()
            < 2
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let still_retained = present(&admin, &snapshot[0].database_name).await;
    let error = completion
        .finish(Duration::from_millis(30))
        .await
        .expect_err("wrong-target observation retains completion ownership");
    let mut completion = *error
        .0
        .downcast::<super::fixture_completion::ObservedRun<(), FixtureError>>()
        .unwrap();
    // A replacement does not close a pool's real backend. Repair that resource
    // explicitly through the pending owner's retained pool; no external clone
    // survives the replacement call. Then resume the same owner.
    tokio::time::timeout(BOUND, completion.observer_pools()[1].close()).await?;
    wait_sessions_absent(&admin, &snapshot[0].database_name).await?;
    completion.retry_with(super::fixture_observation::admin().await?);
    let report = completion.finish(BOUND).await?;
    let absent = fixture_run::absent(&admin, &snapshot[0].database_name).await;
    admin.close().await;
    absent?;
    assert!(cancelled && retained? && still_retained?);
    assert_observer_report(&report, &failure);
    Ok(())
}

type DetachedResources = (
    sqlx::PgConnection,
    i32,
    i32,
    tokio::task::JoinHandle<Result<(), sqlx::Error>>,
    Vec<PgPool>,
    String,
    String,
    oneshot::Sender<()>,
);

async fn detached_body(
    scope: &mut batter_sqlx::test_support::FixtureScope,
    native: bool,
    send: oneshot::Sender<DetachedResources>,
) -> Result<(), FixtureError> {
    // Two separately accounted application connections. The independent admin
    // observer has its own one-slot suite budget, outside the closing pools.
    let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1); 2], 0)?;
    let db = scope.empty(&plan).await?;
    let sibling = scope.empty(&plan).await?;
    let mut blocker = db.pools()[1]
        .acquire()
        .await
        .map_err(FixtureError::Observe)?
        .detach();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut blocker)
        .await
        .map_err(FixtureError::Observe)?;
    sqlx::query("SELECT pg_advisory_lock(73492)")
        .execute(&mut blocker)
        .await
        .map_err(FixtureError::Observe)?;
    let (pid_send, pid_receive) = oneshot::channel();
    let pool = db.pools()[0].clone();
    let operation = tokio::spawn(async move {
        if native {
            let mut connection = pool.acquire().await?.detach();
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut connection)
                .await?;
            pid_send.send(pid).expect("pid observer alive");
            sqlx::query("SELECT pg_advisory_lock(73492)")
                .execute(&mut connection)
                .await?;
        } else {
            let context = batter::operation::OperationContext::new(Duration::from_secs(10))
                .expect("positive budget");
            let mut connection = batter_sqlx::PgLease::acquire(&pool, &context)
                .await
                .expect("adapter acquire");
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(connection.connection())
                .await?;
            pid_send.send(pid).expect("pid observer alive");
            sqlx::query("SELECT pg_advisory_lock(73492)")
                .execute(connection.connection())
                .await?;
        }
        Ok::<_, sqlx::Error>(())
    });
    let pid = pid_receive.await.expect("operation publishes pid");
    let (cleanup, proceed) = oneshot::channel();
    send.send((
        blocker,
        blocker_pid,
        pid,
        operation,
        db.pools().to_vec(),
        db.database_name().to_owned(),
        sibling.database_name().to_owned(),
        cleanup,
    ))
    .unwrap_or_else(|_| panic!("observer alive"));
    proceed.await.expect("blocking observation acknowledged");
    Ok::<_, FixtureError>(())
}

fn assert_observer_report(
    report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>,
    failure: &std::sync::Arc<FixtureError>,
) {
    assert_eq!(report.databases[0].observation_failures.len(), 2);
    assert!(matches!(
        *report.databases[0].observation_failures[1],
        FixtureError::ObservationTarget
    ));
    assert!(matches!(
        **failure,
        FixtureError::Observe(sqlx::Error::PoolClosed)
    ));
    assert!(std::sync::Arc::ptr_eq(
        failure,
        &report.databases[0].observation_failures[0]
    ));
    assert!(
        matches!(&report.body, Err(batter_sqlx::test_support::BodyFailure::Returned(FixtureError::Observe(sqlx::Error::Database(error)))) if error.code().as_deref() == Some("22012"))
    );
    assert!(report.databases[0].result.is_ok() && report.drain.is_ok() && !report.is_ok());
}

fn assert_detached_report(
    report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>,
    name: &str,
) {
    assert!(report.body.is_ok() && report.drain.is_ok());
    assert!(report.databases.iter().all(|db| db.result.is_ok()));
    assert!(
        !report.is_ok(),
        "recovered observation timeout remains a failure"
    );
    let db = report
        .databases
        .iter()
        .find(|db| db.database_name == name)
        .unwrap();
    assert_eq!(db.observation_failures.len(), 1);
    assert!(matches!(
        *db.observation_failures[0],
        FixtureError::ObservationTimeout
    ));
}

async fn close_pair(pools: &[PgPool]) -> ProbeResult {
    tokio::time::timeout(BOUND, async {
        tokio::join!(pools[0].close(), pools[1].close());
    })
    .await?;
    Ok(())
}

fn assert_session_retention(pending: bool, retained: bool, sessions: &[i32], pids: [i32; 2]) {
    assert!(pending);
    assert!(retained);
    for pid in pids {
        assert!(sessions.contains(&pid));
    }
}

pub(super) async fn wait_sessions_absent(admin: &PgPool, name: &str) -> ProbeResult {
    tokio::time::timeout(BOUND, async {
        loop {
            let active: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = $1)",
            )
            .bind(name)
            .fetch_one(admin)
            .await?;
            if !active {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

fn combine_observations(results: [ProbeResult; 6]) -> Result<(), fixture_run::ProbeError> {
    let mut combined = Ok(());
    for result in results {
        combined =
            batter_test_support::finish(combined, result.map_err(fixture_run::ProbeError::new))
                .map_err(|error| fixture_run::ProbeError::new(Box::new(error)));
    }
    combined
}
