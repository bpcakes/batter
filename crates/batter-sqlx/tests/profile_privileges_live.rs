#[allow(dead_code)]
mod support;

use batter_sqlx::{PgProfiledPool, PgSessionProfile};
use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};
use std::time::Duration;
use support::{Result, bounded, combine, require};

// This target runs separately from atomic_live: the function ACL is shared by
// every session in the supplied admin database. Never run it beside other live
// targets using that database. scripts/sqlx_live.py runs targets serially.
#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiles_use_effective_role_function_permissions() -> Result {
    let options = std::env::var("BATTER_SQLX_ADMIN_URL")?.parse()?;
    let mut admin = bounded(PgConnection::connect_with(&options)).await??;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let login = format!("profile_login_{suffix}");
    let role = format!("profile_role_{suffix}");
    let schema = format!("profile_schema_{suffix}");
    let public_execute: Option<bool> = sqlx::query_scalar(
        "SELECT acl.is_grantable FROM pg_catalog.pg_proc p, LATERAL \
         pg_catalog.aclexplode(COALESCE(p.proacl, pg_catalog.acldefault('f', p.proowner))) acl \
         WHERE p.oid = 'pg_catalog.set_config(text,text,boolean)'::regprocedure \
         AND acl.grantee = 0 AND acl.privilege_type = 'EXECUTE'",
    )
    .fetch_optional(&mut admin)
    .await?;
    let setup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD 'profile-regression-password'; \
         CREATE ROLE {role}; CREATE SCHEMA {schema}; \
         GRANT USAGE ON SCHEMA {schema} TO {role}; \
         GRANT {role} TO {login} WITH INHERIT FALSE, SET TRUE; \
         REVOKE EXECUTE ON FUNCTION pg_catalog.set_config(text,text,boolean) FROM PUBLIC; \
         GRANT EXECUTE ON FUNCTION pg_catalog.set_config(text,text,boolean) TO {role}"
    )))
    .execute(&mut admin)
    .await;
    let body = async {
        setup?;
        let rights: (bool, bool, bool) = sqlx::query_as(
            "SELECT has_function_privilege($1::text, 'pg_catalog.set_config(text,text,boolean)', 'EXECUTE'), \
             has_function_privilege($2::text, 'pg_catalog.set_config(text,text,boolean)', 'EXECUTE'), \
             pg_has_role($1::text, $2::text, 'USAGE')",
        ).bind(&login).bind(&role).fetch_one(&mut admin).await?;
        require(rights == (false, true, false), "function grant must belong only to the effective role")?;
        let login_options = options.username(&login).password("profile-regression-password");
        check_profiles(login_options, &login, &role, &schema).await
    }.await;
    let restore = match public_execute {
        Some(true) => {
            "GRANT EXECUTE ON FUNCTION pg_catalog.set_config(text,text,boolean) TO PUBLIC WITH GRANT OPTION;"
        }
        Some(false) => {
            "GRANT EXECUTE ON FUNCTION pg_catalog.set_config(text,text,boolean) TO PUBLIC;"
        }
        None => "",
    };
    let cleanup = restore_then_cleanup(
        &mut admin,
        restore,
        &format!(
            "REVOKE EXECUTE ON FUNCTION pg_catalog.set_config(text,text,boolean) FROM {role}; \
         DROP SCHEMA IF EXISTS {schema}; DROP ROLE IF EXISTS {login}; DROP ROLE IF EXISTS {role}"
        ),
    )
    .await;
    let close = bounded(admin.close())
        .await
        .and_then(|result| result.map_err(Into::into));
    combine(body, combine(cleanup, close))
}

async fn restore_then_cleanup(admin: &mut PgConnection, restore: &str, cleanup: &str) -> Result {
    // Separate acknowledgements: a failed fixture drop must not roll back the
    // restoration of a pre-existing grant. Attempt both and retain both errors.
    let restored = bounded(sqlx::raw_sql(sqlx::AssertSqlSafe(restore)).execute(&mut *admin))
        .await
        .and_then(|result| result.map(|_| ()).map_err(Into::into));
    let removed = bounded(sqlx::raw_sql(sqlx::AssertSqlSafe(cleanup)).execute(admin))
        .await
        .and_then(|result| result.map(|_| ()).map_err(Into::into));
    combine(restored, removed)
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn function_acl_restore_survives_fixture_drop_failure() -> Result {
    let options = std::env::var("BATTER_SQLX_ADMIN_URL")?.parse()?;
    let mut admin = bounded(PgConnection::connect_with(&options)).await??;
    let schema = format!("profile_cleanup_{}", uuid::Uuid::new_v4().simple());
    // Use a test-owned function for fault injection, leaving the built-in ACL alone.
    let body = async {
        bounded(
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA {schema}; \
             CREATE FUNCTION {schema}.marker() RETURNS int LANGUAGE SQL AS 'SELECT 1'; \
             REVOKE EXECUTE ON FUNCTION {schema}.marker() FROM PUBLIC"
            )))
            .execute(&mut admin),
        )
        .await??;
        let result = restore_then_cleanup(
            &mut admin,
            &format!("GRANT EXECUTE ON FUNCTION {schema}.marker() TO PUBLIC;"),
            &format!("DROP SCHEMA {schema}"),
        )
        .await;
        require(result.is_err(), "nonempty fixture schema drop must fail")?;
        let restored: bool = bounded(
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_proc p, LATERAL \
             pg_catalog.aclexplode(p.proacl) acl \
             WHERE p.oid = $1::regprocedure AND acl.grantee = 0 \
             AND acl.privilege_type = 'EXECUTE')",
            )
            .bind(format!("{schema}.marker()"))
            .fetch_one(&mut admin),
        )
        .await??;
        require(
            restored,
            "fixture-drop failure must not undo acknowledged ACL restoration",
        )
    }
    .await;
    let cleanup = bounded(
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE"
        )))
        .execute(&mut admin),
    )
    .await
    .and_then(|result| result.map(|_| ()).map_err(Into::into));
    let close = bounded(admin.close())
        .await
        .and_then(|result| result.map_err(Into::into));
    combine(body, combine(cleanup, close))
}

async fn check_profiles(
    options: sqlx::postgres::PgConnectOptions,
    login: &str,
    role: &str,
    schema: &str,
) -> Result {
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
    for profile in [legacy, complete] {
        let profile = profile.with_setting("app.tenant", "restricted")?;
        let mut connection = bounded(PgConnection::connect_with(&options)).await??;
        let apply = bounded(profile.reset_and_apply(&mut connection))
            .await
            .and_then(|result| result.map_err(Into::into));
        let close = bounded(connection.close())
            .await
            .and_then(|result| result.map_err(Into::into));
        combine(apply, close)?;
        let database = PgProfiledPool::connect_lazy(
            options.clone(),
            profile,
            PgPoolOptions::new().max_connections(1),
        )?;
        let body =
            async {
                let mut original = None;
                for _ in 0..2 {
                    let mut connection = bounded(database.pool().acquire()).await??;
                    let (pid, effective, tenant): (i32, String, String) = sqlx::query_as(
                    "SELECT pg_backend_pid(), current_user::text, current_setting('app.tenant')",
                ).fetch_one(&mut *connection).await?;
                    require(
                        effective == role && tenant == "restricted",
                        "profile policy was not applied",
                    )?;
                    require(
                        *original.get_or_insert(pid) == pid,
                        "idle acquisition must reuse the session",
                    )?;
                    drop(connection);
                    bounded(async {
                        while database.pool().num_idle() != 1 {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await?;
                }
                Ok(())
            }
            .await;
        combine(body, bounded(database.pool().close()).await)?;
    }
    Ok(())
}
