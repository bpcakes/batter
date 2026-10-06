use super::{
    fixture,
    profile_hook_events::HookEvents,
    support::{Result, bounded, combine, require},
};
use batter_sqlx::{PgProfiledPool, PgSessionProfile};
use sqlx::{
    Connection, PgConnection,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::time::Duration;
use tracing_subscriber::prelude::*;

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiled_pool_rejects_idle_set_role_revocation() -> Result {
    idle_revocation(Revocation::Role).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiled_pool_rejects_idle_schema_usage_revocation() -> Result {
    idle_revocation(Revocation::Schema).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiled_pool_rejects_idle_parameter_set_revocation() -> Result {
    idle_revocation(Revocation::Parameter).await
}

#[derive(Clone, Copy, Debug)]
enum Revocation {
    Role,
    Schema,
    Parameter,
}

async fn idle_revocation(revocation: Revocation) -> Result {
    let fixture = fixture().await?;
    let login = format!("profile_login_{}", fixture.key);
    let role = format!("profile_role_{}", fixture.key);
    let schema = format!("profile_schema_{}", fixture.key);
    let options: PgConnectOptions = std::env::var("BATTER_SQLX_ADMIN_URL")?.parse()?;
    let mut admin = bounded(PgConnection::connect_with(&options)).await??;
    // Only the parameter case owns this cluster-wide ACL entry. The other
    // concurrent cases must not update the same pg_parameter_acl row.
    let parameter = matches!(revocation, Revocation::Parameter);
    let parameter_setup = if parameter {
        format!(
            "ALTER ROLE {login} SET session_preload_libraries = 'auto_explain'; \
                 GRANT SET ON PARAMETER auto_explain.log_analyze TO {role}"
        )
    } else {
        String::new()
    };
    // The names are fixture-generated identifiers; this password is test-only.
    let setup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD 'profile-regression-password'; \
         CREATE ROLE {role}; CREATE SCHEMA {schema}; \
         GRANT USAGE ON SCHEMA {schema} TO {role}; \
         GRANT {role} TO {login} WITH INHERIT FALSE, SET TRUE; {parameter_setup}"
    )))
    .execute(&mut admin)
    .await;
    let mut profile = PgSessionProfile::with_timeouts(
        &login,
        &role,
        vec![schema.clone()],
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
    )?
    .with_setting("app.tenant", "restricted")?;
    if parameter {
        profile = profile.with_setting("auto_explain.log_analyze", "on")?;
    }
    let database = PgProfiledPool::connect_lazy(
        options
            .username(&login)
            .password("profile-regression-password"),
        profile,
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(750)),
    )?;
    let body = async {
        setup?;
        check_revocation(&database, &mut admin, &login, &role, &schema, revocation).await
    }
    .await;
    let pool_close = bounded(database.pool().close()).await;
    let parameter_cleanup = if parameter {
        format!("REVOKE SET ON PARAMETER auto_explain.log_analyze FROM {role};")
    } else {
        String::new()
    };
    let cleanup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema}; {parameter_cleanup} \
         DROP ROLE IF EXISTS {login}; DROP ROLE IF EXISTS {role}"
    )))
    .execute(&mut admin)
    .await
    .map(|_| ())
    .map_err(Into::into);
    let admin_close = bounded(admin.close())
        .await
        .and_then(|result| result.map_err(Into::into));
    fixture
        .finish(combine(
            body,
            combine(pool_close, combine(cleanup, admin_close)),
        ))
        .await
}

async fn check_revocation(
    database: &PgProfiledPool,
    admin: &mut PgConnection,
    login: &str,
    role: &str,
    schema: &str,
    revocation: Revocation,
) -> Result {
    let old_pid = admitted_idle_backend(database, login, role, revocation).await?;
    let revoke = match revocation {
        Revocation::Role => format!("REVOKE SET OPTION FOR {role} FROM {login}"),
        Revocation::Schema => format!("REVOKE USAGE ON SCHEMA {schema} FROM {role}"),
        Revocation::Parameter => {
            format!("REVOKE SET ON PARAMETER auto_explain.log_analyze FROM {role}")
        }
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(revoke))
        .execute(&mut *admin)
        .await?;
    check_revoked_authority(admin, login, role, schema, revocation).await?;
    let events = HookEvents::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(events.clone()));
    let acquire = tracing::dispatcher::with_default(&dispatch, || {
        batter_core::telemetry::with_current_dispatch(database.pool().acquire())
    });
    let acquired = bounded(acquire).await?;
    let rejected = matches!(&acquired, Err(sqlx::Error::PoolTimedOut));
    eprintln!("idle_policy_revocation kind={revocation:?} acquisition_rejected={rejected}");
    drop(acquired);
    require(
        rejected,
        "ordinary acquisition handed out a revoked idle session",
    )?;
    // Replacement-connection diagnostics are a separate phase from the idle hook.
    events.assert_hook_redacted(
        "error from `before_acquire`",
        &[login, role, "auto_explain.log_analyze"],
    )?;
    // The hook error discards the old client; replacement also fails setup
    // until authority is restored. Witness backend exit independently.
    bounded(async {
        loop {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)")
                    .bind(old_pid)
                    .fetch_one(&mut *admin)
                    .await?;
            if !exists {
                break Ok::<(), sqlx::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    let restore = match revocation {
        Revocation::Role => format!("GRANT {role} TO {login} WITH SET TRUE"),
        Revocation::Schema => format!("GRANT USAGE ON SCHEMA {schema} TO {role}"),
        Revocation::Parameter => {
            format!("GRANT SET ON PARAMETER auto_explain.log_analyze TO {role}")
        }
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(restore))
        .execute(&mut *admin)
        .await?;
    let mut connection = bounded(database.pool().acquire()).await??;
    let (new_pid, effective): (i32, String) =
        sqlx::query_as("SELECT pg_backend_pid(), current_user::text")
            .fetch_one(&mut *connection)
            .await?;
    require(
        new_pid != old_pid && effective == role,
        "restored authority must acquire a new correctly profiled session",
    )?;
    Ok(())
}

async fn admitted_idle_backend(
    database: &PgProfiledPool,
    login: &str,
    role: &str,
    revocation: Revocation,
) -> Result<i32> {
    let mut original = None;
    // Exercise both fresh setup and ordinary idle reuse under a restricted
    // role, including a USERSET placeholder without an explicit parameter grant.
    for _ in 0..2 {
        let mut connection = bounded(database.pool().acquire()).await??;
        let (pid, actual_login, actual_role, superuser): (i32, String, String, bool) =
            sqlx::query_as(
                "SELECT pg_backend_pid(), session_user::text, current_user::text, \
                (SELECT rolsuper FROM pg_roles WHERE rolname = session_user)",
            )
            .fetch_one(&mut *connection)
            .await?;
        require(
            actual_login == login && actual_role == role && !superuser,
            "revocation control requires the restricted login and distinct effective role",
        )?;
        if matches!(revocation, Revocation::Parameter) {
            let context: String = sqlx::query_scalar(
                "SELECT context FROM pg_settings WHERE name = 'auto_explain.log_analyze'",
            )
            .fetch_one(&mut *connection)
            .await?;
            require(
                context == "superuser",
                "auto_explain must be loaded, not a placeholder",
            )?;
        }
        let placeholder_grant: bool =
            sqlx::query_scalar("SELECT has_parameter_privilege(current_user, 'app.tenant', 'SET')")
                .fetch_one(&mut *connection)
                .await?;
        require(
            !placeholder_grant,
            "USERSET control must have no explicit parameter grant",
        )?;
        require(
            *original.get_or_insert(pid) == pid,
            "idle control must reuse the same backend",
        )?;
        drop(connection);
        // Revoke only after normalization and idle publication, never while the
        // after_release hook is still restoring this session's authority.
        bounded(async {
            while database.pool().num_idle() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
    }
    Ok(original.expect("two acquisitions were checked"))
}

async fn check_revoked_authority(
    admin: &mut PgConnection,
    login: &str,
    role: &str,
    schema: &str,
    revocation: Revocation,
) -> Result {
    let (member, set_role, usage, parameter): (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT pg_has_role($1::name, $2::text, 'MEMBER'), \
                pg_has_role($1::name, $2::text, 'SET'), \
                has_schema_privilege($2::name, $3::text, 'USAGE'), \
                has_parameter_privilege($2::name, 'auto_explain.log_analyze', 'SET')",
    )
    .bind(login)
    .bind(role)
    .bind(schema)
    .fetch_one(admin)
    .await?;
    let expected = match revocation {
        Revocation::Role => (false, true, false),
        Revocation::Schema => (true, false, false),
        Revocation::Parameter => (true, true, false),
    };
    require(
        member && (set_role, usage, parameter) == expected,
        "only the selected authority must be revoked, preserving role membership",
    )
}
