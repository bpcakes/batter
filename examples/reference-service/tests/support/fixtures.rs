use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter_sqlx::test_support::{
    BodyFailure, ConnectionPlan, FixtureDatabase, FixtureError, FixtureScope, FixtureSuite,
    MigrationBundle, MigrationInput, observe_blocked, template_spec,
};
use sqlx::{
    PgPool,
    migrate::{Migration, MigrationType, Migrator},
    postgres::PgPoolOptions,
};

use super::{
    ProbeResult,
    fixture_run::{self, ProbeError},
};

pub const SQL: &str =
    "CREATE TABLE fixture_probe (id bigint PRIMARY KEY, value bigint NOT NULL DEFAULT 10)";
const CHANGED_SQL: &str =
    "CREATE TABLE fixture_probe (id bigint PRIMARY KEY, value bigint NOT NULL DEFAULT 20)";

fn spec(sql: &str) -> postgres_test_harness::TemplateSpec {
    template_spec(
        &[MigrationBundle {
            identity: "probe",
            migrations: &[MigrationInput {
                identity: "1",
                kind: "simple",
                sql: sql.as_bytes(),
            }],
        }],
        "probe-schema-v1",
    )
}

async fn initialize(url: String, sql: &'static str, count: Arc<AtomicUsize>) -> ProbeResult {
    count.fetch_add(1, Ordering::SeqCst);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let migrator = Migrator {
        migrations: Cow::Owned(vec![Migration::new(
            1,
            Cow::Borrowed("probe"),
            MigrationType::Simple,
            sqlx::SqlStr::from_static(sql),
            false,
        )]),
        ..Migrator::DEFAULT
    };
    let result = migrator.run(&pool).await;
    pool.close().await;
    result?;
    Ok(())
}

pub async fn templates() -> ProbeResult {
    let names = fixture_run::run(|scope, _observer| {
        Box::pin(async move {
            let (template, reused, changed) = retained_templates(scope).await?;
            let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
            let (left, right) = tokio::join!(
                scope.from_template(&template, &plan),
                scope.from_template(&reused, &plan)
            );
            // The driver owns both acquisitions before either result is interpreted.
            let left = left?;
            let right = right?;
            assert_ne!(left.database_name(), right.database_name());
            isolated_writes(&left, &right).await?;
            Ok((
                template.database_name().to_owned(),
                changed.database_name().to_owned(),
            ))
        })
    })
    .await?;
    // A fresh suite must reuse persisted identities, not just an in-memory handle.
    fixture_run::run(move |scope, _observer| {
        Box::pin(async move {
            let template = scope
                .template(spec(SQL), |_| async {
                    Err("persisted template unexpectedly initialized".into())
                })
                .await?;
            let changed = scope
                .template(spec(CHANGED_SQL), |_| async {
                    Err("persisted changed template unexpectedly initialized".into())
                })
                .await?;
            assert_eq!(
                names,
                (
                    template.database_name().to_owned(),
                    changed.database_name().to_owned()
                )
            );
            let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
            let clone = scope.from_template(&changed, &plan).await?;
            assert_eq!(
                sqlx::query("INSERT INTO fixture_probe(id) VALUES (1)")
                    .execute(&clone.pools()[0])
                    .await?
                    .rows_affected(),
                1
            );
            let value: i64 = sqlx::query_scalar("SELECT value FROM fixture_probe WHERE id = 1")
                .fetch_one(&clone.pools()[0])
                .await?;
            assert_eq!(value, 20);
            Ok(())
        })
    })
    .await
}

async fn retained_templates(
    scope: &mut FixtureScope,
) -> Result<
    (
        postgres_test_harness::DatabaseTemplate,
        postgres_test_harness::DatabaseTemplate,
        postgres_test_harness::DatabaseTemplate,
    ),
    FixtureError,
> {
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let template = scope
        .template(spec(SQL), |url| initialize(url, SQL, calls))
        .await?;
    let first_calls = count.load(Ordering::SeqCst);
    assert!(first_calls <= 1); // Zero is a persisted cache hit, one is cold initialization.
    let calls = count.clone();
    let reused = scope
        .template(spec(SQL), |url| initialize(url, SQL, calls))
        .await?;
    assert_eq!(template.database_name(), reused.database_name());
    assert_eq!(count.load(Ordering::SeqCst), first_calls);
    let calls = count.clone();
    let changed = scope
        .template(spec(CHANGED_SQL), |url| initialize(url, CHANGED_SQL, calls))
        .await?;
    assert_ne!(template.database_name(), changed.database_name());
    assert!(count.load(Ordering::SeqCst) - first_calls <= 1);
    Ok((template, reused, changed))
}

async fn isolated_writes(left: &FixtureDatabase, right: &FixtureDatabase) -> ProbeResult {
    let write = |pool: PgPool, value: i64| async move {
        assert_eq!(
            sqlx::query("INSERT INTO fixture_probe(id, value) VALUES (1, $1)")
                .bind(value)
                .execute(&pool)
                .await?
                .rows_affected(),
            1
        );
        Ok::<_, sqlx::Error>(())
    };
    let (a, b) = tokio::join!(
        write(left.pools()[0].clone(), 31),
        write(right.pools()[0].clone(), 47)
    );
    batter_test_support::finish(a, b)?;
    for (fixture, value) in [(left, 31_i64), (right, 47_i64)] {
        let identity: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&fixture.pools()[0])
            .await?;
        assert_eq!(identity, fixture.database_name());
        let rows: Vec<(i64, i64)> = sqlx::query_as("SELECT id, value FROM fixture_probe")
            .fetch_all(&fixture.pools()[0])
            .await?;
        assert_eq!(rows, [(1, value)]);
    }
    Ok(())
}

pub async fn lock_operation() -> ProbeResult {
    fixture_run::run(|scope, _catalog| {
        Box::pin(async move {
            let plan = ConnectionPlan::new(
                (0..3)
                    .map(|_| PgPoolOptions::new().max_connections(1))
                    .collect(),
                0,
            )?;
            let fixture = scope.empty(&plan).await?;
            let pool = &fixture.pools()[0];
            sqlx::query(SQL).execute(pool).await?;
            let mut blocker = fixture.pools()[1].acquire().await?;
            let mut observer = fixture.pools()[2].acquire().await?;
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await?;
            sqlx::query("SELECT pg_advisory_lock(73491)")
                .execute(&mut *blocker)
                .await?;
            let mut connection = pool.acquire().await?;
            let waiter_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await?;
            let operation = tokio::spawn(async move {
                use sqlx::Connection;
                let mut tx = connection.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock(73491)")
                    .execute(&mut *tx)
                    .await?;
                let affected = sqlx::query("INSERT INTO fixture_probe(id, value) VALUES (1, 73)")
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                tx.commit().await?;
                Ok::<_, sqlx::Error>(affected)
            });
            let observed = observe_blocked(
                &mut observer,
                waiter_pid,
                blocker_pid,
                Duration::from_secs(10),
            )
            .await;
            let pending = !operation.is_finished();
            // An always-successful observer would fail this wrong-identity control.
            let wrong = observe_blocked(
                &mut observer,
                waiter_pid,
                waiter_pid,
                Duration::from_millis(40),
            )
            .await;
            let released = sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock(73491)")
                .fetch_one(&mut *blocker)
                .await;
            let blocker_closed = blocker.close().await;
            let completed = operation.await;
            drop(observer);
            let conditions = batter_test_support::finish(observed, blocker_closed);
            let completed = completed
                .map_err(|error| ProbeError::new(Box::new(error)))
                .and_then(|result| result.map_err(|error| ProbeError::new(Box::new(error))));
            let completed = batter_test_support::finish(completed, conditions);
            let unlocked = released.as_ref().copied().unwrap_or(false);
            let affected = batter_test_support::finish(completed, released.map(|_| ()))?;
            assert!(unlocked);
            assert!(matches!(wrong, Err(FixtureError::ObservationTimeout)));
            assert!(pending);
            assert_eq!(affected, 1);
            let rows: Vec<(i64, i64)> = sqlx::query_as("SELECT id, value FROM fixture_probe")
                .fetch_all(pool)
                .await?;
            assert_eq!(rows, [(1, 73)]);
            Ok(())
        })
    })
    .await
}

pub async fn returned_body_failure() -> ProbeResult {
    let harness = super::harness().await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let report = FixtureSuite::new(harness)
        .start(|scope| {
            Box::pin(async move {
                let excessive =
                    ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(8)], 1)?;
                assert!(matches!(
                    scope.empty(&excessive).await,
                    Err(FixtureError::InvalidBudget)
                ));
                let plan =
                    ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1); 2], 0)?;
                scope.empty(&plan).await?;
                Err::<(), _>(FixtureError::Observe(sqlx::Error::Protocol(
                    "fixture-sentinel".into(),
                )))
            })
        })
        .into_report()
        .await?;
    let absence = fixture_run::absent(&observer, &report.databases[0].database_name).await;
    observer.close().await;
    absence?;
    assert!(
        report
            .databases
            .iter()
            .all(|database| database.result.is_ok())
    );
    assert!(report.drain.is_ok() && report.acquisitions.iter().all(Result::is_ok));
    assert!(
        matches!(report.body, Err(BodyFailure::Returned(FixtureError::Observe(sqlx::Error::Protocol(message)))) if message == "fixture-sentinel")
    );
    Ok(())
}
