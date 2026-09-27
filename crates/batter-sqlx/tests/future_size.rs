//! Canonical PostgreSQL futures stay small when consumers compose them.
//!
//! The workspace suite runs these checks unoptimized and the verification
//! matrix repeats them with `--release`. Futures are built against a lazy pool
//! and dropped unpolled, so no database is involved. Each entry point erases its
//! whole operation once on first poll; its future keeps only its arguments and
//! one pointer, however large the native or callback state behind it.

use batter_core::operation::{OperationContext, OperationOwner};
use batter_sqlx::{
    PgFailurePolicy, PgLease, PgProfiledPool, PgQueryHandle, PgScopeFailure, PgScopeLoss,
    PgScopeRolledBack, PgSessionProfile, PgTransactionError,
};
use std::{future::Future, time::Duration};

/// Arguments (including a native query value) plus one pointer. Unerased,
/// these futures measured 2.6-79 KB on Rust 1.98.1.
const ERASED: usize = 1024;

fn bounded(name: &str, future: impl Future) {
    let size = std::mem::size_of_val(&future);
    assert!(
        size <= ERASED,
        "{name} future is {size} bytes; bound is {ERASED}"
    );
}

// The eight-bind scalar shape from the original consumer report.
fn query() -> sqlx::query::QueryScalar<'static, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
    (1..=8).fold(
        sqlx::query_scalar("SELECT $1::bigint + $2 + $3 + $4 + $5 + $6 + $7 + $8"),
        |query, value: i64| query.bind(value),
    )
}

struct Rejected;

impl From<PgScopeRolledBack> for Rejected {
    fn from(_: PgScopeRolledBack) -> Self {
        Self
    }
}

impl From<sqlx::Error> for Rejected {
    fn from(_: sqlx::Error) -> Self {
        Self
    }
}

struct Policy;

impl PgFailurePolicy<i64> for Policy {
    type Error = Rejected;
    fn begin_failed(&self, _: PgTransactionError) -> Rejected {
        Rejected
    }
    fn scope_lost(&self, _: PgScopeFailure<Rejected>) -> Rejected {
        Rejected
    }
    fn commit_unconfirmed(&self, _: i64, _: PgTransactionError) -> Rejected {
        Rejected
    }
    fn rollback_unconfirmed(&self, _: Rejected, _: PgTransactionError) -> Rejected {
        Rejected
    }
    fn scope_lost_after_body(&self, _: Result<i64, Rejected>, _: PgScopeLoss) -> Rejected {
        Rejected
    }
}

struct Fixture {
    pool: sqlx::PgPool,
    database: PgProfiledPool,
    owner: OperationOwner,
}

impl Fixture {
    fn new() -> Self {
        let profile = PgSessionProfile::new(
            "login",
            "serving",
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        Self {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused@127.0.0.1:1/unused")
                .unwrap(),
            database: PgProfiledPool::connect_lazy(
                "postgres://login@127.0.0.1:1/unused".parse().unwrap(),
                profile,
                sqlx::postgres::PgPoolOptions::new().min_connections(0),
            )
            .unwrap(),
            owner: OperationOwner::new(Duration::from_secs(5)).unwrap(),
        }
    }

    fn context(&self) -> &OperationContext {
        self.owner.context()
    }
}

#[tokio::test]
async fn lease_and_pooled_query_futures_hold_only_arguments_and_one_allocation() {
    let fixture = Fixture::new();
    let (pool, context) = (&fixture.pool, fixture.context());
    bounded("PgLease::acquire", PgLease::acquire(pool, context));
    let queries =
        PgQueryHandle::within(pool, context, Duration::from_secs(1), "size.query", |e| e).unwrap();
    bounded("PgQueryHandle::fetch_one", queries.fetch_one(query()));
    bounded(
        "PgQueryHandle::fetch_optional",
        queries.fetch_optional(query()),
    );
    bounded("PgQueryHandle::fetch_all", queries.fetch_all(query()));
    bounded("PgQueryHandle::execute", queries.execute(query()));
    assert_eq!(pool.size(), 0, "unpolled futures must not acquire");
}

#[tokio::test]
async fn atomic_runner_futures_keep_callback_state_behind_one_allocation() {
    let fixture = Fixture::new();
    let (pool, database, context) = (&fixture.pool, &fixture.database, fixture.context());
    let profile = database.profile();
    bounded(
        "run_atomic",
        batter_sqlx::run_atomic(pool, async |scope| scope.fetch_one(query()).await),
    );
    bounded(
        "run_atomic_profiled",
        batter_sqlx::run_atomic_profiled(pool, profile, async |scope| {
            scope.fetch_one(query()).await
        }),
    );
    bounded(
        "run_atomic_in",
        batter_sqlx::run_atomic_in(pool, context, "size.atomic", async |scope| {
            scope.fetch_one(query()).await
        }),
    );
    bounded(
        "run_atomic_profiled_in",
        batter_sqlx::run_atomic_profiled_in(database, context, "size.atomic", async |scope| {
            scope.fetch_one(query()).await
        }),
    );
    // Callback state that lives across an await stays in the heap allocation.
    bounded(
        "run_atomic_in with 16 KiB of callback state",
        batter_sqlx::run_atomic_in(pool, context, "size.atomic", async |_| {
            let state = [7_u8; 16 * 1024];
            tokio::task::yield_now().await;
            Ok::<_, ()>(state[0])
        }),
    );
    assert_eq!(pool.size() + database.pool().size(), 0);
}

#[tokio::test]
async fn policy_and_fail_fast_runner_futures_keep_one_allocation() {
    let fixture = Fixture::new();
    let (pool, database, context) = (&fixture.pool, &fixture.database, fixture.context());
    let profile = database.profile();
    let policy = &Policy;
    bounded(
        "run_atomic_with",
        batter_sqlx::run_atomic_with(pool, policy, async |mut scope| {
            scope.fetch_one(query()).await
        }),
    );
    bounded(
        "run_atomic_with_in",
        batter_sqlx::run_atomic_with_in(pool, context, "size.policy", policy, async |mut scope| {
            scope.fetch_one(query()).await
        }),
    );
    bounded(
        "run_atomic_profiled_with",
        batter_sqlx::run_atomic_profiled_with(pool, profile, policy, async |mut scope| {
            scope.fetch_one(query()).await
        }),
    );
    bounded(
        "run_atomic_profiled_with_in",
        batter_sqlx::run_atomic_profiled_with_in(
            database,
            context,
            "size.policy",
            policy,
            async |mut scope| scope.fetch_one(query()).await,
        ),
    );
    bounded(
        "run_atomic_fail_fast_with",
        batter_sqlx::run_atomic_fail_fast_with(pool, policy, async |mut scope| {
            scope.fetch_one(query()).await
        }),
    );
    bounded(
        "run_atomic_fail_fast_with_in",
        batter_sqlx::run_atomic_fail_fast_with_in(
            pool,
            context,
            "size.fast",
            policy,
            async |mut scope| scope.fetch_one(query()).await,
        ),
    );
    bounded(
        "run_atomic_profiled_fail_fast_with",
        batter_sqlx::run_atomic_profiled_fail_fast_with(
            pool,
            profile,
            policy,
            async |mut scope| scope.fetch_one(query()).await,
        ),
    );
    bounded(
        "run_atomic_profiled_fail_fast_with_in",
        batter_sqlx::run_atomic_profiled_fail_fast_with_in(
            database,
            context,
            "size.fast",
            policy,
            async |mut scope| scope.fetch_one(query()).await,
        ),
    );
    assert_eq!(pool.size() + database.pool().size(), 0);
}
