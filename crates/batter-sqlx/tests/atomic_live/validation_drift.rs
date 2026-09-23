use super::{fixture, support::Result};
use batter_sqlx::{
    PgScopeError, PgScopeFailure, PgSessionProfile, PgTransactionError,
    low_level::PgAtomicTransaction,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn combined_validation_rejects_each_setting_and_recovers_local_changes() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = PgSessionProfile::with_timeouts(
            &login, &login, vec!["public".into()], Duration::ZERO, Duration::ZERO,
            Duration::ZERO, Duration::ZERO,
        )?.with_setting("app.tenant", "original")?
            .with_setting("application_name", "original")?
            .with_setting("timezone", "UTC")?;
        for (key, value) in [
            ("search_path", "pg_catalog"),
            ("statement_timeout", "10000ms"),
            ("lock_timeout", "10000ms"),
            ("idle_in_transaction_session_timeout", "10000ms"),
            ("transaction_timeout", "10000ms"),
            ("default_transaction_isolation", "serializable"),
            ("default_transaction_read_only", "on"),
            ("default_transaction_deferrable", "on"),
            ("row_security", "off"),
            ("app.tenant", "changed"),
            ("application_name", "changed"),
            ("timezone", "Europe/Prague"),
        ] {
            let tx = PgAtomicTransaction::begin_profiled(&fixture.pool, &profile).await?;
            let error = tx.application(async |sql| {
                sqlx::query("SELECT pg_catalog.set_config($1, $2, true)")
                    .bind(key).bind(value).execute(sql.executor()).await
            }).await.unwrap_err();
            assert!(matches!(error, PgScopeError::Terminal(PgScopeFailure::Transaction(cause))
                if matches!(*cause, PgTransactionError::Query(_))), "{key}");

            let tx = PgAtomicTransaction::begin_profiled(&fixture.pool, &profile).await?;
            let (tx, rejection) = tx.operation(async |sql| {
                sqlx::query("SELECT pg_catalog.set_config($1, $2, true)")
                    .bind(key).bind(value).execute(sql.executor()).await?;
                Err::<(), _>(sqlx::Error::Protocol("retained rejection".into()))
            }).await?;
            assert!(matches!(rejection, Err(sqlx::Error::Protocol(message)) if message == "retained rejection"));
            // The combined validation after recovery must observe restored state.
            let _ = tx.commit().await?;
        }
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn combined_validation_rejects_role_and_schema_privilege_drift() -> Result {
    let mut fixture = fixture().await?;
    let role = format!("validation_role_{}", fixture.key);
    let schema = format!("validation_schema_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role}; CREATE SCHEMA {schema}; GRANT USAGE ON SCHEMA {schema} TO {role}"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = PgSessionProfile::new(
            &login,
            &role,
            vec![schema.clone(), "public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )?;
        for statement in [
            "RESET ROLE".to_owned(),
            format!("SET SESSION AUTHORIZATION {role}"),
        ] {
            let tx = PgAtomicTransaction::begin_profiled(&fixture.pool, &profile).await?;
            let result = tx
                .application(async |sql| {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(statement.clone()))
                        .execute(sql.executor())
                        .await
                })
                .await;
            assert!(
                matches!(result, Err(PgScopeError::Terminal(_))),
                "{statement}"
            );
        }
        let tx = PgAtomicTransaction::begin_profiled(&fixture.pool, &profile).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE USAGE ON SCHEMA {schema} FROM {role}"
        )))
        .execute(&mut fixture.observer)
        .await?;
        let result = tx
            .application(async |_| {
                panic!("revoked schema access must be rejected before work");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            })
            .await;
        assert!(matches!(result, Err(PgScopeError::Terminal(_))));
        Ok(())
    }
    .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {schema}; DROP ROLE {role}"
    )))
    .execute(&mut fixture.observer)
    .await?;
    fixture.finish(body).await
}
