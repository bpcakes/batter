//! Canonical quota and attempt futures stay small when consumers compose them.
//!
//! The workspace suite runs these checks unoptimized and the verification
//! matrix repeats them with `--release`. Futures are built against lazy pools and
//! dropped unpolled. Native checks, native admission and admitted work are each
//! erased once, so consumer work state never enters the quota future.

use batter_core::operation::{OperationContext, OperationOwner};
use batter_runlimit::{
    Checks, Quota,
    attempts::{AttemptRunner, Authentication},
};
use batter_sqlx::{PgProfiledPool, PgQueryHandle, PgSessionProfile};
use runlimit_core::{
    Check, FixedWindowPolicy, KeyHasher, PolicyId, QuotaPeriod, ScopeId, attempts::AttemptPolicy,
};
use runlimit_memory::{MemoryStore, MemoryStoreConfig};
use runlimit_postgres::PostgresLimiter;
use std::{convert::Infallible, future::Future, time::Duration};

/// Unerased, the pooled query and `run_atomic_in` futures alone measured 24-79 KB
/// and `AttemptRunner::run` 28 KB.
const CANONICAL: usize = 4 * 1024;

fn bounded(name: &str, future: impl Future) {
    let size = std::mem::size_of_val(&future);
    assert!(
        size <= CANONICAL,
        "{name} future is {size} bytes; bound is {CANONICAL}"
    );
}

fn lazy_pool() -> sqlx::PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap()
}

fn context() -> OperationContext {
    OperationOwner::new(Duration::from_secs(5))
        .unwrap()
        .into_context()
}

fn eight_bind_query()
-> sqlx::query::QueryScalar<'static, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
    (1..=8).fold(
        sqlx::query_scalar("SELECT $1::bigint + $2 + $3 + $4 + $5 + $6 + $7 + $8"),
        |query, value: i64| query.bind(value),
    )
}

#[tokio::test]
async fn quota_futures_erase_native_checks_and_admitted_work() {
    let policy = FixedWindowPolicy::new(
        PolicyId::new("size.quota").unwrap(),
        ScopeId::new("owner").unwrap(),
        5,
        Duration::from_secs(60),
    )
    .unwrap();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let checks = [Check::new(hasher.hash_for(&policy, "owner-a"))];
    let context = context();
    let memory = Quota::new(MemoryStore::new(MemoryStoreConfig::new(100).unwrap()));
    bounded(
        "Quota<MemoryStore>::run",
        memory.run(&context, Checks::new(&checks).unwrap(), |_| async {
            Ok::<_, Infallible>(1)
        }),
    );
    let pool = lazy_pool();
    let postgres = Quota::new(PostgresLimiter::new(pool.clone()));
    bounded(
        "Quota<PostgresLimiter>::run",
        postgres.run(&context, Checks::new(&checks).unwrap(), |_| async {
            Ok::<_, Infallible>(1)
        }),
    );
    // Work state that lives across an await stays in the heap allocation.
    bounded(
        "Quota::run with 16 KiB of work state",
        memory.run(&context, Checks::new(&checks).unwrap(), |_| async {
            let state = [7_u8; 16 * 1024];
            tokio::task::yield_now().await;
            Ok::<_, Infallible>(state[0])
        }),
    );
    // The handler shape from the original report: quota around a pooled query
    // and an atomic workflow, all under one parent operation budget.
    bounded(
        "Quota::run composing PgQueryHandle and run_atomic_in",
        memory.run(&context, Checks::new(&checks).unwrap(), |work| {
            let pool = &pool;
            async move {
                let queries =
                    PgQueryHandle::within(pool, &work, Duration::from_secs(1), "size.read", |_| ())
                        .unwrap();
                let total = queries.fetch_one(eight_bind_query()).await?;
                let stored = batter_sqlx::run_atomic_in(pool, &work, "size.write", async |scope| {
                    scope.fetch_one(eight_bind_query()).await
                })
                .await
                .map_err(|_| ())?;
                Ok::<_, ()>(total + stored)
            }
        }),
    );
    assert_eq!(pool.size(), 0, "unpolled futures must not acquire");
}

#[tokio::test]
async fn attempt_runner_future_erases_native_admission_and_completion() {
    let profile = PgSessionProfile::new(
        "login",
        "login",
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let database = PgProfiledPool::connect_lazy(
        "postgres://login@127.0.0.1:1/unused".parse().unwrap(),
        profile,
        sqlx::postgres::PgPoolOptions::new().min_connections(0),
    )
    .unwrap();
    let period = |seconds| QuotaPeriod::new(Duration::from_secs(seconds)).unwrap();
    let policy = AttemptPolicy::new(
        PolicyId::new("size.attempt").unwrap(),
        ScopeId::new("identifier").unwrap(),
        period(1),
        period(60),
        period(300),
        period(30),
    )
    .unwrap();
    let hasher = KeyHasher::new([7; 32]).unwrap();
    let runner = AttemptRunner::new(database.clone()).unwrap();
    let context = context();
    bounded(
        "AttemptRunner::run",
        runner.run(
            &context,
            hasher.hash_attempt_for(&policy, "normalized-identifier"),
            |_| async { Ok::<_, Infallible>(true) },
            async |sql, matches| {
                sqlx::query("SELECT 1").execute(sql.executor()).await?;
                Ok::<Authentication<(), ()>, sqlx::Error>(if matches {
                    Authentication::Accepted(())
                } else {
                    Authentication::Rejected(())
                })
            },
        ),
    );
    assert_eq!(database.pool().size(), 0);
}
