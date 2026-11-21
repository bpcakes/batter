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
//! Declare pools for a separately provisioned login with [`ConnectionPlan::declare`]
//! and [`PlannedPool::password_login`]; role lifecycle stays caller-owned.
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
mod endpoint;
mod plan;
mod report;
mod runner;
mod sessions;
mod template;
mod verification;

pub use endpoint::{
    HarnessEndpointReason, UnsupportedHarnessEndpoint, harness_admin_connect_options,
    harness_connect_options,
};
pub use plan::{ConnectionPlan, LeaseDatabaseMismatch, PlanDeclaration, PlannedPool, PoolKey};
pub use sessions::{CleanupPhase, DatabaseProgress, SessionObserver};
pub use template::{FixtureTemplate, TemplateTarget};

pub use report::{
    AcquisitionFailure, BodyFailure, DatabaseCleanup, FixtureReport, FixtureReportRef,
};
pub use runner::{FixtureBody, FixtureDatabase, FixtureRun, FixtureScope};
pub use verification::{
    VerificationLedgerPause, pause_next_verification_ledger_history,
    pause_next_verification_ledger_lock,
};

use std::{fmt, future::Future, sync::Arc, time::Duration};

use batter_test_support::{TestFailure, finish};
use postgres_test_harness::{
    DatabaseLease, DatabaseTemplate, FingerprintBuilder, PostgresHarness, TemplateSpec,
};
use sqlx::{PgConnection, PgPool};
pub(crate) use verification::{
    pause_after_verification_ledger_lock, pause_before_verification_ledger_history,
};

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

/// Native causes remain inspectable; formatting never emits their contents.
pub enum FixtureError {
    /// An empty, overflowing or over-budget connection declaration.
    InvalidBudget,
    /// The harness endpoint was refused before anything was provisioned: no
    /// lease, template lookup, producer or pool exists for this call, so there
    /// is nothing to clean up and the run report records nothing for it.
    /// Returned by every suite and scope acquisition and template entry point;
    /// later refusals (after a lease exists) stay in [`Self::PoolAcquire`],
    /// [`Self::Connect`] or the harness's template initializer failure.
    ///
    /// Migration: 0.0.1 had no such refusal. Its template calls handed the
    /// harness URL straight to the initializer, and its acquisitions parsed the
    /// lease URL with SQLx after provisioning, honouring ambient `PG*`
    /// variables. Calls that succeeded then can now return this variant;
    /// exhaustive `FixtureError` matches need an arm for it, and suites should
    /// unset client `PG*` variables and put every setting in the admin URL.
    Endpoint(UnsupportedHarnessEndpoint),
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
    /// Pool configuration or its connectivity check failed with resources still
    /// registered for driver cleanup. The same SQLx-typed cause is retained in the
    /// database report, even if handled. Besides native connection and URL errors
    /// it can carry the adapter's pre-construction [`LeaseDatabaseMismatch`] inside
    /// `sqlx::Error::Configuration`. SQLx discards a connection whose
    /// `after_connect` fails and retries until `sqlx::Error::PoolTimedOut`; the
    /// hook's own error is never carried here. The final report, not this error,
    /// establishes their cleanup outcome.
    PoolAcquire(Arc<sqlx::Error>),
    /// Low-level pool configuration or connectivity failure after all opened pools
    /// closed and lease cleanup was awaited. `None` means that cleanup succeeded.
    Connect {
        /// SQLx-typed cause, with the same scope as [`Self::PoolAcquire`].
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
            Self::Endpoint(_) => "fixture harness endpoint refused before provisioning",
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
            Self::Endpoint(error) => Some(error),
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
    endpoint: endpoint::SuiteEndpoint,
    templates: Vec<DatabaseTemplate>,
    observer: Option<SessionObserver>,
}

impl FixtureSuite {
    /// Adopt a configured, started harness. Environment parsing stays in consumers.
    ///
    /// Reads the harness admin URL once, with the harness's own parser. An
    /// endpoint fixture clients cannot use is not refused here: every
    /// acquisition and template call returns [`FixtureError::Endpoint`] before
    /// provisioning anything, after checking the process environment again.
    pub fn new(harness: PostgresHarness) -> Self {
        Self {
            endpoint: endpoint::SuiteEndpoint::new(&harness),
            harness,
            templates: Vec::new(),
            observer: None,
        }
    }

    fn endpoint(&self) -> Result<Arc<endpoint::HarnessEndpoint>, FixtureError> {
        self.endpoint.checked().map_err(FixtureError::Endpoint)
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
    /// Both return [`FixtureError::Endpoint`] for an ambient `PG*` variable or
    /// an unusable admin URL before looking up a cached template.
    /// On a cache miss, both pass the initializer a [`TemplateTarget`] built
    /// from the harness's own parse of its admin URL and the template's
    /// recorded name. If the harness URL would select another database or
    /// endpoint, or a client `PG*` variable appeared since the lookup, the
    /// initializer is not called and the harness aborts the template; see
    /// [`UnsupportedHarnessEndpoint`] and [`LeaseDatabaseMismatch`].
    ///
    /// Migration: 0.0.1 took `FnOnce(String)` and returned the harness's
    /// `DatabaseTemplate`; see [`TemplateTarget`] and [`FixtureTemplate`].
    pub async fn template<F, Fut>(
        &mut self,
        spec: TemplateSpec,
        initializer: F,
    ) -> Result<FixtureTemplate, FixtureError>
    where
        F: FnOnce(TemplateTarget) -> Fut,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
    {
        // Refuse before the lookup: the harness can return a cached template
        // without polling the initializer.
        let endpoint = self.endpoint()?;
        let template = self
            .harness
            .template(spec, move |url| {
                template::initialize(endpoint, url, initializer)
            })
            .await
            .map_err(FixtureError::Harness)?;
        Ok(self.retain(template))
    }

    /// Retain a child of a template this suite retains: a copy of `parent`
    /// with one more setup step, cached by the parent's identity plus `spec`.
    ///
    /// The same contract as [`Self::template`], including the endpoint
    /// refusal before lookup and the [`TemplateTarget`] the initializer
    /// receives, here for the writable child copy. `spec` fingerprints only
    /// the local step. A `parent` this suite did not retain is refused with
    /// [`FixtureError::UnknownTemplate`].
    ///
    /// ```no_run
    /// use batter_sqlx::test_support::{FixtureSuite, FixtureTemplate};
    /// use postgres_test_harness::TemplateSpec;
    /// use sqlx::postgres::PgPoolOptions;
    /// # async fn example(mut suite: FixtureSuite, parent: FixtureTemplate, spec: TemplateSpec) -> Result<(), Box<dyn std::error::Error>> {
    /// let child = suite
    ///     .derive(&parent, spec, |target| async move {
    ///         let pool = PgPoolOptions::new()
    ///             .max_connections(1)
    ///             .connect_with(target.connect_options())
    ///             .await?;
    ///         sqlx::query("INSERT INTO items VALUES (1)").execute(&pool).await?;
    ///         pool.close().await;
    ///         Ok(())
    ///     })
    ///     .await?;
    /// # drop(child);
    /// # Ok(()) }
    /// ```
    pub async fn derive<F, Fut>(
        &mut self,
        parent: &FixtureTemplate,
        spec: TemplateSpec,
        initializer: F,
    ) -> Result<FixtureTemplate, FixtureError>
    where
        F: FnOnce(TemplateTarget) -> Fut,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
    {
        let parent = self.retained(parent)?.clone();
        let endpoint = self.endpoint()?;
        let template = parent
            .derive(spec, move |url| {
                template::initialize(endpoint, url, initializer)
            })
            .await
            .map_err(FixtureError::Harness)?;
        Ok(self.retain(template))
    }

    fn retained(&self, template: &FixtureTemplate) -> Result<&DatabaseTemplate, FixtureError> {
        self.templates
            .iter()
            .find(|retained| retained.database_name() == template.database_name())
            .ok_or(FixtureError::UnknownTemplate)
    }

    fn retain(&mut self, template: DatabaseTemplate) -> FixtureTemplate {
        if !self
            .templates
            .iter()
            .any(|retained| retained.database_name() == template.database_name())
        {
            self.templates.push(template.clone());
        }
        FixtureTemplate { inner: template }
    }

    /// Low-level acquisition: the caller owns finish and suite drain on every path.
    /// Prefer `start` for assertion-bearing bodies and partial acquisitions.
    /// Constructs every declared pool before checking one connection per pool;
    /// SQLx maintains minimum connections in the background.
    pub async fn empty(&self, plan: &ConnectionPlan) -> Result<DatabaseFixture, FixtureError> {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        let endpoint = self.endpoint()?;
        let lease = self
            .harness
            .empty_database()
            .await
            .map_err(FixtureError::Harness)?;
        DatabaseFixture::connect(&endpoint, lease, plan).await
    }

    /// Low-level clone acquisition, validating capacity and retained template identity.
    /// Prefer `start` when cleanup must survive a body error or panic.
    pub async fn from_template(
        &self,
        template: &FixtureTemplate,
        plan: &ConnectionPlan,
    ) -> Result<DatabaseFixture, FixtureError> {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        let template = self.retained(template)?;
        let endpoint = self.endpoint()?;
        let lease = template.database().await.map_err(FixtureError::Harness)?;
        DatabaseFixture::connect(&endpoint, lease, plan).await
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
    plan: u64,
}

impl DatabaseFixture {
    async fn connect(
        endpoint: &endpoint::HarnessEndpoint,
        lease: DatabaseLease,
        plan: &ConnectionPlan,
    ) -> Result<Self, FixtureError> {
        let mut fixture = Self {
            pools: Vec::new(),
            lease,
            pool_failures: Vec::new(),
            plan: plan.id(),
        };
        if let Err(source) = fixture.open_pools(endpoint, plan).await {
            let cleanup = fixture.dispose().await.err();
            return Err(FixtureError::Connect { source, cleanup });
        }
        Ok(fixture)
    }

    async fn open_pools(
        &mut self,
        endpoint: &endpoint::HarnessEndpoint,
        plan: &ConnectionPlan,
    ) -> Result<(), sqlx::Error> {
        let resolved = plan.resolve(
            endpoint,
            self.lease.database_url(),
            self.lease.database_name(),
            endpoint::Environment::process(),
        )?;
        // Every pool is owned here before the first await; dispose closes them all.
        let pools = plan::construct(resolved, &mut self.pools);
        plan::check(&pools).await
    }

    /// Native pools in connection-plan order. All are closed during finish.
    /// For a `ConnectionPlan::declare` plan, positional access does not
    /// distinguish logins: select a pool with [`Self::pool`] and use this slice
    /// to inspect every pool.
    pub fn pools(&self) -> &[PgPool] {
        &self.pools
    }

    /// The pool a [`PoolKey`] declared; `None` for a key from another plan.
    /// See [`ConnectionPlan::declare`] for issuing keys.
    pub fn pool(&self, key: PoolKey) -> Option<&PgPool> {
        plan::lookup(&self.pools, self.plan, key)
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
