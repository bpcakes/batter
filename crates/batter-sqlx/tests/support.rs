use batter::BoxError;
use sqlx::{Connection, PgConnection, PgPool, postgres::PgPoolOptions};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, timeout};

pub type Result<T = ()> = std::result::Result<T, BoxError>;
pub const LOCAL: Duration = Duration::from_secs(2);
pub const OBSERVE: Duration = Duration::from_secs(5);

pub fn require(condition: bool, message: &'static str) -> Result {
    if condition {
        Ok(())
    } else {
        Err(std::io::Error::other(message).into())
    }
}

pub async fn bounded<T>(future: impl Future<Output = T>) -> Result<T> {
    let started = Instant::now();
    let value = timeout(LOCAL, future)
        .await
        .map_err(|_| std::io::Error::other("local operation exceeded two-second budget"))?;
    // Tokio polls the inner future before checking its timeout. A late ready
    // result must not establish completion within the declared local budget.
    require(
        started.elapsed() <= LOCAL,
        "local completion exceeded two-second budget",
    )?;
    Ok(value)
}

// No provisioning: two native control connections and one one-slot pool in an
// externally supplied disposable database. Session locks require no schema DDL.
pub struct Fixture {
    pub pool: PgPool,
    pub observer: PgConnection,
    blocker: PgConnection,
    pub key: i64,
    pub retired: Vec<i32>,
}

impl Fixture {
    pub async fn new() -> Result<Self> {
        let url = std::env::var("DATABASE_URL")
            .map_err(|_| std::io::Error::other("live checks require DATABASE_URL"))?;
        let mut blocker = bounded(PgConnection::connect(&url)).await??;
        let observer = bounded(PgConnection::connect(&url)).await??;
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(LOCAL)
            .connect_lazy(&url)?;
        let pid: i32 =
            bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut blocker))
                .await??;
        let key = (i64::from(pid) << 32) | 0x42545452;
        bounded(
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(key)
                .execute(&mut blocker),
        )
        .await??;
        Ok(Self {
            pool,
            observer,
            blocker,
            key,
            retired: Vec::new(),
        })
    }

    pub async fn blocked(&mut self, pid: i32) -> Result {
        let deadline = Instant::now() + OBSERVE;
        loop {
            let blocked: bool = bounded(sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')"
            ).bind(pid).fetch_one(&mut self.observer)).await??;
            if blocked {
                return Ok(());
            }
            require(
                Instant::now() < deadline,
                "backend did not reach server lock",
            )?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn replacement_and_close(&mut self) -> Result {
        let pid: i32 =
            bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&self.pool)).await??;
        require(!self.retired.contains(&pid), "retired backend was reused")?;
        bounded(self.pool.close()).await?;
        require(
            self.pool.size() == 0 && self.pool.is_closed(),
            "pool close did not release local accounting",
        )?;
        for pid in self.retired.clone() {
            self.blocked(pid).await?;
        }
        eprintln!(
            "local_close_before_unlock retired_backend_count={}",
            self.retired.len()
        );
        Ok(())
    }

    pub async fn finish(mut self, body: Result) -> Result {
        // Drive each cleanup even if a preceding one failed; retain both causes.
        let unlock =
            bounded(sqlx::query("SELECT pg_advisory_unlock_all()").execute(&mut self.blocker))
                .await
                .and_then(|result| result.map(|_| ()).map_err(Into::into));
        let blocker_close = bounded(self.blocker.close())
            .await
            .and_then(|result| result.map_err(Into::into));
        // Closing the blocker also releases its locks if the unlock query failed.
        // The ordinary-return control still owns pool-accounted sessions; close
        // those after unlock before waiting for their independent disappearance.
        let pool_close = bounded(self.pool.close()).await;
        let deadline = Instant::now() + OBSERVE;
        let disappeared = async {
            loop {
                let count: i64 = bounded(
                    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid = ANY($1)")
                        .bind(&self.retired)
                        .fetch_one(&mut self.observer),
                )
                .await??;
                if count == 0 {
                    eprintln!("retired_sessions_disappeared count={}", self.retired.len());
                    break Ok(());
                }
                require(
                    Instant::now() < deadline,
                    "retired server sessions remained after unlock",
                )?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        .await;
        let observer_close = bounded(self.observer.close())
            .await
            .and_then(|result| result.map_err(Into::into));
        let cleanup = combine(
            combine(
                combine(combine(unlock, blocker_close), disappeared),
                pool_close,
            ),
            observer_close,
        );
        combine(body, cleanup)
    }
}

pub fn combine(body: Result, cleanup: Result) -> Result {
    batter_test_support::finish(body.map_err(Cause), cleanup.map_err(Cause))
        .map_err(|error| Box::new(error) as BoxError)
}

struct Cause(BoxError);

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Test-owned assertion messages are safe; native causes remain internal.
        if let Some(error) = self.0.downcast_ref::<std::io::Error>() {
            write!(f, "{error}")
        } else {
            f.write_str("live test cause retained")
        }
    }
}
impl std::fmt::Debug for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for Cause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}
