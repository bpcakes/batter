//! The script vocabulary, the observation recorder, the scripted memory clock
//! and the disposable schema shared by the attempt state tests.
use runlimit_core::{
    PolicyId, QuotaPeriod, ScopeId,
    attempts::{
        AttemptCompletionResult, AttemptDenial, AttemptObservation, AttemptObserver,
        AttemptOutcome, AttemptPolicy,
    },
};
use runlimit_memory::Clock;
use runlimit_postgres::attempts::PostgresAttemptLimiter;
use sqlx::{AssertSqlSafe, PgPool, postgres::PgPoolOptions};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const MINUTE: u64 = 60_000;

#[derive(Clone, Copy)]
pub enum Step {
    /// Moves both clocks forward by this many minutes.
    Advance(u64),
    /// Admits; an admitted receipt takes the next receipt index.
    Admit,
    /// Completes the receipt with this index.
    Complete(usize, AttemptOutcome),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Seen {
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

pub fn minutes_up(delay: runlimit_core::Delay) -> u64 {
    u64::try_from(delay.duration().as_millis())
        .unwrap()
        .div_ceil(MINUTE)
}
pub fn denied(denial: AttemptDenial) -> Seen {
    match denial {
        AttemptDenial::Busy { retry_after } => Seen::Busy(minutes_up(retry_after)),
        AttemptDenial::Backoff { retry_after } => Seen::Backoff(minutes_up(retry_after)),
        AttemptDenial::StorageCapacity => Seen::StorageCapacity,
    }
}
pub fn completed(result: AttemptCompletionResult) -> Seen {
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
pub struct Recorder(pub Mutex<Vec<Seen>>);
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
pub struct ScriptClock(pub Arc<AtomicU64>);
impl Clock for ScriptClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

pub fn policy() -> AttemptPolicy {
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

/// Distinguishes schemas created in one process within one clock tick.
static NEXT: AtomicU64 = AtomicU64::new(0);
/// One disposable schema holding the attempt table.
pub struct Database {
    pub pool: PgPool,
    admin: PgPool,
    schema: String,
}
impl Database {
    pub async fn new() -> Self {
        let url = std::env::var("RUNLIMIT_POSTGRES_TEST_DATABASE_URL")
            .expect("disposable PostgreSQL URL required");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let schema = format!(
            "runlimit_attempt_state_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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
    pub async fn advance(&self, minutes: i64) {
        sqlx::query(
            "UPDATE runlimit_attempts SET last_failure_ms=last_failure_ms-$1,\
             retry_at_ms=retry_at_ms-$1,lease_until_ms=lease_until_ms-$1",
        )
        .bind(minutes * i64::try_from(MINUTE).unwrap())
        .execute(&self.pool)
        .await
        .unwrap();
    }
    pub async fn close(self) {
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
