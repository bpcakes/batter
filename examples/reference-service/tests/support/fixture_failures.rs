use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter_sqlx::test_support::{
    BodyFailure, ConnectionPlan, FixtureBody, FixtureError, FixtureReport, FixtureScope,
    FixtureSuite,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;

use super::{
    ProbeResult,
    fixture_run::{self, ProbeError},
};

async fn observed_failure<F>(
    body: F,
) -> Result<FixtureReport<(), FixtureError>, Box<dyn std::error::Error + Send + Sync>>
where
    F: for<'a> FnOnce(&'a mut FixtureScope) -> FixtureBody<'a, (), FixtureError> + Send + 'static,
{
    let harness = super::harness().await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let report = FixtureSuite::new(harness).start(body).into_report().await;
    let observation = async {
        if let Ok(report) = &report {
            for database in &report.databases {
                fixture_run::absent(&observer, &database.database_name).await?;
            }
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    observer.close().await;
    // Return the actual report for identity checks only after catalog observation.
    let report = report?;
    assert!(
        observation.is_ok(),
        "independent catalog observation failed"
    );
    assert!(
        report
            .databases
            .iter()
            .all(|database| database.result.is_ok())
    );
    assert!(report.drain.is_ok() && report.acquisitions.iter().all(Result::is_ok));
    Ok(report)
}

fn failing_pool(calls: Arc<AtomicUsize>) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(150))
        .after_connect(move |connection, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                // Real PostgreSQL error; SQLx retries initialization until acquisition expires.
                sqlx::query("SELECT 1 / 0").execute(connection).await?;
                Ok(())
            })
        })
}

pub async fn partial_acquisition_and_panic() -> ProbeResult {
    low_level_partial().await?;
    for sibling in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let report = observed_failure(move |scope| {
            Box::pin(async move {
                let valid = PgPoolOptions::new().max_connections(1);
                let failing = ConnectionPlan::new(vec![valid.clone(), failing_pool(calls)], 0)?;
                if sibling {
                    let valid = ConnectionPlan::new(vec![valid], 0)?;
                    let (left, right) = tokio::join!(scope.empty(&valid), scope.empty(&failing));
                    let _left = left?;
                    let _right = right?;
                } else {
                    scope.empty(&failing).await?;
                }
                Ok(())
            })
        })
        .await?;
        assert!(observed.load(Ordering::SeqCst) > 0);
        assert_eq!(report.databases.len(), if sibling { 2 } else { 1 });
        assert!(matches!(
            report.body,
            Err(BodyFailure::Returned(FixtureError::PoolAcquire(
                sqlx::Error::PoolTimedOut
            )))
        ));
    }
    for during_connect in [false, true] {
        let report =
            observed_failure(move |scope| {
                Box::pin(async move {
                    let mut pools = vec![PgPoolOptions::new().max_connections(1)];
                    if during_connect {
                        pools.push(PgPoolOptions::new().max_connections(1).after_connect(
                            |_, _| {
                                Box::pin(async {
                                    panic!("controlled fixture initialization panic")
                                })
                            },
                        ));
                    }
                    let plan = ConnectionPlan::new(pools, 0)?;
                    scope.empty(&plan).await?;
                    panic!("controlled fixture body panic");
                })
            })
            .await?;
        assert_eq!(report.databases.len(), 1);
        assert!(matches!(report.body, Err(BodyFailure::Task(error)) if error.is_panic()));
    }
    Ok(())
}

async fn present(observer: &PgPool, name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(observer)
        .await
}

async fn wait_absent(observer: &PgPool, name: &str) -> ProbeResult {
    tokio::time::timeout(Duration::from_secs(10), async {
        while present(observer, name).await? {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, sqlx::Error>(())
    })
    .await??;
    Ok(())
}

pub async fn close_order_and_resumable_wait() -> ProbeResult {
    let harness = super::harness().await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let (sender, receiver) = oneshot::channel();
    let mut run = FixtureSuite::new(harness).start(move |scope| {
        Box::pin(async move {
            let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1); 2], 0)?;
            let blocked = scope.empty(&plan).await?;
            let independent = scope.empty(&plan).await?;
            assert!(matches!(
                scope.empty(&plan).await,
                Err(FixtureError::BatchCapacity)
            ));
            let connection = blocked.pools()[0]
                .acquire()
                .await
                .map_err(FixtureError::Observe)?;
            sender
                .send((
                    connection,
                    blocked.pools().to_vec(),
                    blocked.database_name().to_owned(),
                    independent.database_name().to_owned(),
                ))
                .unwrap_or_else(|_| panic!("test observer must remain alive"));
            Ok::<_, FixtureError>(44)
        })
    });
    let (connection, pools, name, independent) = receiver.await?;
    pools[0].close_event().await;
    pools[1].close_event().await; // Independent pool closes must also start.
    let pending = tokio::time::timeout(Duration::from_millis(40), run.wait())
        .await
        .is_err();
    let retained = present(&observer, &name).await;
    let independent_cleaned = wait_absent(&observer, &independent).await;
    drop(connection);
    let report = run.wait().await;
    let success = report.is_ok_and(|report| report.is_ok() && matches!(report.body, Ok(44)));
    let repeated = run.wait().await.is_ok_and(|report| report.is_ok());
    let absence = fixture_run::absent(&observer, &name).await;
    observer.close().await;
    assert!(pending && retained? && success && repeated);
    independent_cleaned?;
    absence?;
    assert!(pools.iter().all(PgPool::is_closed));
    Ok(())
}

pub async fn foreign_template_rejected() -> ProbeResult {
    let harness = super::harness().await?;
    let mut foreign = FixtureSuite::new(harness.clone());
    let spec = batter_sqlx::test_support::template_spec(&[], "foreign-template-control-v1");
    let template = foreign.template(spec, |_| async { Ok(()) }).await?;
    let report = FixtureSuite::new(harness)
        .start(move |scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.from_template(&template, &plan).await?;
                Ok::<_, FixtureError>(())
            })
        })
        .into_report()
        .await?;
    assert!(matches!(
        report.body,
        Err(BodyFailure::Returned(FixtureError::UnknownTemplate))
    ));
    assert!(report.databases.is_empty());
    assert!(report.drain.is_ok() && report.acquisitions.iter().all(Result::is_ok));
    Ok(())
}

pub async fn observer_failure_preserves_body() -> ProbeResult {
    let result = fixture_run::run(|scope, observer| {
        Box::pin(async move {
            let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
            let fixture = scope.empty(&plan).await?;
            let body = sqlx::query("SELECT 1 / 0")
                .execute(&fixture.pools()[0])
                .await;
            observer.close().await; // Force the subsequent independent catalog read to fail.
            body?;
            Ok(())
        })
    })
    .await;
    let failure = result.expect_err("both body and observer must fail");
    let combined = failure
        .downcast_ref::<batter_test_support::TestFailure<ProbeError, ProbeError>>()
        .expect("retain both branches");
    let batter_test_support::TestFailure::Both { body, cleanup } = combined else {
        panic!("both branches required");
    };
    let report = body
        .0
        .downcast_ref::<FixtureReport<(), ProbeError>>()
        .expect("actual fixture report retained");
    assert!(
        report
            .databases
            .iter()
            .all(|database| database.result.is_ok())
    );
    assert!(report.drain.is_ok() && report.acquisitions.iter().all(Result::is_ok));
    let Err(BodyFailure::Returned(error)) = &report.body else {
        panic!("body error required");
    };
    assert!(
        matches!(error.0.downcast_ref::<sqlx::Error>(), Some(sqlx::Error::Database(error)) if error.code().as_deref() == Some("22012"))
    );
    assert!(matches!(
        cleanup.0.downcast_ref::<sqlx::Error>(),
        Some(sqlx::Error::PoolClosed)
    ));
    Ok(())
}

async fn low_level_partial() -> ProbeResult {
    let harness = super::harness().await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let suite = FixtureSuite::new(harness.clone());
    let names = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_names = names.clone();
    let first = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _| {
            let names = names.clone();
            Box::pin(async move {
                let name: String = sqlx::query_scalar("SELECT current_database()")
                    .fetch_one(connection)
                    .await?;
                names.lock().expect("test name list").push(name);
                Ok(())
            })
        });
    let calls = Arc::new(AtomicUsize::new(0));
    let plan = ConnectionPlan::new(vec![first, failing_pool(calls)], 0)?;
    let result = suite.empty(&plan).await;
    let names = observed_names.lock().expect("test name list").clone();
    let observation = async {
        for name in &names {
            fixture_run::absent(&observer, name).await?;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    observer.close().await;
    let drained = suite.drain().await;
    let shutdown = harness.shutdown().await;
    assert!(!names.is_empty());
    assert!(matches!(
        result,
        Err(FixtureError::Connect {
            source: sqlx::Error::PoolTimedOut,
            cleanup: None
        })
    ));
    observation?;
    drained?;
    shutdown?;
    Ok(())
}

pub async fn pending_pool_error() -> ProbeResult {
    let url = std::env::var("POSTGRES_TEST_ADMIN_URL")?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let body_observer = observer.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let report = observed_failure(move |scope| {
        Box::pin(async move {
            let before: Vec<String> = sqlx::query_scalar("SELECT datname::text FROM pg_database")
                .fetch_all(&body_observer)
                .await
                .map_err(FixtureError::Observe)?;
            let plan = ConnectionPlan::new(
                vec![PgPoolOptions::new().max_connections(1), failing_pool(calls)],
                0,
            )?;
            let error = scope
                .empty(&plan)
                .await
                .err()
                .expect("second pool must fail");
            let after: Vec<String> = sqlx::query_scalar("SELECT datname::text FROM pg_database")
                .fetch_all(&body_observer)
                .await
                .map_err(FixtureError::Observe)?;
            let created: Vec<_> = after
                .into_iter()
                .filter(|name| !before.contains(name))
                .collect();
            assert_eq!(
                created.len(),
                1,
                "failed acquisition remains registered until body exit"
            );
            let connected: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = $1)",
            )
            .bind(&created[0])
            .fetch_one(&body_observer)
            .await
            .map_err(FixtureError::Observe)?;
            assert!(
                connected,
                "the first opened pool remains registered for ordered cleanup"
            );
            assert!(matches!(
                error,
                FixtureError::PoolAcquire(sqlx::Error::PoolTimedOut)
            ));
            Err(error)
        })
    })
    .await;
    observer.close().await;
    let report = report?;
    assert!(observed.load(Ordering::SeqCst) > 0);
    assert_eq!(report.databases.len(), 1);
    assert!(matches!(
        report.body,
        Err(BodyFailure::Returned(FixtureError::PoolAcquire(
            sqlx::Error::PoolTimedOut
        )))
    ));
    Ok(())
}
