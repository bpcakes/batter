use std::{future::Future, pin::Pin, sync::Arc};

use batter_core::telemetry::with_current_dispatch;
use postgres_test_harness::{
    BoxError, DatabaseLease, DatabaseTemplate, PostgresHarness, TemplateSpec,
};
use sqlx::PgPool;
use tokio::task::JoinHandle;

use super::acquisition::{Registered, Resources, join_producers, produce};
use super::endpoint::{HarnessEndpoint, SuiteEndpoint};
use super::plan::{PoolKey, check, construct, lookup};
use super::report::{BodyFailure, FixtureReport, FixtureReportRef};
use super::template::{FixtureTemplate, TemplateTarget, find_retained, initialize, retain};
use super::{ConnectionPlan, DatabaseFixture, FixtureError, FixtureSuite};

/// Run a template initializer in its own task. Native template Drop can retain
/// an initializing database, so an initializer panic becomes its returned-error
/// path and upstream awaits abort and preserves any independent abort failure.
/// The target check runs inside the same task: a refusal then drops the
/// initializer there too, so a panic in its destructor still reaches the
/// awaited abort instead of unwinding past it.
async fn in_task<F, Fut>(
    endpoint: Arc<HarnessEndpoint>,
    url: String,
    initializer: F,
) -> Result<(), BoxError>
where
    F: FnOnce(TemplateTarget) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
{
    tokio::spawn(with_current_dispatch(initialize(
        endpoint,
        url,
        initializer,
    )))
    .await
    .map_err(|error| Box::new(error) as BoxError)?
}

#[cfg(test)]
mod tests;

/// A body borrowing its allocation scope; the scope cannot escape into a task.
pub type FixtureBody<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Native access handles. The runner, not this handle, owns database cleanup.
/// Closed pool clones may be retained for inspection; active checkouts must be released.
pub struct FixtureDatabase {
    pools: Vec<PgPool>,
    name: String,
    plan: u64,
}

impl FixtureDatabase {
    /// Native pools in declaration order, for inspection such as close checks.
    /// Indexes are unambiguous only for `ConnectionPlan::new`, where every pool
    /// uses the lease login; for a `ConnectionPlan::declare` plan select pools
    /// with [`Self::pool`], since an index can reach the owner pool by mistake.
    pub fn pools(&self) -> &[PgPool] {
        &self.pools
    }
    /// The pool a [`PoolKey`] declared; `None` for a key from another plan.
    /// See `ConnectionPlan::declare` for issuing keys.
    pub fn pool(&self, key: PoolKey) -> Option<&PgPool> {
        lookup(&self.pools, self.plan, key)
    }
    /// Database identity for independent catalog observations.
    pub fn database_name(&self) -> &str {
        &self.name
    }
}

/// Borrowed allocation scope for one owned fixture run.
///
/// Native acquisition producers register with the independent driver before
/// their delivery waiter can be cancelled. Producers publish leases/templates
/// before delivery; every declared pool registers, in declaration order and with
/// no await in between, before the first connectivity check. An eager
/// `min_connections` pool's background `after_connect` may already run during
/// construction. Body exit cannot skip
/// joining those producers and cleaning their leases. A run retains leases until body exit and rejects batches
/// exceeding the harness simultaneous-lease limit rather than waiting on its own
/// held permits. Shared harness owners must release their native capacity for
/// waiting acquisitions to progress; observation cancellation does not stop them.
/// Use separate runs for successive batches. Application operations must still be joined before return;
/// detached server sessions require separate positive quiescence observation.
///
/// The allocation scope cannot be moved into a detached task:
/// ```compile_fail,E0521
/// use batter_sqlx::test_support::{ConnectionPlan, FixtureError, FixtureSuite};
/// use sqlx::postgres::PgPoolOptions;
/// # fn example(suite: FixtureSuite) {
/// let _run = suite.start(|scope| Box::pin(async move {
///     let _task = tokio::spawn(async move {
///         let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
///         scope.empty(&plan).await
///     });
///     Ok::<_, FixtureError>(())
/// }));
/// # }
/// ```
pub struct FixtureScope {
    harness: PostgresHarness,
    endpoint: SuiteEndpoint,
    resources: Resources,
    capacity: Arc<tokio::sync::Semaphore>,
}

impl FixtureScope {
    /// Retain a stable upstream template. Initializer policy remains caller-owned.
    /// Initializers must explicitly close any pools they create before returning.
    /// Preparation is an owned producer, so its initializer must be Send and static.
    /// Cancelling this waiter does not cancel preparation or discard its failure.
    /// The initializer receives the same [`TemplateTarget`] as
    /// [`super::FixtureSuite::template`] describes, and the same
    /// [`FixtureError::Endpoint`] refusal precedes the lookup; a refused call
    /// starts no producer, so the report records nothing for it.
    ///
    /// Migration: 0.0.1 took `FnOnce(String)` and returned the harness's
    /// `DatabaseTemplate`; see [`TemplateTarget`] and [`FixtureTemplate`].
    pub async fn template<F, Fut>(
        &mut self,
        spec: TemplateSpec,
        initializer: F,
    ) -> Result<FixtureTemplate, FixtureError>
    where
        F: FnOnce(TemplateTarget) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        // Refuse before the lookup: the harness can return a cached template
        // without polling the initializer.
        let endpoint = self.endpoint.checked()?;
        let harness = self.harness.clone();
        self.retain(async move {
            harness
                .template(spec, move |url| in_task(endpoint, url, initializer))
                .await
        })
        .await
    }

    /// Retain a child of a template this scope's suite retains, as
    /// [`super::FixtureSuite::derive`] describes, with the owned preparation
    /// and panic handling of [`Self::template`]. A `parent` the suite did not
    /// retain is refused with [`FixtureError::UnknownTemplate`] before any
    /// producer starts.
    ///
    /// ```no_run
    /// use batter_sqlx::test_support::{ConnectionPlan, FixtureError, FixtureSuite, template_spec};
    /// use sqlx::postgres::PgPoolOptions;
    /// # async fn example(suite: FixtureSuite) -> Result<(), Box<dyn std::error::Error>> {
    /// let run = suite.start(|scope| Box::pin(async move {
    ///     let schema = scope
    ///         .template(template_spec(&[], "schema-v1"), |target| async move {
    ///             let pool = PgPoolOptions::new().connect_with(target.connect_options()).await?;
    ///             sqlx::query("CREATE TABLE items (id bigint)").execute(&pool).await?;
    ///             pool.close().await;
    ///             Ok(())
    ///         })
    ///         .await?;
    ///     let seeded = scope
    ///         .derive(&schema, template_spec(&[], "seed-v1"), |target| async move {
    ///             let pool = PgPoolOptions::new().connect_with(target.connect_options()).await?;
    ///             sqlx::query("INSERT INTO items VALUES (1)").execute(&pool).await?;
    ///             pool.close().await;
    ///             Ok(())
    ///         })
    ///         .await?;
    ///     let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    ///     let _fixture = scope.from_template(&seeded, &plan).await?;
    ///     Ok::<_, FixtureError>(())
    /// }));
    /// run.into_report().await?.into_result()?;
    /// # Ok(()) }
    /// ```
    pub async fn derive<F, Fut>(
        &mut self,
        parent: &FixtureTemplate,
        spec: TemplateSpec,
        initializer: F,
    ) -> Result<FixtureTemplate, FixtureError>
    where
        F: FnOnce(TemplateTarget) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        // The endpoint refusal comes first on every template entry point.
        let endpoint = self.endpoint.checked()?;
        let parent = self.retained(parent)?;
        self.retain(async move {
            parent
                .derive(spec, move |url| in_task(endpoint, url, initializer))
                .await
        })
        .await
    }

    fn retained(&self, template: &FixtureTemplate) -> Result<DatabaseTemplate, FixtureError> {
        let registered = self
            .resources
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        find_retained(&registered.templates, template).cloned()
    }

    /// Run a template lookup as an owned producer and retain its result.
    async fn retain<W>(&self, lookup: W) -> Result<FixtureTemplate, FixtureError>
    where
        W: Future<Output = Result<DatabaseTemplate, postgres_test_harness::Error>> + Send + 'static,
    {
        let resources = self.resources.clone();
        produce(&self.resources, async move {
            let template = lookup.await?;
            let mut registered = resources.lock().unwrap_or_else(|error| error.into_inner());
            Ok(retain(&mut registered.templates, template))
        })
        .await
    }

    /// Register an empty database, then every declared lazy pool, before the first
    /// connectivity check. Cancelling during the checks leaves the database and
    /// all its pools registered. SQLx owns background minimum-connection
    /// maintenance; an eager `min_connections` pool may connect as soon as it exists.
    pub async fn empty(&self, plan: &ConnectionPlan) -> Result<FixtureDatabase, FixtureError> {
        let harness = self.harness.clone();
        self.acquire(plan, async move { harness.empty_database().await })
            .await
    }

    /// Clone only a template retained by this scope's suite, with checked capacity.
    pub async fn from_template(
        &self,
        template: &FixtureTemplate,
        plan: &ConnectionPlan,
    ) -> Result<FixtureDatabase, FixtureError> {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        // Report an endpoint refusal before an unknown template, as `empty`
        // and the template entry points do; `acquire` checks it again.
        self.endpoint.checked()?;
        let template = self.retained(template)?;
        self.acquire(plan, async move { template.database().await })
            .await
    }

    async fn acquire<F>(
        &self,
        plan: &ConnectionPlan,
        work: F,
    ) -> Result<FixtureDatabase, FixtureError>
    where
        F: Future<Output = Result<DatabaseLease, postgres_test_harness::Error>> + Send + 'static,
    {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        let endpoint = self.endpoint.checked()?;
        let capacity = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| FixtureError::BatchCapacity)?;
        let resources = self.resources.clone();
        let plan_id = plan.id();
        let index = produce(&self.resources, async move {
            let lease = work.await?;
            let mut registered = resources.lock().unwrap_or_else(|error| error.into_inner());
            let index = registered.fixtures.len();
            registered.fixtures.push(DatabaseFixture {
                pools: Vec::new(),
                lease,
                pool_failures: Vec::new(),
                plan: plan_id,
            });
            capacity.forget(); // Held until the driver cleans this registered lease.
            Ok(index)
        })
        .await?;
        self.open(&endpoint, index, plan).await
    }

    fn pool_failed(&self, index: usize, error: sqlx::Error) -> FixtureError {
        let error = Arc::new(error);
        self.resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .fixtures[index]
            .pool_failures
            .push(error.clone());
        FixtureError::PoolAcquire(error)
    }

    async fn open(
        &self,
        endpoint: &HarnessEndpoint,
        index: usize,
        plan: &ConnectionPlan,
    ) -> Result<FixtureDatabase, FixtureError> {
        let (name, url) = {
            let resources = self
                .resources
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let lease = &resources.fixtures[index].lease;
            (
                lease.database_name().to_owned(),
                lease.database_url().to_owned(),
            )
        };
        // Resolution runs no caller code. A refusal registers its cause with no
        // pool constructed; the lease is already registered for cleanup.
        let resolved = plan
            .resolve(
                endpoint,
                &url,
                &name,
                super::endpoint::Environment::process(),
            )
            .map_err(|error| self.pool_failed(index, error))?;
        // No await separates delivery from this registration: a waiter cancelled
        // later leaves every declared pool registered. Unlike connect().await,
        // each pool is retained before this run's checks poll after_connect.
        let pools = construct(
            resolved,
            &mut self
                .resources
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .fixtures[index]
                .pools,
        );
        check(&pools)
            .await
            .map_err(|error| self.pool_failed(index, error))?;
        Ok(FixtureDatabase {
            pools,
            name,
            plan: plan.id(),
        })
    }
}

/// A waiter for independently running body/cleanup tasks in the current runtime.
///
/// Cancelling `wait` does not abort the driver; another wait observes the same
/// result. Dropping this handle detaches the driver and loses its report. Runtime
/// destruction or internal driver failure has no cleanup-completion guarantee.
#[must_use = "await the fixture report to observe body and cleanup failures"]
pub struct FixtureRun<T, E> {
    state: RunState<T, E>,
    progress: super::sessions::Progress,
}

enum RunState<T, E> {
    Running(JoinHandle<FixtureReport<T, E>>),
    Completed(Result<FixtureReport<T, E>, tokio::task::JoinError>),
}

impl<T, E> FixtureRun<T, E> {
    /// Observe for at most `bound`; `Ok(None)` means pending, never clean reuse.
    /// Only this observation expires; the body and cleanup driver remain owned.
    /// Cancelling this future likewise leaves `wait`/`wait_for` resumable. A native
    /// driver JoinError remains distinct from timeout and SQLx close's unit result.
    /// See [`super::SessionObserver`] for an example.
    pub async fn wait_for(
        &mut self,
        bound: std::time::Duration,
    ) -> Result<Option<FixtureReportRef<'_, T, E>>, &tokio::task::JoinError> {
        match tokio::time::timeout(bound, self.wait()).await {
            Ok(result) => result.map(Some),
            Err(_) => Ok(None),
        }
    }

    /// Snapshot database phases and all observation errors recorded so far.
    /// Empty progress can mean the body or acquisition producers are still active.
    /// Completed entries still require inspection of the final report and drain.
    pub fn cleanup_progress(&self) -> Vec<super::DatabaseProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Observe the same driver repeatedly, including after a cancelled wait.
    pub async fn wait(&mut self) -> Result<FixtureReportRef<'_, T, E>, &tokio::task::JoinError> {
        loop {
            match self.state {
                RunState::Running(ref mut driver) => {
                    self.state = RunState::Completed(driver.await);
                }
                RunState::Completed(ref result) => {
                    return result.as_ref().map(FixtureReportRef);
                }
            }
        }
    }

    /// Consume the waiter and return the actual report. Cancelling this convenience
    /// future loses the waiter; use `wait` when observation must be resumable.
    pub async fn into_report(self) -> Result<FixtureReport<T, E>, tokio::task::JoinError> {
        match self.state {
            RunState::Running(driver) => driver.await,
            RunState::Completed(result) => result,
        }
    }
}

impl FixtureSuite {
    /// Start an owned body and cleanup driver before returning its waiter.
    ///
    /// The body borrows its allocation scope. Every registered database is cleaned
    /// after body completion/panic, even when an acquisition waiter is abandoned.
    /// The driver joins native lease/template producers before cleaning databases
    /// concurrently and draining deferred cleanup. Producer failures remain in the
    /// report even if the body handled the delivered error. Server shutdown remains
    /// caller-owned; clones may still have active leases or runs.
    /// Neither native operation descendants nor retired sessions are implicitly joined.
    ///
    /// ```no_run
    /// use batter_sqlx::test_support::{ConnectionPlan, FixtureSuite};
    /// use sqlx::postgres::PgPoolOptions;
    /// # async fn example(suite: FixtureSuite) {
    /// let run = suite.start(|scope| Box::pin(async move {
    ///     let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    ///     let db = scope.empty(&plan).await?;
    ///     Ok::<_, batter_sqlx::test_support::FixtureError>(db.database_name().to_owned())
    /// }));
    /// let report = run.into_report().await.expect("fixture driver failed");
    /// assert!(report.is_ok());
    /// assert!(report.acquisitions.iter().all(Result::is_ok));
    /// # }
    /// ```
    pub fn start<T, E, F>(self, body: F) -> FixtureRun<T, E>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: for<'a> FnOnce(&'a mut FixtureScope) -> FixtureBody<'a, T, E> + Send + 'static,
    {
        let resources = Registered::new(self.templates);
        let observer = self.observer;
        let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
        let driver_progress = progress.clone();
        let harness = self.harness.clone();
        let capacity = Arc::new(tokio::sync::Semaphore::new(
            harness.connection_limits().max_simultaneous_leases(),
        ));
        let mut scope = FixtureScope {
            capacity,
            harness: self.harness,
            endpoint: self.endpoint,
            resources: resources.clone(),
        };
        let body = tokio::spawn(with_current_dispatch(async move { body(&mut scope).await }));
        let driver = tokio::spawn(with_current_dispatch(async move {
            let body = match body.await {
                Ok(result) => result.map_err(BodyFailure::Returned),
                Err(error) => Err(BodyFailure::Task(error)),
            };
            let acquisitions = join_producers(&resources).await;
            let fixtures = std::mem::take(
                &mut resources
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .fixtures,
            );
            *driver_progress
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = fixtures
                .iter()
                .map(|fixture| super::DatabaseProgress {
                    database_name: fixture.database_name().to_owned(),
                    phase: super::CleanupPhase::ClosingPools,
                    observation_failures: Vec::new(),
                })
                .collect();
            let databases = futures_util::future::join_all(fixtures.into_iter().enumerate().map(
                |(index, fixture)| {
                    super::sessions::cleanup(
                        fixture,
                        observer.clone(),
                        driver_progress.clone(),
                        index,
                    )
                },
            ))
            .await;
            let drain = harness
                .drain_deferred_cleanup()
                .await
                .map_err(FixtureError::Harness);
            FixtureReport {
                body,
                acquisitions,
                databases,
                drain,
            }
        }));
        FixtureRun {
            state: RunState::Running(driver),
            progress,
        }
    }
}
