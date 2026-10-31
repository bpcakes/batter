//! Retained ownership and report outcomes when a separately provisioned login
//! cannot open its fixture pool.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter::{
    settings::SecretString,
    sqlx::{
        SqlxFailure,
        test_support::{BodyFailure, ConnectionPlan, FixtureError, FixtureSuite, PlannedPool},
    },
};
use runledger_postgres::PgSessionProfile;
use sqlx::postgres::PgPoolOptions;

use super::{
    ProbeResult,
    fixture_failures::observed_failure,
    fixture_login::{declared, one, secret, with_login},
    fixture_run,
};

/// SQLSTATE of a native database error, if the error is one.
pub(super) fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned())
}

fn rejected(password: &str) -> SecretString {
    secret(format!("{password}-rejected"))
}

pub async fn rejected_login_retained() -> ProbeResult {
    with_login(|role, password| async move {
        owned_rejection(&role, &password).await?;
        rejection_before_eager_pool(&role, &password).await?;
        low_level_rejection(&role, &password).await
    })
    .await
}

/// A sibling acquisition with the real password proves the role authenticates,
/// so the rejected sibling fails on its credentials. The body handles the
/// delivered error; the report must still retain the same cause.
async fn owned_rejection(role: &str, password: &str) -> ProbeResult {
    let mut accepted = ConnectionPlan::declare(0);
    let login = accepted.pool(PlannedPool::password_login(one(), role, secret(password)));
    let accepted = accepted.finish()?;
    let wrong = declared(vec![
        PlannedPool::lease_login(one()),
        PlannedPool::password_login(one(), role, rejected(password)),
    ])?;
    let delivered = Arc::new(Mutex::new(None));
    let body_delivered = Arc::clone(&delivered);
    let report = observed_failure(move |scope| {
        Box::pin(async move {
            let (accepted, wrong) = tokio::join!(scope.empty(&accepted), scope.empty(&wrong));
            let accepted = accepted?;
            let user: String = sqlx::query_scalar("SELECT session_user::text")
                .fetch_one(accepted.pool(login).expect("declared login pool"))
                .await
                .map_err(FixtureError::Observe)?;
            let Err(FixtureError::PoolAcquire(error)) = wrong else {
                panic!("a wrong password must fail the connectivity check");
            };
            *body_delivered.lock().expect("delivery slot") = Some((user, error));
            Ok(())
        })
    })
    .await?;
    let (user, delivered) = delivered
        .lock()
        .expect("delivery slot")
        .take()
        .expect("body delivered the rejection");
    assert_eq!(user, role);
    assert_eq!(code(&delivered).as_deref(), Some("28P01"));
    assert!(report.body.is_ok() && !report.is_ok());
    assert_eq!(report.databases.len(), 2);
    let failures: Vec<_> = report
        .databases
        .iter()
        .flat_map(|database| &database.pool_failures)
        .collect();
    assert_eq!(failures.len(), 1);
    assert!(Arc::ptr_eq(&delivered, failures[0]));
    Ok(())
}

/// The rejected login is declared first. The later eager pool still exists,
/// connects on its own and is closed by the driver before lease cleanup.
async fn rejection_before_eager_pool(role: &str, password: &str) -> ProbeResult {
    let connected = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&connected);
    let eager = one()
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .after_connect(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        });
    let plan = declared(vec![
        PlannedPool::password_login(one(), role, rejected(password)),
        PlannedPool::lease_login(eager),
    ])?;
    let witness = Arc::clone(&connected);
    let report = observed_failure(move |scope| {
        Box::pin(async move {
            let error = scope
                .empty(&plan)
                .await
                .err()
                .expect("the first declared login must be rejected");
            tokio::time::timeout(Duration::from_secs(10), async {
                while witness.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| FixtureError::ObservationTimeout)?;
            Err(error)
        })
    })
    .await?;
    assert_eq!(connected.load(Ordering::SeqCst), 1);
    assert!(matches!(
        &report.body,
        Err(BodyFailure::Returned(FixtureError::PoolAcquire(error)))
            if code(error).as_deref() == Some("28P01")
    ));
    assert_eq!(report.databases.len(), 1);
    assert_eq!(report.databases[0].pool_failures.len(), 1);
    Ok(())
}

async fn low_level_rejection(role: &str, password: &str) -> ProbeResult {
    let harness = super::harness().await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let suite = FixtureSuite::new(harness.clone());
    let names = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&names);
    // Declared after the rejected login and eager: it exists only if the
    // low-level path constructed every pool before the first check, and close
    // waits for its connection, so the name is recorded before `Connect` returns.
    let owner = one()
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .after_connect(move |connection, _| {
            let recorder = Arc::clone(&recorder);
            Box::pin(async move {
                let name: String = sqlx::query_scalar("SELECT current_database()::text")
                    .fetch_one(connection)
                    .await?;
                recorder.lock().expect("name list").push(name);
                Ok(())
            })
        });
    let plan = declared(vec![
        PlannedPool::password_login(one(), role, rejected(password)),
        PlannedPool::lease_login(owner),
    ])?;
    let result = suite.empty(&plan).await;
    let names = names.lock().expect("name list").clone();
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
    assert_eq!(names.len(), 1);
    assert!(matches!(
        &result,
        Err(FixtureError::Connect { source, cleanup: None }) if code(source).as_deref() == Some("28P01")
    ));
    observation?;
    drained?;
    shutdown?;
    Ok(())
}

/// The login authenticates, but its declared session policy names an effective
/// role it cannot assume. SQLx discards each connection whose after_connect
/// fails and retries until acquisition expires.
pub async fn session_policy_failure_retained() -> ProbeResult {
    with_login(|role, password| async move {
        let profile = Arc::new(PgSessionProfile::new(
            role.clone(),
            "pg_read_all_stats",
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )?);
        let first = Arc::new(Mutex::new(None));
        let recorder = Arc::clone(&first);
        let failing = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(300))
            .after_connect(move |connection, _| {
                let (profile, recorder) = (Arc::clone(&profile), Arc::clone(&recorder));
                Box::pin(async move {
                    let result = profile.reset_and_apply(connection).await;
                    if let Err(error) = &result {
                        recorder
                            .lock()
                            .expect("first failure")
                            .get_or_insert_with(|| native_code(error));
                    }
                    result
                })
            });
        let plan = declared(vec![
            PlannedPool::lease_login(one()),
            PlannedPool::password_login(failing, role, secret(password)),
        ])?;
        let delivered = Arc::new(Mutex::new(None));
        let body_delivered = Arc::clone(&delivered);
        let report = observed_failure(move |scope| {
            Box::pin(async move {
                let Err(FixtureError::PoolAcquire(error)) = scope.empty(&plan).await else {
                    panic!("the login's session policy must fail");
                };
                *body_delivered.lock().expect("delivery slot") = Some(error);
                Ok(())
            })
        })
        .await?;
        let delivered = delivered
            .lock()
            .expect("delivery slot")
            .take()
            .expect("body delivered the failure");
        assert!(matches!(delivered.as_ref(), sqlx::Error::PoolTimedOut));
        assert!(report.body.is_ok() && !report.is_ok());
        assert_eq!(report.databases.len(), 1);
        assert_eq!(report.databases[0].pool_failures.len(), 1);
        assert!(Arc::ptr_eq(
            &delivered,
            &report.databases[0].pool_failures[0]
        ));
        let first = first.lock().expect("first failure").clone();
        assert_eq!(first.flatten().as_deref(), Some("42501"));
        Ok(())
    })
    .await
}

/// Profile failures carry the native cause inside a redacted `SqlxFailure`.
fn native_code(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Configuration(source) => source
            .downcast_ref::<SqlxFailure>()
            .and_then(|failure| code(failure.native())),
        other => code(other),
    }
}
