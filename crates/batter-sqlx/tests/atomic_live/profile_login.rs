use super::support::{Result, bounded, combine, require};
use batter_sqlx::{
    PgAtomicError, PgProfileError, PgReadOnlySnapshot, PgSessionProfile, PgSnapshotError,
    PgTransactionError, SqlxFailure, run_atomic_profiled,
};
use sqlx::{Connection, PgConnection, PgPool, postgres::PgPoolOptions};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiles_reject_declared_login_mismatch_before_application_work() -> Result {
    let options = std::env::var("BATTER_SQLX_ADMIN_URL")?.parse()?;
    let mut admin = bounded(PgConnection::connect_with(&options)).await??;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let login = format!("profile_login_{suffix}");
    let role = format!("profile_role_{suffix}");
    let schema = format!("profile_schema_{suffix}");
    let setup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD 'profile-regression-password'; \
         CREATE ROLE {role}; CREATE SCHEMA {schema}; \
         GRANT USAGE ON SCHEMA {schema} TO {role}; \
         GRANT {role} TO {login} WITH INHERIT FALSE, SET TRUE"
    )))
    .execute(&mut admin)
    .await;
    let pool = PgPoolOptions::new().max_connections(1).connect_lazy_with(
        options
            .username(&login)
            .password("profile-regression-password"),
    );
    let body = async {
        setup?;
        check_profiles(&pool, &login, &role, &schema).await
    }
    .await;
    let pool_close = bounded(pool.close()).await;
    let cleanup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema}; DROP ROLE IF EXISTS {login}; \
         DROP ROLE IF EXISTS {role}"
    )))
    .execute(&mut admin)
    .await
    .map(|_| ())
    .map_err(Into::into);
    let admin_close = bounded(admin.close())
        .await
        .and_then(|result| result.map_err(Into::into));
    combine(body, combine(pool_close, combine(cleanup, admin_close)))
}

fn profiles(login: &str, role: &str, schema: &str) -> Result<[PgSessionProfile; 2]> {
    let legacy = PgSessionProfile::new(
        login,
        role,
        vec![schema.into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    let complete = PgSessionProfile::with_timeouts(
        login,
        role,
        vec![schema.into()],
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
    )?;
    Ok([
        legacy.with_setting("app.tenant", "restricted")?,
        complete.with_setting("app.tenant", "restricted")?,
    ])
}

async fn check_profiles(pool: &PgPool, login: &str, role: &str, schema: &str) -> Result {
    // The wrong declared login equals current_user, so accidentally selecting
    // current_user instead of session_user cannot satisfy this regression.
    for (valid, mismatched) in profiles(login, role, schema)?
        .into_iter()
        .zip(profiles(role, role, schema)?)
    {
        let mut connection = bounded(pool.acquire()).await??;
        bounded(valid.reset_and_apply(&mut connection)).await??;
        let (actual_login, actual_role, superuser, tenant): (String, String, bool, String) =
            sqlx::query_as(
                "SELECT session_user::text, current_user::text, \
                 (SELECT rolsuper FROM pg_roles WHERE rolname = session_user), \
                 current_setting('app.tenant')",
            )
            .fetch_one(&mut *connection)
            .await?;
        require(
            actual_login == login && actual_role == role && !superuser && tenant == "restricted",
            "valid profile control must establish restricted identity and settings",
        )?;
        let Err(error) = bounded(mismatched.reset_and_apply(&mut connection)).await? else {
            return Err(std::io::Error::other("mismatched login must reject direct setup").into());
        };
        assert_mismatch(&error, login, role)?;
        // A direct native reset failure leaves disposition to its caller.
        // Close this connection instead of returning its rejected state.
        bounded(connection.close()).await??;
        check_protected_work(pool, &mismatched, login, role).await?;
    }
    Ok(())
}

async fn check_protected_work(
    pool: &PgPool,
    profile: &PgSessionProfile,
    login: &str,
    role: &str,
) -> Result {
    let invoked = AtomicBool::new(false);
    let result = bounded(run_atomic_profiled(pool, profile, async |_| {
        invoked.store(true, Ordering::Relaxed);
        Ok::<(), ()>(())
    }))
    .await?;
    require(
        !invoked.load(Ordering::Relaxed),
        "atomic callback ran before login rejection",
    )?;
    let Err(PgAtomicError::Begin(PgTransactionError::Query(cause))) = result else {
        return Err(std::io::Error::other("atomic setup must retain login rejection").into());
    };
    assert_mismatch(cause.native(), login, role)?;
    let result = bounded(PgReadOnlySnapshot::inspect_profiled(
        pool,
        profile,
        async |_| {
            invoked.store(true, Ordering::Relaxed);
            Ok::<(), ()>(())
        },
    ))
    .await?;
    require(
        !invoked.load(Ordering::Relaxed),
        "snapshot callback ran before login rejection",
    )?;
    let Err(PgSnapshotError::Transaction(PgTransactionError::Query(cause))) = result else {
        return Err(std::io::Error::other("snapshot setup must retain login rejection").into());
    };
    assert_mismatch(cause.native(), login, role)
}

fn assert_mismatch(error: &sqlx::Error, login: &str, role: &str) -> Result {
    let rendered = format!("{error} {error:?}");
    require(
        !rendered.contains(login) && !rendered.contains(role),
        "login rejection disclosed identity",
    )?;
    let sqlx::Error::Configuration(cause) = error else {
        return Err(std::io::Error::other("login rejection must be configuration failure").into());
    };
    // reset_and_apply wraps the native mismatch; protected runners retain it
    // through PgTransactionError::Query. Check the actual cause on both paths.
    let original = cause
        .downcast_ref::<SqlxFailure>()
        .map_or(error, SqlxFailure::native);
    let sqlx::Error::Configuration(cause) = original else {
        return Err(std::io::Error::other("native profile mismatch was not retained").into());
    };
    require(
        cause.downcast_ref::<PgProfileError>().is_some(),
        "setup must fail for profile mismatch rather than SQL permissions",
    )
}
