use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter_sqlx::test_support::{
    AcquisitionFailure, BodyFailure, ConnectionPlan, FixtureError, FixtureScope, FixtureSuite,
    template_spec,
};
use postgres_test_harness::{
    DatabaseTemplate, HarnessConfig, PostgresHarness, cleanup_stale_databases,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;

use super::{ProbeResult, fixture_run};

const PROJECT: &str = "batter_creation";

#[derive(Clone, Copy)]
enum Creation {
    Empty,
    Clone,
    Template,
}

async fn harness(
    budget: usize,
) -> Result<PostgresHarness, Box<dyn std::error::Error + Send + Sync>> {
    let (url, _) = super::live_endpoint::from_process()?;
    Ok(PostgresHarness::start(
        HarnessConfig::new(PROJECT)?
            .with_admin_database_url(url.expose_secret())
            .with_cleanup_on_start(false)
            .with_connection_budget(budget)?
            .with_connections_per_database(4)?
            .with_operation_timeout(Duration::from_secs(20))?,
    )
    .await?)
}

async fn inventory(observer: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT datname::text FROM pg_database WHERE starts_with(datname::text, $1) ORDER BY datname")
        .bind(format!("pgh_{PROJECT}_")).fetch_all(observer).await
}

async fn create(
    scope: &mut FixtureScope,
    mode: Creation,
    template: Option<DatabaseTemplate>,
) -> Result<(), FixtureError> {
    let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    match mode {
        Creation::Empty => {
            scope.empty(&plan).await?;
        }
        Creation::Clone => {
            scope
                .from_template(&template.expect("clone template"), &plan)
                .await?;
        }
        Creation::Template => {
            scope
                .template(template_spec(&[], "cancelled-template-v1"), |_| async {
                    Ok(())
                })
                .await?;
        }
    }
    Ok(())
}

async fn blocked_creation(
    observer: &PgPool,
    blocker: i32,
    before: &[String],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND query LIKE 'COMMENT ON DATABASE %' AND strpos(query, $2) > 0)")
                .bind(blocker).bind(format!("pgh_{PROJECT}_")).fetch_one(observer).await?;
            let names = inventory(observer).await?;
            if blocked {
                let created: Vec<_> = names.into_iter().filter(|name| !before.contains(name)).collect();
                if let [name] = created.as_slice() { return Ok::<_, sqlx::Error>(name.clone()); }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await?.map_err(Into::into)
}

pub async fn cancelled_creation() -> ProbeResult {
    for mode in [Creation::Empty, Creation::Clone, Creation::Template] {
        cancelled(mode).await?;
    }
    Ok(())
}

async fn cancelled(mode: Creation) -> ProbeResult {
    let harness = harness(8).await?;
    let url = harness.admin_database_url().to_owned();
    // Reclaim only tagged, inactive resources through upstream ownership checks.
    // Stable inputs are reused; no random template identity is introduced.
    cleanup_stale_databases(&url, PROJECT, Duration::ZERO).await?;
    let observer = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    let (suite, template) = prepare_suite(harness.clone(), mode).await?;
    let before = inventory(&observer).await?;
    let (lock, blocker) = catalog_lock(&observer).await?;
    let (cancel_sender, cancel_receiver) = oneshot::channel();
    let (body_sender, body_receiver) = oneshot::channel();
    let mut run = start_creation(suite, mode, template, cancel_receiver, body_sender);
    let observed = blocked_creation(&observer, blocker, &before).await;
    let _ = cancel_sender.send(());
    let body_exited = tokio::time::timeout(Duration::from_secs(2), body_receiver).await;
    let pending = tokio::time::timeout(Duration::from_millis(100), run.wait())
        .await
        .is_err();
    let unlocked = lock.rollback().await;
    let report = run.into_report().await?;
    let before_reclaim = inventory(&observer).await;
    drop(harness);
    let reclaimed = cleanup_stale_databases(&url, PROJECT, Duration::ZERO).await;
    let names = inventory(&observer).await;
    observer.close().await;
    let name = observed?;
    unlocked?;
    reclaimed?;
    assert!(body_exited?.is_ok());
    assert!(
        pending,
        "report must wait for native creation after its waiter is cancelled"
    );
    assert_cancelled_report(mode, &report, &name, &before_reclaim?);
    assert!(names?.is_empty());
    Ok(())
}

async fn catalog_lock(
    observer: &PgPool,
) -> Result<(sqlx::Transaction<'static, sqlx::Postgres>, i32), sqlx::Error> {
    let mut lock = observer.begin().await?;
    sqlx::query("LOCK TABLE pg_catalog.pg_shdescription IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await?;
    Ok((lock, blocker))
}

async fn prepare_suite(
    harness: PostgresHarness,
    mode: Creation,
) -> Result<(FixtureSuite, Option<DatabaseTemplate>), FixtureError> {
    let mut suite = FixtureSuite::new(harness);
    let template = if matches!(mode, Creation::Clone) {
        Some(
            suite
                .template(template_spec(&[], "clone-source-v1"), |_| async { Ok(()) })
                .await?,
        )
    } else {
        None
    };
    Ok((suite, template))
}

fn start_creation(
    suite: FixtureSuite,
    mode: Creation,
    template: Option<DatabaseTemplate>,
    cancel_receiver: oneshot::Receiver<()>,
    body_sender: oneshot::Sender<()>,
) -> batter_sqlx::test_support::FixtureRun<(), FixtureError> {
    suite.start(move |scope| {
        Box::pin(async move {
            let result = tokio::select! {
                result = create(scope, mode, template) => result,
                _ = cancel_receiver => Err(FixtureError::ObservationTimeout),
            };
            let _ = body_sender.send(());
            result
        })
    })
}

fn assert_cancelled_report(
    mode: Creation,
    report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>,
    name: &str,
    before_reclaim: &[String],
) {
    assert!(matches!(
        report.body,
        Err(BodyFailure::Returned(FixtureError::ObservationTimeout))
    ));
    assert_eq!(report.acquisitions.len(), 1);
    assert!(report.acquisitions.iter().all(Result::is_ok) && report.drain.is_ok());
    if matches!(mode, Creation::Template) {
        assert!(before_reclaim.iter().any(|existing| existing == name));
        assert!(report.databases.is_empty());
    } else {
        assert!(!before_reclaim.iter().any(|existing| existing == name));
        assert_eq!(report.databases.len(), 1);
        assert_eq!(report.databases[0].database_name, name);
        assert!(report.databases[0].result.is_ok());
    }
}

pub async fn abandoned_failure() -> ProbeResult {
    for panic in [false, true] {
        failed_template(panic).await?;
    }
    Ok(())
}

async fn failed_template(panic: bool) -> ProbeResult {
    let harness = harness(8).await?;
    let url = harness.admin_database_url().to_owned();
    cleanup_stale_databases(&url, PROJECT, Duration::ZERO).await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let (ready_sender, ready_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let (cancel_sender, cancel_receiver) = oneshot::channel();
    let (body_sender, body_receiver) = oneshot::channel();
    let mut run = FixtureSuite::new(harness.clone()).start(move |scope| Box::pin(async move {
        let result = tokio::select! {
            result = scope.template(template_spec(&[], "failed-template-v1"), move |_| async move {
                let _ = ready_sender.send(());
                let _ = release_receiver.await;
                assert!(!panic, "controlled template initializer panic");
                Err(std::io::Error::other("controlled initializer failure").into())
            }) => result.map(|_| ()),
            _ = cancel_receiver => Err(FixtureError::ObservationTimeout),
        };
        let _ = body_sender.send(());
        result
    }));
    let ready = tokio::time::timeout(Duration::from_secs(10), ready_receiver).await;
    let _ = cancel_sender.send(());
    let body_exited = tokio::time::timeout(Duration::from_secs(2), body_receiver).await;
    let pending = tokio::time::timeout(Duration::from_millis(100), run.wait())
        .await
        .is_err();
    let _ = release_sender.send(());
    let report = run.into_report().await?;
    let before_reclaim = inventory(&observer).await;
    drop(harness);
    let reclaimed = cleanup_stale_databases(&url, PROJECT, Duration::ZERO).await;
    let names = inventory(&observer).await;
    observer.close().await;
    assert!(ready?.is_ok() && body_exited?.is_ok() && pending);
    assert!(
        before_reclaim?.is_empty(),
        "producer cleanup must finish before fallback recovery"
    );
    reclaimed?;
    assert!(names?.is_empty());
    assert_template_failure(&report, panic);
    Ok(())
}

fn assert_template_failure(
    report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>,
    panic: bool,
) {
    assert!(matches!(
        report.body,
        Err(BodyFailure::Returned(FixtureError::ObservationTimeout))
    ));
    assert!(!report.is_ok());
    assert_eq!(report.acquisitions.len(), 1);
    let Err(AcquisitionFailure::Native(error)) = &report.acquisitions[0] else {
        panic!("native producer error required")
    };
    let postgres_test_harness::Error::TemplateInitializer { source } = error.as_ref() else {
        panic!("initializer cause required")
    };
    if panic {
        assert!(
            source
                .downcast_ref::<tokio::task::JoinError>()
                .is_some_and(tokio::task::JoinError::is_panic)
        );
    } else {
        assert_eq!(
            source.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Other
        );
    }
    assert!(report.databases.is_empty() && report.drain.is_ok());
}

pub async fn shared_admission() -> ProbeResult {
    let harness = harness(4).await?;
    let suite = FixtureSuite::new(harness.clone());
    let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    let held = suite.empty(&plan).await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (started_sender, started_receiver) = oneshot::channel();
    let mut run = FixtureSuite::new(harness.clone()).start(move |scope| {
        Box::pin(async move {
            let plan = ConnectionPlan::new(
                vec![
                    PgPoolOptions::new()
                        .max_connections(1)
                        .after_connect(move |_, _| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Box::pin(async { Ok(()) })
                        }),
                ],
                0,
            )?;
            let _ = started_sender.send(());
            let db = scope.empty(&plan).await?;
            Ok::<_, FixtureError>(db.database_name().to_owned())
        })
    });
    started_receiver.await?;
    let pending = tokio::time::timeout(Duration::from_millis(100), run.wait())
        .await
        .is_err();
    let calls_before_release = observed.load(Ordering::SeqCst);
    let held_name = held.database_name().to_owned();
    let released = held.finish(Ok::<_, FixtureError>(())).await;
    let report = run.into_report().await?;
    let held_absence = fixture_run::absent(&observer, &held_name).await;
    let observation = async {
        for db in &report.databases {
            fixture_run::absent(&observer, &db.database_name).await?;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    // The external owner still controls a usable harness after the run completes.
    let later = suite
        .empty(&ConnectionPlan::new(
            vec![PgPoolOptions::new().max_connections(1)],
            0,
        )?)
        .await?;
    let later_cleanup = later.finish(Ok::<_, FixtureError>(())).await;
    let drain = suite.drain().await;
    observer.close().await;
    released?;
    later_cleanup?;
    drain?;
    held_absence?;
    observation?;
    assert!(pending && calls_before_release == 0);
    assert!(observed.load(Ordering::SeqCst) > 0 && report.is_ok());
    assert_eq!(report.databases.len(), 1);
    Ok(())
}
