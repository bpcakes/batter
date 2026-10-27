//! Mutation, transaction escape and interruption cannot become apply success.

use super::*;
use batter_core::operation::{OperationError, OperationOwner};
use batter_sqlx::managed::Retirement;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_application_cannot_change_definitions_ledger_or_public_policy() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let pool = fixture.pool(Role::Migrator).await?;
        let original = fixture.objects_with(
            PublicExecute::Revoke,
            fixture.standard_objects(),
            [Retirement::function(RoutineSignature::new(&schema, "retired_total", [])?)],
        )?;
        let _ = fixture.apply(&pool, &original.manifest).await?;
        let before = catalog_state(&pool,&schema).await?;
        let before_ledger = ledger(&pool,&schema).await?;
        for statement in [
            format!("CREATE OR REPLACE FUNCTION {schema}.record_total(extra bigint) RETURNS bigint LANGUAGE sql RETURN 99"),
            format!("ALTER TABLE {schema}.records DISABLE TRIGGER records_touch"),
            format!("UPDATE {schema}.managed_objects SET rendered_checksum='\\x00'::bytea"),
            format!("UPDATE {schema}.managed_objects SET applied_at=applied_at + interval '1 second'"),
            format!("GRANT EXECUTE ON FUNCTION {schema}.record_total(bigint) TO PUBLIC"),
            format!("CREATE FUNCTION {schema}.retired_total() RETURNS bigint LANGUAGE sql RETURN 0"),
        ] {
            let error = apply(fixture,&pool,&original.manifest,async |sql| {
                sqlx::raw_sql(AssertSqlSafe(statement)).execute(sql.executor()).await?;
                Ok(())
            }).await.unwrap_err();
            require(matches!(error,ManagedApplyWithError::ManagedStateChanged), "managed mutation accepted or misclassified")?;
            require(catalog_state(&pool,&schema).await? == before, "definition or ACL mutation escaped")?;
            require(ledger(&pool,&schema).await? == before_ledger, "ledger mutation escaped")?;
        }
        // Work deferred by callback SQL must run before the final verification.
        // Otherwise a COMMIT-time trigger could change a certified definition.
        fixture.admin(&format!(
            "CREATE FUNCTION {schema}.deferred_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN EXECUTE 'CREATE OR REPLACE FUNCTION {schema}.record_total(extra bigint) RETURNS bigint LANGUAGE sql RETURN 99'; RETURN NULL; END $$;
             CREATE CONSTRAINT TRIGGER deferred_mutation AFTER INSERT ON {schema}.records
             DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION {schema}.deferred_mutation()"
        )).await?;
        let before = catalog_state(&pool,&schema).await?;
        let error = apply(fixture,&pool,&original.manifest,async |sql| {
            sqlx::raw_sql(AssertSqlSafe(format!("INSERT INTO {schema}.records(note) VALUES ('pending')")))
                .execute(sql.executor()).await?;
            Ok(())
        }).await.unwrap_err();
        require(matches!(error,ManagedApplyWithError::ManagedStateChanged), "deferred mutation escaped verification")?;
        require(catalog_state(&pool,&schema).await? == before, "deferred mutation committed")
    }).await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_application_transaction_escape_is_unconfirmed() -> Result {
    for control in ["COMMIT", "ROLLBACK", "COMMIT; BEGIN", "ROLLBACK; BEGIN"] {
        for rejected in [false, true] {
            Fixture::run(async |fixture| {
                let pool = fixture.pool(Role::Migrator).await?;
                let objects = fixture.objects(PublicExecute::Revoke);
                let previous = pid(&pool).await?;
                let error = apply(fixture, &pool, &objects.manifest, async |sql| {
                    sqlx::raw_sql(AssertSqlSafe(control.to_owned()))
                        .execute(sql.executor())
                        .await?;
                    if rejected {
                        Err(sqlx::Error::Protocol("retained-refusal".into()))
                    } else {
                        Ok(())
                    }
                })
                .await
                .unwrap_err();
                let ManagedApplyWithError::Unconfirmed { application, .. } = error else {
                    return Err(
                        std::io::Error::other("escaped transaction became known outcome").into(),
                    );
                };
                require(
                    application.is_some() == rejected,
                    "escape lost callback refusal",
                )?;
                require(
                    pid(&pool).await? != previous,
                    "escaped connection was reused",
                )?;
                let published = fixture.exists(&pool, "record_total").await?;
                require(
                    published == control.starts_with("COMMIT"),
                    "escape evidence contradicts actual publication",
                )
            })
            .await?;
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_application_cancellation_retires_and_preserves_old_state() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let pool = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let original = fixture.objects(PublicExecute::Revoke);
        let _ = fixture.apply(&pool, &original.manifest).await?;
        let before = catalog_state(&observer, &schema).await?;
        let before_ledger = ledger(&observer, &schema).await?;
        let mut objects = fixture.standard_objects();
        objects.push(function(fixture, "private_total", "RETURN 7;")?);
        let changed = fixture.objects_with(PublicExecute::Revoke, objects, [])?;
        let previous = pid(&pool).await?;
        let owner = OperationOwner::new(Duration::from_secs(30))?;
        let context = owner.context();
        let lease = PgLease::acquire(&pool, context).await?;
        let bundle = fixture.migrator_bundle();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let applying = context.run("test.managed-application", |_| {
            lease.migrate_and_apply_with(&bundle, &changed.manifest, async |sql| {
                sqlx::raw_sql(AssertSqlSafe(format!(
                    "INSERT INTO {schema}.records(note) VALUES ('uncommitted')"
                )))
                .execute(sql.executor())
                .await?;
                let _ = entered_tx.send(());
                std::future::pending::<()>().await;
                Ok::<_, sqlx::Error>(())
            })
        });
        let cancel = async {
            entered_rx.await?;
            owner.cancel();
            Ok::<_, tokio::sync::oneshot::error::RecvError>(())
        };
        let (outcome, cancelled) = tokio::join!(applying, cancel);
        cancelled?;
        require(
            matches!(outcome, Err(OperationError::Interrupted(_))),
            "cancellation was not retained",
        )?;
        require(
            pid(&pool).await? != previous,
            "cancelled lease returned to pool",
        )?;
        // Actual server settlement, not an inference from local retirement.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1)",
                )
                .bind(previous)
                .fetch_one(&observer)
                .await?;
                if !exists {
                    return Ok::<_, sqlx::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        require(
            catalog_state(&observer, &schema).await? == before,
            "cancelled definitions committed",
        )?;
        require(
            ledger(&observer, &schema).await? == before_ledger,
            "cancelled ledger committed",
        )?;
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT count(*) FROM {schema}.records"
        )))
        .fetch_one(&observer)
        .await?;
        require(count == 0, "cancelled callback data committed")
    })
    .await
}
