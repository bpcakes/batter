//! External-style composition of quota admission, a bounded pooled query and an
//! atomic workflow inside one Axum handler behind the canonical HTTP boundary.
//!
//! `scripts/check_facade_features.py` also copies this file into an independent
//! consumer crate and runs it unoptimized and optimized there: at rustc's default
//! recursion limit, on the default test-thread stack, without future erasure.
//! Keep it free of crate recursion limits, stack settings and boxed futures.

use axum::{
    Extension,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    routing::post,
};
use batter::{
    axum::{GuardedRouter, HttpBoundary, RequestPolicy, ResponseConstructionBudget},
    lifecycle::ShutdownHandle,
    operation::OperationContext,
    runlimit::{Checks, Quota, RunResult},
    sqlx::{PgQueryHandle, run_atomic_in},
};
use runlimit_core::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
use runlimit_memory::{MemoryStore, MemoryStoreConfig};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Records {
    pool: sqlx::PgPool,
    quota: Quota<MemoryStore>,
    policy: FixedWindowPolicy,
    hasher: KeyHasher,
    admitted: AtomicUsize,
}

#[derive(Debug)]
enum RecordError {
    Read,
    Write,
}

// The eight-bind scalar shape from the original consumer report.
fn total() -> sqlx::query::QueryScalar<'static, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
    (1..=8).fold(
        sqlx::query_scalar("SELECT $1::bigint + $2 + $3 + $4 + $5 + $6 + $7 + $8"),
        |query, value: i64| query.bind(value),
    )
}

async fn record(
    State(records): State<Arc<Records>>,
    Extension(context): Extension<OperationContext>,
) -> StatusCode {
    let records = &*records;
    let checks = [Check::new(
        records.hasher.hash_for(&records.policy, "owner-a"),
    )];
    let checks = Checks::new(&checks).expect("one quota check");
    let result = records
        .quota
        .run(&context, checks, |work| async move {
            records.admitted.fetch_add(1, Ordering::SeqCst);
            let stored = run_atomic_in(&records.pool, &work, "records.write", async |scope| {
                scope.fetch_one(total()).await
            })
            .await
            .map_err(|_| RecordError::Write);
            let reads = PgQueryHandle::within(
                &records.pool,
                &work,
                Duration::from_secs(1),
                "records.read",
                |_| RecordError::Read,
            )
            .expect("positive query budget");
            let read = reads.fetch_one(total()).await;
            Ok::<_, RecordError>(stored? + read?)
        })
        .await;
    match result {
        RunResult::Admitted { work: Ok(_), .. } => StatusCode::OK,
        RunResult::Admitted { work: Err(_), .. } => StatusCode::SERVICE_UNAVAILABLE,
        RunResult::Rejected { .. } => StatusCode::TOO_MANY_REQUESTS,
        RunResult::Backend { .. } | RunResult::Interrupted { .. } => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[tokio::test]
async fn quota_query_and_atomic_workflow_compose_in_one_registered_handler() {
    // No database is reachable: both operations fail at acquisition after the
    // quota admitted work, which still drives every adapter layer on this thread.
    let records = Arc::new(Records {
        pool: sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://consumer@127.0.0.1:1/unused")
            .expect("lazy pool"),
        quota: Quota::new(MemoryStore::new(MemoryStoreConfig::new(16).unwrap())),
        policy: FixedWindowPolicy::new(
            PolicyId::new("records.write").unwrap(),
            ScopeId::new("owner").unwrap(),
            1,
            Duration::from_secs(3600),
        )
        .unwrap(),
        hasher: KeyHasher::new([7; 32]).unwrap(),
        admitted: AtomicUsize::new(0),
    });
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let budget = ResponseConstructionBudget::new(Duration::from_secs(10)).unwrap();
    let guarded = GuardedRouter::new()
        .route("/records", post(record))
        .with_state(records.clone());
    let client = HttpBoundary::new(RequestPolicy::new(control.operation_admission(), budget))
        .assemble(guarded)
        .await
        .expect("guarded routes assemble")
        .in_process();
    let mut statuses = Vec::new();
    for _ in 0..2 {
        let request = Request::post("/records").body(Body::empty()).unwrap();
        statuses.push(client.request(request).await.status());
    }
    assert_eq!(
        statuses,
        [
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::TOO_MANY_REQUESTS
        ]
    );
    assert_eq!(records.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(records.pool.size(), 0);
}
