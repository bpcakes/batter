//! Live tests of the shared attempt lease state against PostgreSQL.
//!
//! The memory and PostgreSQL attempt backends answer one scripted lifecycle
//! identically: the same admissions, denials, completions, delays and
//! observations, in the same order. A stored row that fits no lease state is
//! refused before commit instead of being reinterpreted, and a lease expiry
//! that a denial reported stays expired when the server clock steps backwards.
//!
//! Memory time is an injected clock. PostgreSQL time is the server clock, so
//! the script advances it by moving every stored instant of the subject's row
//! back by the same amount. Real time that passes while the test runs only
//! moves the server clock forward, so a deadline reached exactly in memory is
//! also reached in PostgreSQL, and remaining delays are compared rounded up to
//! the script's one-minute step.
use runlimit_core::{
    KeyHasher, PolicyId, QuotaPeriod, ScopeId,
    attempts::{
        AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptObservation,
        AttemptObserver, AttemptOutcome, AttemptPolicy,
    },
};
use runlimit_memory::{Clock, attempts::MemoryAttemptLimiter};
use runlimit_postgres::attempts::PostgresAttemptLimiter;
use sqlx::{AssertSqlSafe, PgPool, postgres::PgPoolOptions};
use std::{
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MINUTE: u64 = 60_000;

#[derive(Clone, Copy)]
enum Step {
    /// Moves both clocks forward by this many minutes.
    Advance(u64),
    /// Admits; an admitted receipt takes the next receipt index.
    Admit,
    /// Completes the receipt with this index.
    Complete(usize, AttemptOutcome),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Seen {
    Admitted,
    /// Whole minutes until the live lease expires, rounded up.
    Busy(u64),
    /// Whole minutes of remaining backoff, rounded up.
    Backoff(u64),
    StorageCapacity,
    /// Outcome, consecutive failures and the exact delay in minutes.
    Applied(AttemptOutcome, u32, u64),
    Stale,
    CommitUncertain,
}

fn minutes_up(delay: runlimit_core::Delay) -> u64 {
    u64::try_from(delay.duration().as_millis())
        .unwrap()
        .div_ceil(MINUTE)
}
fn denied(denial: AttemptDenial) -> Seen {
    match denial {
        AttemptDenial::Busy { retry_after } => Seen::Busy(minutes_up(retry_after)),
        AttemptDenial::Backoff { retry_after } => Seen::Backoff(minutes_up(retry_after)),
        AttemptDenial::StorageCapacity => Seen::StorageCapacity,
    }
}
fn completed(result: AttemptCompletionResult) -> Seen {
    match result {
        AttemptCompletionResult::Applied(done) => {
            let delay = u64::try_from(done.retry_after().duration().as_millis()).unwrap();
            assert_eq!(delay % MINUTE, 0, "completion delays are exact");
            Seen::Applied(done.outcome(), done.consecutive_failures(), delay / MINUTE)
        }
        AttemptCompletionResult::Stale => Seen::Stale,
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Seen>>);
impl AttemptObserver for Recorder {
    fn observe(&self, observation: AttemptObservation) {
        self.0.lock().unwrap().push(match observation {
            AttemptObservation::Admitted => Seen::Admitted,
            AttemptObservation::Denied(denial) => denied(denial),
            AttemptObservation::Completed(done) => {
                completed(AttemptCompletionResult::Applied(done))
            }
            AttemptObservation::Stale => Seen::Stale,
            AttemptObservation::CommitUncertain => Seen::CommitUncertain,
        });
    }
}

#[derive(Clone, Default)]
struct ScriptClock(Arc<AtomicU64>);
impl Clock for ScriptClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn policy() -> AttemptPolicy {
    let minutes = |value: u64| QuotaPeriod::new(Duration::from_millis(value * MINUTE)).unwrap();
    AttemptPolicy::new(
        PolicyId::new("auth").unwrap(),
        ScopeId::new("identifier").unwrap(),
        minutes(10),
        minutes(40),
        minutes(100),
        minutes(20),
    )
    .unwrap()
}

/// Initial delay 10, cap 40, quiet period 100 and lease 20, in minutes.
fn script() -> Vec<(Step, Option<Seen>)> {
    use AttemptOutcome::{Abandoned, Failure, Success};
    use Step::{Admit, Advance, Complete};
    vec![
        // One live lease; a second admission waits for its deadline.
        (Admit, Some(Seen::Admitted)),
        (Admit, Some(Seen::Busy(20))),
        (Advance(5), None),
        (Admit, Some(Seen::Busy(15))),
        // Failures double the delay up to the cap; abandonment is a failure.
        (Complete(0, Failure), Some(Seen::Applied(Failure, 1, 10))),
        (Admit, Some(Seen::Backoff(10))),
        (Advance(5), None),
        (Admit, Some(Seen::Backoff(5))),
        (Advance(5), None),
        (Admit, Some(Seen::Admitted)),
        (
            Complete(1, Abandoned),
            Some(Seen::Applied(Abandoned, 2, 20)),
        ),
        (Advance(20), None),
        (Admit, Some(Seen::Admitted)),
        (Complete(2, Failure), Some(Seen::Applied(Failure, 3, 40))),
        (Advance(40), None),
        (Admit, Some(Seen::Admitted)),
        (Complete(3, Failure), Some(Seen::Applied(Failure, 4, 40))),
        (Advance(40), None),
        // A lease that reaches its deadline uncompleted is the fifth failure,
        // dated at the deadline, and its late success is stale.
        (Admit, Some(Seen::Admitted)),
        (Advance(20), None),
        (Admit, Some(Seen::Backoff(40))),
        (Complete(4, Success), Some(Seen::Stale)),
        (Admit, Some(Seen::Backoff(40))),
        (Advance(40), None),
        // Success clears the failures, so the next failure is the first again.
        (Admit, Some(Seen::Admitted)),
        (Complete(5, Success), Some(Seen::Applied(Success, 0, 0))),
        (Admit, Some(Seen::Admitted)),
        (Complete(6, Failure), Some(Seen::Applied(Failure, 1, 10))),
        // A full quiet period after the last failure also clears them.
        (Advance(100), None),
        (Admit, Some(Seen::Admitted)),
        (Complete(7, Failure), Some(Seen::Applied(Failure, 1, 10))),
        (Advance(10), None),
        // One minute short of the quiet period, an expired lease is still the
        // second failure, and the reservation that follows carries it forward.
        (Admit, Some(Seen::Admitted)),
        (Advance(119), None),
        (Admit, Some(Seen::Admitted)),
        (Complete(8, Success), Some(Seen::Stale)),
        (Complete(9, Failure), Some(Seen::Applied(Failure, 3, 40))),
        (Advance(40), None),
        // The expiry counts first and the quiet period then runs from the lease
        // deadline: 20 minutes of lease plus 100 quiet minutes clear the subject.
        (Admit, Some(Seen::Admitted)),
        (Advance(120), None),
        (Admit, Some(Seen::Admitted)),
        (Complete(10, Failure), Some(Seen::Stale)),
        (Complete(11, Failure), Some(Seen::Applied(Failure, 1, 10))),
    ]
}

fn run_memory(policy: &AttemptPolicy, recorder: Arc<Recorder>) -> Vec<Seen> {
    let clock = ScriptClock::default();
    let limiter =
        MemoryAttemptLimiter::with_clock(NonZeroUsize::MIN, clock.clone()).with_observer(recorder);
    let subject = KeyHasher::new([7; 32])
        .unwrap()
        .hash_attempt_for(policy, b"user");
    let mut receipts = Vec::new();
    let mut seen = Vec::new();
    for (step, _) in script() {
        match step {
            Step::Advance(minutes) => {
                clock.0.fetch_add(minutes * MINUTE, Ordering::SeqCst);
            }
            Step::Admit => seen.push(match limiter.admit(subject).unwrap() {
                AttemptAdmission::Admitted(receipt) => {
                    receipts.push(Some(receipt));
                    Seen::Admitted
                }
                AttemptAdmission::Denied(denial) => denied(denial),
            }),
            Step::Complete(index, outcome) => {
                let receipt = receipts[index].take().expect("receipt completed once");
                seen.push(completed(limiter.complete(receipt, outcome).unwrap()));
            }
        }
    }
    seen
}

/// One disposable schema holding the attempt table.
struct Database {
    pool: PgPool,
    admin: PgPool,
    schema: String,
}
impl Database {
    async fn new() -> Self {
        let url = std::env::var("RUNLIMIT_POSTGRES_TEST_DATABASE_URL")
            .expect("disposable PostgreSQL URL required");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let schema = format!(
            "runlimit_attempt_state_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .unwrap();
        let search_path = format!("SET search_path={schema},pg_catalog");
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .after_connect(move |connection, _| {
                let query = search_path.clone();
                Box::pin(async move {
                    sqlx::query(AssertSqlSafe(query))
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        PostgresAttemptLimiter::new(pool.clone())
            .migrate()
            .await
            .unwrap();
        Self {
            pool,
            admin,
            schema,
        }
    }
    /// Moves the server clock by `minutes` relative to every stored instant:
    /// forward when positive, backwards when negative.
    async fn advance(&self, minutes: i64) {
        sqlx::query(
            "UPDATE runlimit_attempts SET last_failure_ms=last_failure_ms-$1,\
             retry_at_ms=retry_at_ms-$1,lease_until_ms=lease_until_ms-$1",
        )
        .bind(minutes * i64::try_from(MINUTE).unwrap())
        .execute(&self.pool)
        .await
        .unwrap();
    }
    async fn close(self) {
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .unwrap();
        self.admin.close().await;
    }
}

async fn run_postgres(policy: &AttemptPolicy, recorder: Arc<Recorder>) -> Vec<Seen> {
    let db = Database::new().await;
    let limiter = PostgresAttemptLimiter::new(db.pool.clone()).with_observer(recorder);
    let subject = KeyHasher::new([7; 32])
        .unwrap()
        .hash_attempt_for(policy, b"user");
    let mut receipts = Vec::new();
    let mut seen = Vec::new();
    for (step, _) in script() {
        match step {
            Step::Advance(minutes) => db.advance(i64::try_from(minutes).unwrap()).await,
            Step::Admit => seen.push(match limiter.admit(subject).await.unwrap() {
                AttemptAdmission::Admitted(receipt) => {
                    receipts.push(Some(receipt));
                    Seen::Admitted
                }
                AttemptAdmission::Denied(denial) => denied(denial),
            }),
            Step::Complete(index, outcome) => {
                let receipt = receipts[index].take().expect("receipt completed once");
                seen.push(completed(limiter.complete(receipt, outcome).await.unwrap()));
            }
        }
    }
    db.close().await;
    seen
}

fn expected() -> Vec<Seen> {
    script().into_iter().filter_map(|(_, seen)| seen).collect()
}

/// The script is not vacuous without a database: memory alone must match it.
#[test]
fn memory_follows_the_scripted_attempt_lifecycle() {
    let recorder = Arc::new(Recorder::default());
    assert_eq!(run_memory(&policy(), recorder.clone()), expected());
    assert_eq!(*recorder.0.lock().unwrap(), expected());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn memory_and_postgres_share_attempt_transition_semantics() {
    let policy = policy();
    let memory_observed = Arc::new(Recorder::default());
    let postgres_observed = Arc::new(Recorder::default());
    let memory = run_memory(&policy, memory_observed.clone());
    let postgres = run_postgres(&policy, postgres_observed.clone()).await;
    assert_eq!(memory, expected());
    assert_eq!(postgres, memory);
    assert_eq!(
        *postgres_observed.0.lock().unwrap(),
        *memory_observed.0.lock().unwrap()
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_delay_without_failures_is_a_storage_invariant_until_the_row_is_corrected() {
    let db = Database::new().await;
    let limiter = PostgresAttemptLimiter::new(db.pool.clone());
    let policy = policy();
    let subject = KeyHasher::new([7; 32])
        .unwrap()
        .hash_attempt_for(&policy, b"user");
    let AttemptAdmission::Admitted(receipt) = limiter.admit(subject).await.unwrap() else {
        panic!("first admission was denied");
    };
    assert_eq!(
        completed(
            limiter
                .complete(receipt, AttemptOutcome::Success)
                .await
                .unwrap()
        ),
        Seen::Applied(AttemptOutcome::Success, 0, 0)
    );
    let rewrite = |assignment: &'static str| {
        sqlx::query(AssertSqlSafe(format!(
            "UPDATE runlimit_attempts SET {assignment}"
        )))
        .execute(&db.pool)
    };

    // No lease and no failures is `Clear`, which has no delay to honor.
    rewrite("retry_at_ms=last_failure_ms+600000").await.unwrap();
    let error = limiter.admit(subject).await.unwrap_err();
    assert_eq!(
        error.consumption(),
        runlimit_core::ConsumptionStatus::NotConsumed
    );
    let runlimit_postgres::CheckError::StorageInvariant(error) = error else {
        panic!("expected storage invariant")
    };
    assert_eq!(
        error.detail(),
        "attempt without failures carries a retry delay"
    );
    assert!(std::error::Error::source(&error).is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM runlimit_attempts WHERE lease_until_ms IS NULL AND failures=0"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        1,
        "the refused admission reserved nothing and rewrote nothing"
    );

    // The same delay beside one failure is ordinary backoff.
    rewrite("failures=1").await.unwrap();
    assert_eq!(
        match limiter.admit(subject).await.unwrap() {
            AttemptAdmission::Denied(denial) => denied(denial),
            AttemptAdmission::Admitted(_) => panic!("held subject was admitted"),
        },
        Seen::Backoff(10)
    );
    // Moving the retry time into the past releases it and keeps the failure.
    rewrite("retry_at_ms=0").await.unwrap();
    let AttemptAdmission::Admitted(receipt) = limiter.admit(subject).await.unwrap() else {
        panic!("released subject was denied");
    };
    assert_eq!(
        completed(
            limiter
                .complete(receipt, AttemptOutcome::Failure)
                .await
                .unwrap()
        ),
        Seen::Applied(AttemptOutcome::Failure, 2, 20)
    );
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn an_expiry_reported_by_a_denial_survives_a_backwards_clock_step() {
    let db = Database::new().await;
    let limiter = PostgresAttemptLimiter::new(db.pool.clone());
    let policy = policy();
    let subject = KeyHasher::new([7; 32])
        .unwrap()
        .hash_attempt_for(&policy, b"user");
    let admit = async || match limiter.admit(subject).await.unwrap() {
        AttemptAdmission::Admitted(receipt) => Ok(receipt),
        AttemptAdmission::Denied(denial) => Err(denied(denial)),
    };
    let stored = async || {
        sqlx::query_as::<_, (i64, bool)>(
            "SELECT failures, lease_token IS NULL AND lease_until_ms IS NULL FROM runlimit_attempts",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };

    let receipt = admit().await.expect("first admission was denied");
    // A live lease denies without changing the row.
    assert_eq!(admit().await.err(), Some(Seen::Busy(20)));
    assert_eq!(stored().await, (0, false));
    // At its deadline the lease has failed, and the denial that reports the
    // backoff stores that failure and releases the token.
    db.advance(20).await;
    assert_eq!(admit().await.err(), Some(Seen::Backoff(10)));
    assert_eq!(stored().await, (1, true));

    // The clock steps back behind the deadline. The receipt stays stale and
    // cannot erase the failure the denial reported.
    db.advance(-1).await;
    assert_eq!(
        completed(
            limiter
                .complete(receipt, AttemptOutcome::Success)
                .await
                .unwrap()
        ),
        Seen::Stale
    );
    assert_eq!(stored().await, (1, true));
    assert_eq!(admit().await.err(), Some(Seen::Backoff(11)));
    db.close().await;
}
