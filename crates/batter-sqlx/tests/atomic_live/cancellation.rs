use super::*;
use batter_sqlx::PgReadOnlySnapshot;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn unpolled_consuming_scopes_retire_the_owner() -> Result {
    let fixture = fixture().await?;
    let body = async {
        for mode in 0..4 {
            let (tx, previous) = transaction(&fixture.pool).await?;
            match mode {
                0 => drop(tx.application(async |_| Ok::<_, sqlx::Error>(()))),
                1 => drop(tx.operation(async |_| Ok::<_, sqlx::Error>(()))),
                2 => drop(tx.commit()),
                _ => drop(tx.rollback()),
            }
            replacement(&fixture.pool, previous).await?;
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn cancelled_application_operation_and_snapshot_retire() -> Result {
    for mode in 0..3 {
        let mut fixture = fixture().await?;
        let pool = fixture.pool.clone();
        let key = fixture.key;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let work = async move |sql: &mut PgScopedSql<'_>| {
            let backend = pid(sql).await?;
            let _ = sender.send(backend);
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(key)
                .execute(sql.executor())
                .await?;
            Ok::<_, sqlx::Error>(())
        };
        let task = tokio::spawn(async move {
            if mode == 2 {
                let _ = PgReadOnlySnapshot::inspect(&pool, work).await;
            } else {
                let tx = PgAtomicTransaction::begin(&pool).await.unwrap();
                if mode == 0 {
                    let _ = tx.application(work).await;
                } else {
                    let _ = tx.operation(work).await;
                }
            }
        });
        let body = async {
            let previous = support::bounded(receiver).await??;
            fixture.retired.push(previous);
            fixture.blocked(previous).await?;
            task.abort();
            assert!(support::bounded(task).await?.unwrap_err().is_cancelled());
            fixture.replacement_and_close().await
        }
        .await;
        fixture.finish(body).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn panicking_scope_retires_the_owner() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let (tx, previous) = transaction(&fixture.pool).await?;
        let task = tokio::spawn(tx.application(
            async |_| -> std::result::Result<(), sqlx::Error> {
                panic!("deliberate owned-scope panic")
            },
        ));
        assert!(task.await.unwrap_err().is_panic());
        replacement(&fixture.pool, previous).await
    }
    .await;
    fixture.finish(body).await
}
