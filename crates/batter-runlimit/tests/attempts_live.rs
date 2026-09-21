//! Explicit PostgreSQL 18 acceptance. Run with a dedicated disposable database:
//! DATABASE_URL=... cargo test -p batter-runlimit --features postgres --test attempts_live -- --ignored
use batter_core::operation::OperationContext;
use batter_runlimit::attempts::{
    AttemptError, AttemptPhase, AttemptRunner, AttemptWriteError, Authentication,
};
use batter_sqlx::{PgAtomicError, PgProfiledPool, PgScopeError, PgSessionProfile};
use runlimit_core::{KeyHasher, PolicyId, QuotaPeriod, ScopeId, attempts::*};
use runlimit_postgres::attempts::PostgresAttemptLimiter;
use sqlx::PgPool;
use std::{
    convert::Infallible,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

static INITIALIZED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

#[path = "attempts_live/profile.rs"]
mod profile;

struct Fixture {
    database: PgProfiledPool,
    pool: PgPool,
    runner: AttemptRunner,
    native: PostgresAttemptLimiter,
    policy: AttemptPolicy,
    hasher: KeyHasher,
    subject: String,
}

impl Fixture {
    async fn new(case: &str, lease_ms: u64) -> Self {
        let url = std::env::var("DATABASE_URL").expect("explicit live suite requires DATABASE_URL");
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        let login = options.get_username();
        let profile = PgSessionProfile::new(
            login,
            login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        let database = PgProfiledPool::connect(options, profile, 8).await.unwrap();
        let pool = database.pool().clone();
        let native = PostgresAttemptLimiter::new(pool.clone());
        INITIALIZED.get_or_init(|| async {
            let version: String = sqlx::query_scalar("SHOW server_version_num")
                .fetch_one(&pool).await.unwrap();
            assert_eq!(version.parse::<u32>().unwrap() / 10_000, 18);
            native.migrate().await.unwrap();
            sqlx::query("CREATE TABLE IF NOT EXISTS batter_attempt_events (subject text NOT NULL, kind text NOT NULL)")
                .execute(&pool).await.unwrap();
            sqlx::query("CREATE TABLE IF NOT EXISTS batter_attempt_parents (id integer PRIMARY KEY)")
                .execute(&pool).await.unwrap();
            sqlx::query("CREATE TABLE IF NOT EXISTS batter_attempt_deferred (id integer REFERENCES batter_attempt_parents DEFERRABLE INITIALLY DEFERRED)")
                .execute(&pool).await.unwrap();
        }).await;
        let unique: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
            .fetch_one(&pool)
            .await
            .unwrap();
        let period = |ms| QuotaPeriod::new(Duration::from_millis(ms)).unwrap();
        let policy = AttemptPolicy::new(
            PolicyId::new("credential.verify").unwrap(),
            ScopeId::new("identifier").unwrap(),
            period(5),
            period(100),
            period(200),
            period(lease_ms),
        )
        .unwrap();
        Self {
            runner: AttemptRunner::new(database.clone()).unwrap(),
            database,
            pool,
            native,
            policy,
            hasher: KeyHasher::new([17; 32]).unwrap(),
            subject: format!("{case}-{unique}"),
        }
    }
    fn subject(&self) -> AttemptSubject<'_> {
        self.hasher.hash_attempt_for(&self.policy, &self.subject)
    }
    async fn event_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM batter_attempt_events WHERE subject=$1")
            .bind(&self.subject)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn context(ms: u64) -> OperationContext {
    OperationContext::new(Duration::from_millis(ms)).unwrap()
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn rejection_commits_audit_and_failure_then_success_resets_only_retry_state() {
    let f = Fixture::new("reject-success", 2000).await;
    let rejected = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async { Ok::<_, Infallible>(false) },
            async |sql, password_matches| {
                assert!(!password_matches);
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'rejected')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<(), &'static str>, sqlx::Error>(Authentication::Rejected(
                    "invalid",
                ))
            },
        )
        .await
        .unwrap();
    assert_eq!(rejected.completion().outcome(), AttemptOutcome::Failure);
    assert_eq!(rejected.completion().consecutive_failures(), 1);
    assert!(matches!(
        rejected.into_authentication(),
        Authentication::Rejected("invalid")
    ));
    assert_eq!(f.event_count().await, 1);
    tokio::time::sleep(Duration::from_millis(15)).await;
    let accepted = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async { Ok::<_, Infallible>(true) },
            async |sql, password_matches| {
                assert!(password_matches);
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'accepted')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<_, ()>, sqlx::Error>(Authentication::Accepted("session"))
            },
        )
        .await
        .unwrap();
    assert_eq!(accepted.completion().outcome(), AttemptOutcome::Success);
    assert_eq!(accepted.completion().consecutive_failures(), 0);
    assert!(matches!(
        accepted.into_authentication(),
        Authentication::Accepted("session")
    ));
    assert_eq!(
        f.event_count().await,
        2,
        "success must preserve the rejection audit"
    );
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn native_busy_denial_invokes_neither_factory() {
    let f = Fixture::new("busy", 2000).await;
    let AttemptAdmission::Admitted(_held) = f.native.admit(f.subject()).await.unwrap() else {
        panic!("admit")
    };
    let calls = AtomicUsize::new(0);
    let result = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Infallible>(())
            },
            async |_, ()| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(AttemptError::Denied(AttemptDenial::Busy { .. }))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn stale_claim_prevents_application_factory_and_session_writes() {
    let f = Fixture::new("stale", 20).await;
    let calls = AtomicUsize::new(0);
    let result = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok::<_, Infallible>(())
            },
            async |_, ()| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(AttemptError::Atomic(PgAtomicError::Rejected(
            PgScopeError::Application(AttemptWriteError::Stale)
        )))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn application_error_rolls_back_audit_and_does_not_reset_reservation() {
    let f = Fixture::new("rollback", 2000).await;
    let result = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async { Ok::<_, Infallible>(()) },
            async |sql, ()| {
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'must-rollback')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await
                    .unwrap();
                Err::<Authentication<(), ()>, _>("application unavailable")
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(AttemptError::Atomic(PgAtomicError::Rejected(
            PgScopeError::Application(AttemptWriteError::Application("application unavailable"))
        )))
    ));
    assert_eq!(f.event_count().await, 0);
    assert!(matches!(
        f.native.admit(f.subject()).await.unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn final_replay_rejection_after_password_verification_counts_as_failure() {
    let f = Fixture::new("replay", 2000).await;
    let result = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async { Ok::<_, Infallible>(true) },
            async |sql, verified_password| {
                assert!(verified_password);
                // The application's replay/account-state decision belongs inside this transaction.
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'replay-rejected')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<(), _>, sqlx::Error>(Authentication::Rejected("replay"))
            },
        )
        .await
        .unwrap();
    assert_eq!(result.completion().outcome(), AttemptOutcome::Failure);
    assert_eq!(result.completion().consecutive_failures(), 1);
    assert_eq!(f.event_count().await, 1);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn verification_and_transaction_share_one_parent_deadline() {
    let f = Fixture::new("budget", 2000).await;
    let result = f
        .runner
        .run(
            &context(100),
            f.subject(),
            |scope| async move {
                tokio::time::sleep(Duration::from_millis(40)).await;
                assert!(scope.remaining() < Duration::from_millis(70));
                Ok::<_, Infallible>(())
            },
            async |sql, ()| {
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'must-timeout')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await?;
                sqlx::query("SELECT pg_sleep(0.2)")
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(AttemptError::Interrupted {
            phase: AttemptPhase::Completion,
            ..
        })
    ));
    assert_eq!(f.event_count().await, 0);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn cancelled_verification_keeps_busy_lease_without_open_application_transaction() {
    let f = Fixture::new("cancel-verify", 2000).await;
    let operation = context(1000);
    let result = f
        .runner
        .run(
            &operation,
            f.subject(),
            |scope| async move {
                scope.cancel();
                std::future::pending::<Result<(), Infallible>>().await
            },
            async |_, ()| {
                panic!("cancelled verification must not invoke application transaction");
                #[allow(unreachable_code)]
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(AttemptError::Interrupted {
            phase: AttemptPhase::Verification,
            ..
        })
    ));
    assert!(matches!(
        f.native.admit(f.subject()).await.unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
    assert_eq!(f.event_count().await, 0);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn acknowledged_commit_survives_observer_cancellation_at_publication() {
    struct CancelAfterCommit(OperationContext);
    impl AttemptObserver for CancelAfterCommit {
        fn observe(&self, event: AttemptObservation) {
            if matches!(event, AttemptObservation::Completed(_)) {
                self.0.cancel();
            }
        }
    }
    let f = Fixture::new("commit-cancel", 2000).await;
    let operation = context(1000);
    let runner = AttemptRunner::new(f.database.clone())
        .unwrap()
        .with_observer(std::sync::Arc::new(CancelAfterCommit(operation.clone())));
    let fixture = &f;
    let output = operation
        .run("outer.request", |scope| async move {
            runner
                .run(
                    &scope,
                    fixture.subject(),
                    |_| async { Ok::<_, Infallible>(()) },
                    async |sql, ()| {
                        sqlx::query(
                            "INSERT INTO batter_attempt_events VALUES ($1,'commit-cancel')",
                        )
                        .bind(&fixture.subject)
                        .execute(sql.executor())
                        .await?;
                        Ok::<Authentication<_, ()>, sqlx::Error>(Authentication::Accepted(
                            "published",
                        ))
                    },
                )
                .await
        })
        .await
        .unwrap();
    assert!(matches!(
        output.into_authentication(),
        Authentication::Accepted("published")
    ));
    assert_eq!(f.event_count().await, 1);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn claim_before_expiry_holds_authority_through_slow_transaction() {
    let f = Fixture::new("claim-expiry", 100).await;
    let output = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async { Ok::<_, Infallible>(()) },
            async |sql, ()| {
                sqlx::query("SELECT pg_sleep(0.15)")
                    .execute(sql.executor())
                    .await?;
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'after-lease-time')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await
        .unwrap();
    assert_eq!(output.completion().outcome(), AttemptOutcome::Success);
    assert_eq!(f.event_count().await, 1);
    f.pool.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn unconfirmed_commit_retains_provisional_output_without_replaying() {
    let f = Fixture::new("commit-failure", 2000).await;
    let calls = AtomicUsize::new(0);
    let result = f
        .runner
        .run(
            &context(1000),
            f.subject(),
            |_| async { Ok::<_, Infallible>(()) },
            async |sql, ()| {
                calls.fetch_add(1, Ordering::SeqCst);
                sqlx::query("INSERT INTO batter_attempt_events VALUES ($1,'commit-must-fail')")
                    .bind(&f.subject)
                    .execute(sql.executor())
                    .await?;
                sqlx::query("INSERT INTO batter_attempt_deferred VALUES (12345)")
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<_, ()>, sqlx::Error>(Authentication::Accepted("provisional"))
            },
        )
        .await;
    match result {
        Err(AttemptError::Atomic(PgAtomicError::Uncertain(
            batter_sqlx::PgAtomicUncertainty::CommitUnconfirmed { output, .. },
        ))) => assert!(matches!(
            output.into_authentication(),
            Authentication::Accepted("provisional")
        )),
        other => panic!("expected explicit commit uncertainty: {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.event_count().await, 0);
    assert!(matches!(
        f.native.admit(f.subject()).await.unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
    f.pool.close().await;
}
