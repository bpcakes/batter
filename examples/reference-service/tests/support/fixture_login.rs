//! Owned fixture pools that authenticate as a separately provisioned login.
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use batter::{
    settings::SecretString,
    sqlx::test_support::{ConnectionPlan, FixtureError, PlannedPool},
};
use batter_example_reference_service::{config::PoolSettings, database};
use runledger_postgres::PgSessionProfile;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;

use super::{
    ProbeResult,
    fixture_failures::observed_failure,
    fixture_run::{self, ProbeError},
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Run a case with a fresh restricted login owned by this outer cleanup owner.
/// The case is joined before the role is dropped, so an assertion panic cannot
/// skip role cleanup. A completed case has consumed every lease it created. If a
/// run is still pending, a lease holding the role's grants makes `DROP ROLE`
/// fail; both errors are returned and the role keeps its five-minute expiry.
pub(super) async fn with_login<F, Fut>(case: F) -> ProbeResult
where
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = ProbeResult> + Send + 'static,
{
    let catalog = super::fixture_observation::admin().await?;
    // The SQL identifier contains only this literal prefix and decimal digits.
    let role = format!(
        "fixture_login_{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let password = super::fixture_observer_role::create_role(&catalog, &role).await?;
    let result = tokio::spawn(case(role.clone(), password)).await;
    let removed = sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE {role}")))
        .execute(&catalog)
        .await;
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role)
            .fetch_one(&catalog)
            .await;
    catalog.close().await;
    let result = result
        .map_err(|error| ProbeError::new(Box::new(error)))
        .and_then(|result| result.map_err(ProbeError::new));
    batter::test_support::finish(result, removed.map(|_| ()))?;
    assert!(!exists?, "the outer owner must remove the restricted login");
    Ok(())
}

/// Declare pools for a case that never selects one of them by key.
pub(super) fn declared(pools: Vec<PlannedPool>) -> Result<ConnectionPlan, FixtureError> {
    let mut plan = ConnectionPlan::declare(0);
    for pool in pools {
        plan.pool(pool);
    }
    plan.finish()
}

pub(super) fn secret(password: impl Into<String>) -> SecretString {
    SecretString::new(password)
}

pub(super) fn one() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
}

/// The reference application's own pool settings and declared session profile.
/// Public hooks reset the policy after connect and before idle reuse; the
/// crate-private acquisition check of `PgProfiledPool` is not reproduced.
fn reference_login_pool(profile: PgSessionProfile) -> Result<PgPoolOptions, BoxError> {
    let profile = Arc::new(profile);
    let on_release = Arc::clone(&profile);
    Ok(PoolSettings::new(2, 0, Duration::from_secs(10))?
        .pool_options()
        .after_connect(move |connection, _| {
            let profile = Arc::clone(&profile);
            Box::pin(async move { profile.reset_and_apply(connection).await })
        })
        .after_release(move |connection, _| {
            let profile = Arc::clone(&on_release);
            Box::pin(async move { profile.reset_and_apply(connection).await.map(|()| true) })
        }))
}

pub async fn principals_and_authority() -> ProbeResult {
    with_login(|role, password| async move {
        let pools = fixture_run::run(move |scope, observer| {
            Box::pin(async move {
                let admin: String = sqlx::query_scalar("SELECT session_user::text")
                    .fetch_one(&observer)
                    .await?;
                let mut plan = ConnectionPlan::declare(0);
                let owner = plan.pool(PlannedPool::lease_login(
                    PoolSettings::new(1, 0, Duration::from_secs(10))?.pool_options(),
                ));
                let login = plan.pool(PlannedPool::password_login(
                    reference_login_pool(database::session_profile(&role)?)?,
                    role.clone(),
                    secret(password),
                ));
                let plan = plan.finish()?;
                // Index 0 in its own plan: it must not select `owner` here.
                let mut other = ConnectionPlan::declare(0);
                let foreign = other.pool(PlannedPool::lease_login(one()));
                let _other = other.finish()?;
                let db = scope.empty(&plan).await?;
                let name = db.database_name().to_owned();
                assert!(db.pool(foreign).is_none());
                let owner = db.pool(owner).expect("declared lease-login pool");
                let restricted = db.pool(login).expect("declared password-login pool");
                assert!(std::ptr::eq(owner, &db.pools()[0]));
                assert!(std::ptr::eq(restricted, &db.pools()[1]));
                // system_user names the authenticated identity and method. SET ROLE
                // or SET SESSION AUTHORIZATION on an owner connection cannot change it.
                let owner_identity: (String, String, Option<String>) =
                    sqlx::query_as("SELECT current_database()::text, session_user::text, system_user")
                        .fetch_one(owner)
                        .await?;
                let expected = (name.clone(), admin.clone(), Some(format!("scram-sha-256:{admin}")));
                assert_eq!(owner_identity, expected);
                let login_identity: (String, String, String, Option<String>, bool, bool) =
                    sqlx::query_as(
                        "SELECT current_database()::text, session_user::text, current_user::text,
                                system_user, rolsuper, current_setting('search_path') LIKE '%pg_temp'
                         FROM pg_roles WHERE rolname = session_user",
                    )
                    .fetch_one(restricted)
                    .await?;
                let expected = (
                    name,
                    role.clone(),
                    role.clone(),
                    Some(format!("scram-sha-256:{role}")),
                    false,
                    true, // The reference profile, not the server default, set the path.
                );
                assert_eq!(login_identity, expected);
                assert_ne!(admin, role);
                authority(owner, restricted, &role).await?;
                Ok(db.pools().to_vec())
            })
        })
        .await?;
        // The runner closed both pools before consuming the lease; clones observe it.
        assert_eq!(pools.len(), 2);
        assert!(pools.iter().all(PgPool::is_closed));
        Ok(())
    })
    .await
}

async fn authority(owner: &PgPool, restricted: &PgPool, role: &str) -> ProbeResult {
    sqlx::query("CREATE TABLE fixture_owned (id bigint PRIMARY KEY)")
        .execute(owner)
        .await?;
    sqlx::query("INSERT INTO fixture_owned VALUES (1)")
        .execute(owner)
        .await?;
    let grant: String =
        sqlx::query_scalar("SELECT format('GRANT SELECT ON fixture_owned TO %I', $1::text)")
            .bind(role)
            .fetch_one(owner)
            .await?;
    sqlx::query(sqlx::AssertSqlSafe(grant))
        .execute(owner)
        .await?;
    let granted: Vec<i64> = sqlx::query_scalar("SELECT id FROM fixture_owned")
        .fetch_all(restricted)
        .await?;
    assert_eq!(granted, [1]);
    for statement in [
        "INSERT INTO fixture_owned VALUES (2)",
        "DROP TABLE fixture_owned",
        "CREATE TABLE public.fixture_denied (id bigint)",
    ] {
        let error = sqlx::query(statement)
            .execute(restricted)
            .await
            .expect_err("an owner-only operation must fail for the restricted login");
        assert_eq!(
            super::fixture_login_failures::code(&error).as_deref(),
            Some("42501")
        );
    }
    let retained: Vec<i64> = sqlx::query_scalar("SELECT id FROM fixture_owned")
        .fetch_all(owner)
        .await?;
    assert_eq!(retained, [1]);
    Ok(())
}

pub async fn cancelled_waiter_retains_pools() -> ProbeResult {
    with_login(|role, password| async move {
        cancel_during_login_check(&role, &password).await?;
        cancel_before_login_check(&role, &password).await
    })
    .await
}

/// The restricted pool's after_connect announces its session, then never
/// finishes; the body cancels the waiter. The observer must still witness that
/// backend's absence after both registered pools close.
async fn cancel_during_login_check(role: &str, password: &str) -> ProbeResult {
    let (sender, receiver) = oneshot::channel::<(i32, String)>();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let blocked = one().after_connect(move |connection, _| {
        let sender = Arc::clone(&sender);
        Box::pin(async move {
            let identity: (i32, String) =
                sqlx::query_as("SELECT pg_backend_pid(), session_user::text")
                    .fetch_one(&mut *connection)
                    .await?;
            if let Some(sender) = take(&sender) {
                let _ = sender.send(identity);
            }
            std::future::pending::<()>().await; // Only waiter cancellation ends this.
            Ok(())
        })
    });
    let plan = declared(vec![
        PlannedPool::lease_login(one()),
        PlannedPool::password_login(blocked, role, secret(password)),
    ])?;
    let (observed, identity) = oneshot::channel();
    let report = observed_failure(move |scope| {
        Box::pin(async move {
            tokio::select! {
                result = scope.empty(&plan) => {
                    result?;
                    panic!("the blocked login check cannot complete");
                }
                announced = receiver => {
                    let _ = observed.send(announced.expect("login check announced its session"));
                }
            }
            Ok(())
        })
    })
    .await?;
    let (pid, user) = identity.await?;
    assert!(pid > 0);
    assert_eq!(user, role);
    // Cancellation is not a failure: the run retained and cleaned everything.
    assert!(report.is_ok(), "{report:?}");
    assert_eq!(report.databases.len(), 1);
    Ok(())
}

/// The lease-login pool is checked first and never finishes; the restricted
/// pool is declared second and eager. Its login connection proves it was
/// registered before the first check, since the waiter was cancelled there.
async fn cancel_before_login_check(role: &str, password: &str) -> ProbeResult {
    let (sender, receiver) = oneshot::channel::<()>();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let blocked = one().after_connect(move |_, _| {
        let sender = Arc::clone(&sender);
        Box::pin(async move {
            if let Some(sender) = take(&sender) {
                let _ = sender.send(());
            }
            std::future::pending::<()>().await;
            Ok(())
        })
    });
    let logins = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&logins);
    let eager = one()
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .after_connect(move |connection, _| {
            let recorder = Arc::clone(&recorder);
            Box::pin(async move {
                let user: String = sqlx::query_scalar("SELECT session_user::text")
                    .fetch_one(&mut *connection)
                    .await?;
                recorder.lock().expect("login list").push(user);
                Ok(())
            })
        });
    let plan = declared(vec![
        PlannedPool::lease_login(blocked),
        PlannedPool::password_login(eager, role, secret(password)),
    ])?;
    let witness = Arc::clone(&logins);
    let report = observed_failure(move |scope| {
        Box::pin(async move {
            tokio::select! {
                result = scope.empty(&plan) => {
                    result?;
                    panic!("the blocked lease-login check cannot complete");
                }
                _ = receiver => {}
            }
            tokio::time::timeout(Duration::from_secs(10), async {
                while recorded(&witness) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| FixtureError::ObservationTimeout)?;
            Ok(())
        })
    })
    .await?;
    assert_eq!(*logins.lock().expect("login list"), [role.to_owned()]);
    assert!(report.is_ok(), "{report:?}");
    assert_eq!(report.databases.len(), 1);
    Ok(())
}

fn take<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().expect("signal slot").take()
}

fn recorded(logins: &Mutex<Vec<String>>) -> usize {
    logins.lock().expect("login list").len()
}
