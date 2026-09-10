use std::{future::Future, pin::Pin, sync::Arc};

use batter::telemetry::with_current_dispatch;
use postgres_test_harness::{DatabaseLease, DatabaseTemplate, PostgresHarness, TemplateSpec};
use sqlx::PgPool;
use tokio::task::JoinHandle;

use super::acquisition::{Registered, Resources, join_producers, produce};
use super::report::{BodyFailure, DatabaseCleanup, FixtureReport, FixtureReportRef};
use super::{ConnectionPlan, DatabaseFixture, FixtureError, FixtureSuite};

/// A body borrowing its allocation scope; the scope cannot escape into a task.
pub type FixtureBody<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Native access handles. The runner, not this handle, owns database cleanup.
/// Closed pool clones may be retained for inspection; active checkouts must be released.
pub struct FixtureDatabase {
    pools: Vec<PgPool>,
    name: String,
}

impl FixtureDatabase {
    /// Native pools in declaration order.
    pub fn pools(&self) -> &[PgPool] {
        &self.pools
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
/// before delivery; pools register before initialization. Body exit cannot skip
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
    resources: Resources,
    capacity: Arc<tokio::sync::Semaphore>,
}

impl FixtureScope {
    /// Retain a stable upstream template. Initializer policy remains caller-owned.
    /// Initializers must explicitly close any pools they create before returning.
    /// Preparation is an owned producer, so its initializer must be Send and static.
    /// Cancelling this waiter does not cancel preparation or discard its failure.
    pub async fn template<F, Fut>(
        &mut self,
        spec: TemplateSpec,
        initializer: F,
    ) -> Result<DatabaseTemplate, FixtureError>
    where
        F: FnOnce(String) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
    {
        let harness = self.harness.clone();
        let resources = self.resources.clone();
        produce(&self.resources, async move {
            let template = harness
                .template(spec, move |url| async move {
                    // Native template Drop can retain an initializing database. Turn
                    // initializer panic into its returned-error path so upstream
                    // awaits abort and preserves any independent abort failure.
                    tokio::spawn(with_current_dispatch(async move { initializer(url).await }))
                        .await
                        .map_err(|error| Box::new(error) as postgres_test_harness::BoxError)?
                })
                .await?;
            let mut registered = resources.lock().unwrap_or_else(|error| error.into_inner());
            if !registered
                .templates
                .iter()
                .any(|retained| retained.database_name() == template.database_name())
            {
                registered.templates.push(template.clone());
            }
            Ok(template)
        })
        .await
    }

    /// Register an empty database and each lazy pool before its first acquisition.
    /// This checks connectivity; SQLx owns background minimum-connection maintenance.
    pub async fn empty(&self, plan: &ConnectionPlan) -> Result<FixtureDatabase, FixtureError> {
        let harness = self.harness.clone();
        self.acquire(plan, async move { harness.empty_database().await })
            .await
    }

    /// Clone only a template retained by this scope's suite, with checked capacity.
    pub async fn from_template(
        &self,
        template: &DatabaseTemplate,
        plan: &ConnectionPlan,
    ) -> Result<FixtureDatabase, FixtureError> {
        plan.validate(self.harness.connection_limits().connections_per_database())?;
        let template = self
            .resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .templates
            .iter()
            .find(|retained| retained.database_name() == template.database_name())
            .ok_or(FixtureError::UnknownTemplate)?
            .clone();
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
        let capacity = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| FixtureError::BatchCapacity)?;
        let resources = self.resources.clone();
        let index = produce(&self.resources, async move {
            let lease = work.await?;
            let mut registered = resources.lock().unwrap_or_else(|error| error.into_inner());
            let index = registered.fixtures.len();
            registered.fixtures.push(DatabaseFixture {
                pools: Vec::new(),
                lease,
            });
            capacity.forget(); // Held until the driver cleans this registered lease.
            Ok(index)
        })
        .await?;
        self.open(index, plan).await
    }

    async fn open(
        &self,
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
        let mut pools = Vec::new();
        for options in &plan.pools {
            let pool = options
                .clone()
                .connect_lazy(&url)
                .map_err(FixtureError::PoolAcquire)?;
            self.resources
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .fixtures[index]
                .pools
                .push(pool.clone());
            // Unlike connect().await, the pool is retained before this explicit
            // acquisition polls after_connect. SQLx may also maintain minimum
            // connections in its own background task.
            let connection = pool.acquire().await.map_err(FixtureError::PoolAcquire)?;
            drop(connection);
            pools.push(pool);
        }
        Ok(FixtureDatabase { pools, name })
    }
}

/// A waiter for independently running body/cleanup tasks in the current runtime.
///
/// Cancelling `wait` does not abort the driver; another wait observes the same
/// result. Dropping this handle detaches the driver and loses its report. Runtime
/// destruction or internal driver failure has no cleanup-completion guarantee.
#[must_use = "await the fixture report to observe body and cleanup failures"]
pub struct FixtureRun<T, E> {
    driver: Option<JoinHandle<FixtureReport<T, E>>>,
    result: Option<Result<FixtureReport<T, E>, tokio::task::JoinError>>,
}

impl<T, E> FixtureRun<T, E> {
    /// Observe the same driver repeatedly, including after a cancelled wait.
    pub async fn wait(&mut self) -> Result<FixtureReportRef<'_, T, E>, &tokio::task::JoinError> {
        if self.result.is_none() {
            let result = self
                .driver
                .as_mut()
                .expect("unobserved driver exists")
                .await;
            self.result = Some(result);
            self.driver = None;
        }
        self.result
            .as_ref()
            .expect("driver result was recorded")
            .as_ref()
            .map(FixtureReportRef)
    }

    /// Consume the waiter and return the actual report. Cancelling this convenience
    /// future loses the waiter; use `wait` when observation must be resumable.
    pub async fn into_report(mut self) -> Result<FixtureReport<T, E>, tokio::task::JoinError> {
        let _ = self.wait().await;
        self.result.take().expect("driver result was recorded")
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
        let harness = self.harness.clone();
        let capacity = Arc::new(tokio::sync::Semaphore::new(
            harness.connection_limits().max_simultaneous_leases(),
        ));
        let mut scope = FixtureScope {
            capacity,
            harness: self.harness,
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
            let databases =
                futures_util::future::join_all(fixtures.into_iter().map(|fixture| async move {
                    let database_name = fixture.database_name().to_owned();
                    let result = fixture.cleanup().await;
                    DatabaseCleanup {
                        database_name,
                        result,
                    }
                }))
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
            driver: Some(driver),
            result: None,
        }
    }
}
