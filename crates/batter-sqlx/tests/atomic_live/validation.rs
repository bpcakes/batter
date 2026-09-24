use super::{fixture, support::Result};
use batter_sqlx::{PgSessionProfile, low_level::PgAtomicTransaction};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tracing::{Event, Subscriber};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

#[derive(Clone, Default)]
struct Statements(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for Statements {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() == "sqlx::query" {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

pub(super) async fn counted<T>(expected: usize, future: impl Future<Output = T>) -> T {
    let statements = Statements::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(statements.clone()));
    let future = tracing::dispatcher::with_default(&dispatch, || {
        batter_core::telemetry::with_current_dispatch(future)
    });
    let result = future.await;
    assert_eq!(statements.0.load(Ordering::SeqCst), expected);
    result
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiled_schema_lookup_remains_index_eligible() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        // A small test catalog may favor a sequential scan even for an indexed
        // predicate. Disabling that plan exposes whether this query can use the
        // namespace-name index at all.
        sqlx::raw_sql("SET enable_seqscan = off")
            .execute(&mut fixture.observer)
            .await?;
        let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (VERBOSE, COSTS OFF) {}",
            include_str!("../../src/profile/validation.sql")
        )))
        .bind(vec!["public"])
        .bind(Vec::<String>::new())
        .bind(Vec::<String>::new())
        .fetch_all(&mut fixture.observer)
        .await?;
        assert!(
            plan.iter()
                .any(|line| line.contains("Index Scan using pg_namespace_nspname_index")),
            "{plan:#?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("Index Cond: (n.nspname = declared.name)")),
            "{plan:#?}"
        );
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn atomic_scope_statement_counts_include_recovery_and_completion() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let legacy = PgSessionProfile::new(
            &login,
            &login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )?;
        let complete = PgSessionProfile::with_timeouts(
            &login,
            &login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        )?
        .with_setting("app.tenant", "counted")?
        .with_setting("timezone", "UTC")?;
        for profile in [None, Some(&legacy), Some(&complete)] {
            let mut tx = if let Some(profile) = profile {
                PgAtomicTransaction::begin_profiled(&fixture.pool, profile).await?
            } else {
                PgAtomicTransaction::begin(&fixture.pool).await?
            };
            let count = if profile.is_some() { 5 } else { 4 };
            // First, repeated and post-recovery scopes have the same cost.
            for _ in 0..2 {
                let (owner, value) = counted(
                    count,
                    tx.application(async |sql| {
                        sqlx::query_scalar::<_, i32>("SELECT 42")
                            .fetch_one(sql.executor())
                            .await
                    }),
                )
                .await?;
                assert_eq!(value, 42);
                tx = owner;
            }
            let (owner, rejection) = counted(
                count + 1,
                tx.operation(async |sql| sqlx::query("SELECT 1 / 0").execute(sql.executor()).await),
            )
            .await?;
            assert_eq!(
                rejection
                    .unwrap_err()
                    .as_database_error()
                    .unwrap()
                    .code()
                    .as_deref(),
                Some("22012")
            );
            let (owner, _) = counted(
                count,
                owner
                    .application(async |sql| sqlx::query("SELECT 1").execute(sql.executor()).await),
            )
            .await?;
            let _ = counted(2, owner.commit()).await?;
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profiled_scope_checks_external_schema_loss_before_work() -> Result {
    let mut fixture = fixture().await?;
    let schema = format!("validation_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&mut fixture.observer)
        .await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = PgSessionProfile::new(
            &login,
            &login,
            vec!["public".into(), schema.clone()],
            Duration::ZERO,
            Duration::ZERO,
        )?;
        let tx = PgAtomicTransaction::begin_profiled(&fixture.pool, &profile).await?;
        let (tx, ()) = tx.application(async |_| Ok::<_, sqlx::Error>(())).await?;
        // No SQL runs through the owner between validation and this next scope.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema}")))
            .execute(&mut fixture.observer)
            .await?;
        let result = counted(
            1,
            tx.application(async |_| {
                panic!("catalog drift must be rejected before invoking work");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(batter_sqlx::PgScopeError::Terminal(_))
        ));
        Ok(())
    }
    .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema}"
    )))
    .execute(&mut fixture.observer)
    .await?;
    fixture.finish(body).await
}
