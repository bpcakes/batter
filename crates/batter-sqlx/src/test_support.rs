//! Opt-in native SQLx fixture ownership for an externally configured harness.
//!
//! Prefer [`FixtureSuite::start`]: it retains registered resources outside the
//! assertion-bearing body, joins every native acquisition producer, then cleans up. Cancelling a borrowed
//! [`FixtureRun::wait`] leaves the same driver/result available for another wait.
//! The caller owns server shutdown; harness clones share native admission.
//! Low-level suite methods and [`DatabaseFixture::finish`] remain manual:
//! cancelling those futures or dropping their owners may invoke destructive lease
//! Drop. Abandoned low-level template preparation can leave initializing templates
//! outside deferred drain. Use [`SessionObserver`] for explicit independent session-absence observation.
//! No runtime-death or remote-termination guarantee is implied.
//!
//! ```no_run
//! use batter_sqlx::test_support::{ConnectionPlan, FixtureSuite, FixtureError};
//! use sqlx::postgres::PgPoolOptions;
//! # async fn example(suite: FixtureSuite) -> Result<(), Box<dyn std::error::Error>> {
//! let run = suite.start(|scope| Box::pin(async move {
//!     let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
//!     let fixture = scope.empty(&plan).await?;
//!     sqlx::query("SELECT 1").execute(&fixture.pools()[0]).await.map_err(FixtureError::Observe)?;
//!     Ok::<_, FixtureError>(())
//! }));
//! let report = run.into_report().await?;
//! report.into_result()?;
//! # Ok(()) }
//! ```

mod acquisition;
mod report;
mod runner;
mod sessions;

pub use sessions::{CleanupPhase, DatabaseProgress, SessionObserver};

pub use report::{
    AcquisitionFailure, BodyFailure, DatabaseCleanup, FixtureReport, FixtureReportRef,
};
pub use runner::{FixtureBody, FixtureDatabase, FixtureRun, FixtureScope};

use std::{fmt, future::Future, sync::Arc, time::Duration};

use batter_test_support::{TestFailure, finish};
use postgres_test_harness::{
    DatabaseLease, DatabaseTemplate, FingerprintBuilder, PostgresHarness, TemplateSpec,
};
use sqlx::{PgConnection, PgPool, postgres::PgPoolOptions};

/// One ordered migration input; the consumer must execute the declared content.
pub struct MigrationInput<'a> {
    /// Stable migration identity within its bundle.
    pub identity: &'a str,
    /// Migration kind (for example `up` or `simple`).
    pub kind: &'a str,
    /// Exact SQL bytes; filenames alone are insufficient.
    pub sql: &'a [u8],
}

/// An ordered migration bundle, with stable identity and ordered contents.
pub struct MigrationBundle<'a> {
    /// Stable bundle identity.
    pub identity: &'a str,
    /// Migrations in their execution order.
    pub migrations: &'a [MigrationInput<'a>],
}

/// Frame all ordered inputs using the upstream fingerprint implementation.
///
/// `setup_revision` must change when initialization behavior outside the declared
/// SQL changes. The initializer is trusted to implement these inputs faithfully.
pub fn template_spec(bundles: &[MigrationBundle<'_>], setup_revision: &str) -> TemplateSpec {
    let mut fingerprint =
        FingerprintBuilder::new("batter-sqlx-fixture-v1").add("setup-revision", setup_revision);
    for bundle in bundles {
        fingerprint = fingerprint.add("bundle", bundle.identity);
        for migration in bundle.migrations {
            fingerprint = fingerprint
                .add("migration", migration.identity)
                .add("kind", migration.kind)
                .add("sql", migration.sql);
        }
        fingerprint = fingerprint.add("end-bundle", b"");
    }
    TemplateSpec::new(fingerprint.finish())
}

/// Declared application capacity for one lease, validated before acquisition.
///
/// Reserved connections include standalone blockers, observers and administration
/// even when they target the admin database. Separately bounded suite resources
/// and upstream owner/template/lifecycle sessions are additional server usage.
/// Multiple harnesses do not share a server-wide budget. Pool clones share
/// capacity; every separately constructed pool needs its own declaration.
#[derive(Clone)]
pub struct ConnectionPlan {
    pools: Vec<PgPoolOptions>,
    total: u32,
}

impl ConnectionPlan {
    /// Sum pool maxima and standalone capacity using checked arithmetic.
    pub fn new(pools: Vec<PgPoolOptions>, reserved: u32) -> Result<Self, FixtureError> {
        let total = pools
            .iter()
            .try_fold(reserved, |total, pool| {
                total.checked_add(pool.get_max_connections())
            })
            .ok_or(FixtureError::InvalidBudget)?;
        if pools.is_empty() || pools.iter().any(|pool| pool.get_max_connections() == 0) {
            return Err(FixtureError::InvalidBudget);
        }
        Ok(Self { pools, total })
    }

    /// Reject a declaration larger than the selected per-database capacity.
    pub fn validate(&self, per_database: u32) -> Result<(), FixtureError> {
        if self.total > per_database {
            Err(FixtureError::InvalidBudget)
        } else {
            Ok(())
        }
    }
}

/// Native causes remain inspectable; formatting never emits their contents.
pub enum FixtureError {
    /// An empty, overflowing or over-budget connection declaration.
    InvalidBudget,
    /// A single run requested more leases than can coexist in its harness.
    BatchCapacity,
    /// The template was not retained by this suite.
    UnknownTemplate,
    /// Upstream acquisition, template or cleanup failure.
    Harness(postgres_test_harness::Error),
    /// Owned producer failed; the same native cause is retained in the run report.
    Acquisition(Arc<postgres_test_harness::Error>),
    /// Producer stopped before delivery. Its native join failure is in the report.
    AcquisitionStopped,
    /// Pool acquisition failed with resources still registered for driver cleanup.
    /// The same native cause is retained in the database report, even if handled.
    /// The final report, not this error, establishes their cleanup outcome.
    PoolAcquire(Arc<sqlx::Error>),
    /// Low-level pool acquisition failure after all opened pools closed and lease
    /// cleanup was awaited. `None` means that cleanup succeeded on this path.
    Connect {
        /// Native connection error.
        source: sqlx::Error,
        /// Cleanup also failed after all opened pools were closed.
        cleanup: Option<postgres_test_harness::Error>,
    },
    /// PostgreSQL observation failed.
    Observe(sqlx::Error),
    /// A blocker/waiter relation or database-session absence was not observed
    /// within the bound. A session timeout means absence was not established,
    /// not proof that sessions remain; its lease stays pending until recovery.
    ObservationTimeout,
    /// Observer targets the disposable database or cannot see that database.
    ObservationTarget,
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BatchCapacity => "fixture batch exceeds simultaneous lease capacity",
            Self::InvalidBudget => "invalid fixture connection budget",
            Self::UnknownTemplate => "template does not belong to this fixture suite",
            Self::Harness(_) => "fixture harness failed",
            Self::Acquisition(_) => "fixture acquisition producer failed",
            Self::AcquisitionStopped => "fixture acquisition producer stopped before delivery",
            Self::PoolAcquire(_) => "fixture pool acquisition failed; driver cleanup pending",
            Self::Connect {
                cleanup: Some(_), ..
            } => "fixture pool acquisition and cleanup failed",
            Self::Connect { .. } => "fixture pool acquisition failed",
            Self::Observe(_) => "fixture PostgreSQL observation failed",
            Self::ObservationTarget => "fixture observation target is invalid",
            Self::ObservationTimeout => "fixture PostgreSQL observation timed out",
        })
    }
}
impl fmt::Debug for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for FixtureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Harness(error) => Some(error),
            Self::Acquisition(error) => Some(error.as_ref()),
            Self::PoolAcquire(error) => Some(error.as_ref()),
            Self::Connect { source, .. } | Self::Observe(source) => Some(source),
            _ => None,
        }
    }
}

/// Runtime-local fixture composition and retained upstream template handles.
/// The caller owns the server lifetime, including shutdown. Harness clones share
/// native admission; other owners must release leases for waiting runs to progress.
pub struct FixtureSuite {
    harness: PostgresHarness,
    templates: Vec<DatabaseTemplate>,
    observer: Option<SessionObserver>,
}

impl FixtureSuite {
    /// Adopt a configured, started harness. Environment parsing stays in consumers.
    pub fn new(harness: PostgresHarness) -> Self {
        Self {
            harness,
            templates: Vec::new(),
            observer: None,
        }
    }

    /// Require independently observed session absence before owned-run lease cleanup.
    /// This applies to `start`, including partial acquisition; manual `empty` and
    /// `DatabaseFixture::finish` retain their existing caller-driven contract.
    /// See [`SessionObserver`] for configuration, retry and a complete example.
    pub fn with_session_observer(mut self, observer: SessionObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Retain a template so unchanged inputs reuse upstream initialization.
    ///
    /// The initializer owns and must close its native pools on returned success
    /// and failure. Its capacity is a separately bounded suite resource.
    /// This manual path does not catch initializer panic or retain preparation
    /// after cancellation; upstream can retain an initializing template outside
    /// deferred drain. Use [`FixtureScope::template`] inside [`Self::start`] for
    /// owned preparation and panic-to-abort handling. Neither path joins detached
    /// initializer operations or promises cleanup of initializer-owned live pools.
    pub async fn template<F, Fut>(
        &mut self,
        spec: TemplateSpec,
        initializer: F,
    ) -> Result<DatabaseTemplate, FixtureError>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
    {
        let template = self
            .harness
            .template(spec, initializer)
            .await
            .map_err(FixtureError::Harness)?;
        if !self
            .templates
            .iter()
            .any(|retained| retained.database_name() == template.database_name())
        {
            self.templates.push(template.clone());
        }
        Ok(template)
    }

    /// Low-level acquisition: the caller owns finish and suite drain on every path.
    /// Prefer `start` for assertion-bearing bodies and partial acquisitions.
    /// Checks one connection per pool; SQLx maintains minimum connections in the background.
    pub async fn empty(&self, plan: &ConnectionPlan) -> Result<DatabaseFixture, FixtureError> {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        let lease = self
            .harness
            .empty_database()
            .await
            .map_err(FixtureError::Harness)?;
        DatabaseFixture::connect(lease, plan).await
    }

    /// Low-level clone acquisition, validating capacity and retained template identity.
    /// Prefer `start` when cleanup must survive a body error or panic.
    pub async fn from_template(
        &self,
        template: &DatabaseTemplate,
        plan: &ConnectionPlan,
    ) -> Result<DatabaseFixture, FixtureError> {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        let template = self
            .templates
            .iter()
            .find(|retained| retained.database_name() == template.database_name())
            .ok_or(FixtureError::UnknownTemplate)?;
        let lease = template.database().await.map_err(FixtureError::Harness)?;
        DatabaseFixture::connect(lease, plan).await
    }

    /// Drain submissions accepted before this call. Call after producers finish.
    /// External harness shutdown is separate and does not replace this barrier.
    pub async fn drain(&self) -> Result<(), FixtureError> {
        self.harness
            .drain_deferred_cleanup()
            .await
            .map_err(FixtureError::Harness)
    }
}

/// Own all declared native pools and the disposable database lease together.
/// Release active checkouts before finish. Closed pool clones may remain for inspection.
/// Prefer `FixtureSuite::start` to retain ownership outside the body.
///
/// Ignoring an acquired owner warns because ordinary Drop can queue destructive
/// lease cleanup before asynchronous pool shutdown completes:
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_sqlx::test_support::{ConnectionPlan, FixtureError, FixtureSuite};
/// # async fn example(suite: &FixtureSuite, plan: &ConnectionPlan) -> Result<(), FixtureError> {
/// suite.empty(plan).await?;
/// # Ok(()) }
/// ```
#[must_use = "await DatabaseFixture::finish or use FixtureSuite::start to own cleanup"]
pub struct DatabaseFixture {
    pools: Vec<PgPool>,
    lease: DatabaseLease,
    pool_failures: Vec<Arc<sqlx::Error>>,
}

impl DatabaseFixture {
    async fn connect(lease: DatabaseLease, plan: &ConnectionPlan) -> Result<Self, FixtureError> {
        let mut fixture = Self {
            pools: Vec::new(),
            lease,
            pool_failures: Vec::new(),
        };
        if let Err(source) = fixture.open_pools(plan).await {
            let cleanup = fixture.dispose().await.err();
            return Err(FixtureError::Connect { source, cleanup });
        }
        Ok(fixture)
    }

    async fn open_pools(&mut self, plan: &ConnectionPlan) -> Result<(), sqlx::Error> {
        for options in &plan.pools {
            let pool = options.clone().connect_lazy(self.lease.database_url())?;
            self.pools.push(pool.clone());
            let connection = pool.acquire().await?;
            drop(connection);
        }
        Ok(())
    }

    /// Native pools in connection-plan order. All are closed during finish.
    pub fn pools(&self) -> &[PgPool] {
        &self.pools
    }

    /// Disposable database identity for independent catalog observations.
    pub fn database_name(&self) -> &str {
        self.lease.database_name()
    }

    /// Close all pools before awaited lease cleanup, retaining both returned errors.
    /// Call only after joining operations and releasing checked-out connections.
    /// This future must be driven to completion; cancellation is not shielded.
    pub async fn finish<T, E>(self, body: Result<T, E>) -> Result<T, TestFailure<E, FixtureError>> {
        finish(body, self.cleanup().await)
    }

    async fn cleanup(self) -> Result<(), FixtureError> {
        self.dispose().await.map_err(FixtureError::Harness)
    }

    async fn dispose(self) -> Result<(), postgres_test_harness::Error> {
        futures_util::future::join_all(self.pools.iter().map(PgPool::close)).await;
        self.lease.cleanup().await
    }
}

/// Observe the exact backend lock relation through an independent connection.
///
/// The observer must have declared capacity independent of a one-slot waiter
/// pool. A returned success acknowledges PostgreSQL's observation, not elapsed
/// time. The caller owns blocker acquisition/release and joins the real operation.
pub async fn observe_blocked(
    observer: &mut PgConnection,
    waiter: i32,
    blocker: i32,
    bound: Duration,
) -> Result<(), FixtureError> {
    let observe = async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT $1 = ANY(pg_blocking_pids($2))")
                .bind(blocker)
                .bind(waiter)
                .fetch_one(&mut *observer)
                .await
                .map_err(FixtureError::Observe)?;
            if blocked {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(bound, observe)
        .await
        .map_err(|_| FixtureError::ObservationTimeout)?
}
