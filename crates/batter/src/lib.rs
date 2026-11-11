//! Batter's public facade.
//!
//! The operational implementation lives in [`batter_core`]. This crate keeps
//! the established `batter::...` paths source-compatible and exposes optional
//! adapter namespaces without changing native ownership.
//!
//! The default feature set is empty. Enable only the namespaces an application
//! uses: `at-rest`, `axum`, `metrics`, `otlp`, `outbound-http`, `sqlx`, `runledger`, `runlimit`,
//! `flags`, `test-support`, or the narrower bridge features `runlimit-memory`,
//! `runlimit-postgres`, `runlimit-axum`, `runlimit-native-http`, `runlimit-native-axum`,
//! `runledger-test-support`, `sqlx-test-support`, `flags-postgres`, `flags-axum`,
//! `flags-runledger` and `flags-test-support`.
//!
//! # Important limits
//!
//! Deadlines drop futures; they do not undo external effects. Cancellation is
//! cooperative, not preemption. Supervision owns directly registered tasks,
//! not tasks secretly spawned by a component. Owned startup, command and service
//! drivers retain registered cleanup independently of borrowed waiters. A
//! command's signal listeners live only while its work can be cancelled; Tokio
//! never restores the replaced disposition. Direct `CleanupStack::close` driving
//! remains caller-owned. See `docs/guarantees.md`.
//!
//! # Choosing between supported paths
//!
//! Each row is a policy choice between supported APIs, not a gap in the
//! protected path. Namespaces behind a feature are named with that feature.
//!
//! | Need | Use | Not |
//! | --- | --- | --- |
//! | Panic output from any executable root | `std::panic::set_hook(batter::redacting_panic_hook())`, unwrapped, as the first statement of every binary's `main`, before argument parsing, configuration, tracing and mode dispatch; inside a `#[tokio::main]` body the runtime is built first, so install it in a synchronous `main` before building the runtime when runtime-construction panics matter. The hook writes one location line per panic, never the payload or a backtrace; its rustdoc lists the panics it cannot cover | the default hook, or a chain that calls it; a wrapper or later hook, the application's or a dependency's, that formats, logs or sends `PanicHookInfo` or its payload, or that can itself panic (`eprintln!`, `thread::current`, a `tracing` event), which aborts the process; installing it late, in only some binaries or modes, or from library code, tests or a helper that tests call |
//! | A long-running service | [`startup::Startup::protected`] with an async-closure initializer, `with_unix_signals`, `start`, `finish_with_output` for a typed result such as the bound address, then [`lifecycle::RunningSupervisor::wait_checked`]; [`startup::ApplicationError`] when the initializer combines unrelated errors with `?`; with metrics export, [`service::start`] and `otlp::prepare` (feature `otlp`), unit output only | driving [`lifecycle::Supervisor::start`] yourself, a channel to carry startup values out, an adapter error type, or awaiting the raw report without the checked form |
//! | Finite work: setup, migration, an operator command | [`command::Command::new`] or [`command::Command::within`]; at an executable root [`command::Command::start_with_unix_signals`], then [`command::CommandOwner::finish`] and [`command::CommandSuccess::into_value`]; in reusable helpers and in-process tests [`command::Command::start_without_signals`]; [`command::Command::start`] with [`command::check_command`] only when observers must share one report | `Startup`, which reports `EmptySupervisor` for a process with no critical component; hand-installed `tokio::signal` listeners around a command; a command started inside another command's work, whose cleanup an interrupted outer command leaves unjoined; `expect` after `check_command`, or `Clone`, to copy a value out of a shared report |
//! | Work whose finalization needs its typed outcome | [`operation::PhasedOperation::new`] with a parent context and positive reserve, then `start()`; retain the running owner and await `running.wait().await`, whose `Ok` proves both phases succeeded and whose phase-failure report retains both outcomes; parent cancellation still interrupts finalization | manual `work()` / `finalization()` sequencing that can return on work error before finalization |
//! | Job work/finalization borrowing native progress or checkpoint services | `runledger::PhasedJob::new` with a `runledger::PhasedJobHandler`; the native worker owns both callbacks | manual `job_phases` sequencing or an independently spawned coordinator capturing the invocation |
//! | A supported native resource with registered cleanup | obtain a slot with `reserve_cleanup` in owned startup/command work, then use [`cleanup::CleanupSlot::acquire_semaphore`] or [`cleanup::CleanupSlot::bind_udp`]; use adapter-owned acquisition such as `sqlx::pool_in` for a PostgreSQL pool | acquiring first and calling `push` / `register` by hand, or [`cleanup::low_level::acquire_pair`], which leaves acquisition-to-pair construction with the caller; application hooks and unsupported resource protocols retain explicit manual ownership |
//! | A PostgreSQL pool the service owns | `sqlx::pool_in` after `reserve_cleanup` with the pool's `sqlx::PgSessionProfile`, returning the `sqlx::PgProfiledPool` (feature `sqlx`); `runledger::RunledgerDatabase` when Runledger runs on the same pool (feature `runledger`) | a pool built outside startup with a manual close |
//! | Routines, triggers and constraint triggers a release owns | one `sqlx::managed::ManagedObjectManifest` built from in-place source files: `sqlx::PgLease::migrate_and_apply` at setup, then the same manifest in `sqlx::verification::VerificationPlan::with_managed_objects` at readiness (feature `sqlx`) | whole routine bodies restated in new migrations, a constraint trigger created in a migration (it binds the previously installed body), or objects applied by hand before or after `PgLease::migrate` |
//! | A transaction write | `sqlx::run_atomic_in`, or `sqlx::run_atomic_with_in` for an exhaustive failure policy; use their `run_atomic_profiled_in` / `run_atomic_profiled_with_in` counterparts with a profiled pool. For a durable job in the same transaction, wrap `runledger::run_atomic` in the caller's `context.run` | `sqlx::low_level::PgAtomicTransaction`, which leaves commit confirmation to the caller |
//! | Quota admission | `runlimit::http::HttpQuota` when Batter orders deadline, authentication and quota before the body (feature `runlimit-axum`); `runlimit::Quota::run` inside your own work; `runlimit::native_transport` when the application owns subject derivation and rejection mapping (features `runlimit-native-http`, `runlimit-native-axum`) | the native Tower layer under `HttpBoundary`, which receives no deadline or ordering from it |
//! | Async work under a concurrency bound | [`admission::Bulkhead::run`] owns admission, execution and permit release | `Bulkhead::enter` was removed because it left permit lifetime and context checks with the caller |
//! | Synchronous CPU work or blocking I/O under a concurrency bound | [`admission::Bulkhead::run_blocking`] with the caller's context, matching its exhaustive [`admission::BlockingError`] in separate arms | the removed `Bulkhead::enter` with its permit moved into `tokio::task::spawn_blocking` by hand, a permit held by the awaiting future, or unbounded `spawn_blocking` |
//! | PostgreSQL quota storage a service serves from | `runlimit::maintenance::register_in` with the native limiter, then serve the returned limiter through `runlimit::Quota` or `runlimit::http::HttpQuota`; its login needs the store's admission and expiry-cleanup `runlimit::grants` selections (feature `runlimit-postgres`) | a `runlimit::postgres` limiter served directly, whose expired rows nothing deletes |
//! | A provider HTTP call | `outbound_http::Client::new` with a validated `outbound_http::Origin` and `outbound_http::ConnectTimeout`, built once and cloned; `client.request(method, path_segments)` with its headers and body, then `prepare()?`, which reports every construction error before anything is sent; `PreparedRequest::send_in` with the work phase of `reserve_finalization` after the application's dispatch fence is written, so the outcome can be recorded in the finalization phase unless the parent is cancelled; then an exhaustive `outbound_http::Exchange` match with no wildcard arm, where only `NotStarted` and `NotDispatched` (`proves_unsent`) show that this attempt wrote nothing and what a `Responded` status means is the application's reading of its provider (feature `outbound-http`) | a hand-built `reqwest` client, whose default redirects and replays make a connect error no proof that nothing was sent; retrying `Unknown` as though nothing was sent; reading `Responded` as processed, accepted or refused by the provider, when a load balancer or proxy may have answered; wrapping `send_in` in `run`, `retry`, `timeout` or `select!`, whose interruption drops the exchange and proves nothing |
//! | A job handler's budget | `runledger::PhasedJob::new(handler, reserve)?` derives both budgets from the native invocation, whose exit cancels both phases. `runledger::job_phases` has been removed; phase sequencing belongs to the handler adapter | a fresh root sized from the remaining budget, which never sees the invocation's exit; returning with `?` from manual work before finalization |
//! | A job handler test without a worker | Pass a `runledger::PhasedJob` directly to `runledger::test_support::HandlerInvocation::new(budget)?.execute(&handler, payload)`; use `HandlerInvocation::for_context(budget, context)?` for a handler that reads its job's organization, attempt or lease. Native `JobExecutionHandler` values need `into_job_handler()` first. Use a `[dev-dependencies]` graph (feature `runledger-test-support`), then match the exhaustive `HandlerExit` | custom `JobExecutionServices` owning a `JobInvocationOwner` the test must end itself |
//!
//! Every executable root starts the same way, whatever it goes on to run:
//!
//! ```no_run
//! #[tokio::main]
//! async fn main() {
//!     std::panic::set_hook(batter::redacting_panic_hook());
//!     // Parse arguments, load configuration and initialize tracing after it.
//! }
//! ```
//!
//! An interruption selected before the Runledger runner returns local commit
//! acknowledgement proves neither rollback nor permission to replay. When the
//! work branch completes, `context.run` returns its result without rechecking
//! cancellation or the clock; dropping the outer future loses its result.
//!
//! An owned service root awaits checked completion after startup handoff. The
//! success witness retains the report; `?` propagates the original failed
//! report or coordinator error through the application's error boundary.
//!
//! ```no_run
//! #![deny(unused_must_use)]
//! use batter::{BoxError, lifecycle::RunningSupervisor};
//!
//! async fn finish(running: &RunningSupervisor) -> Result<(), BoxError> {
//!     running.wait_checked().await?;
//!     Ok(())
//! }
//! ```
//!
//! ```
//! use batter::operation::OperationContext;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let context = batter::operation::OperationOwner::new(Duration::from_secs(2))?.into_context();
//! let value = context.run("example.read", |_scope| async {
//!     Ok::<_, std::io::Error>(42)
//! }).await?;
//! assert_eq!(value, 42);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub use batter_core::*;

/// Synchronous envelope encryption and stable MAC keys.
///
/// Enabled by the `at-rest` feature. The implementation remains the standalone
/// [`batter_at_rest`] package, and every item has the same type identity through
/// the direct and facade paths.
///
/// ```
/// let _: Option<batter::at_rest::Keyring> = None;
/// ```
#[cfg(feature = "at-rest")]
pub mod at_rest {
    pub use batter_at_rest::*;
}

/// Send-once outbound provider HTTP exchange with exhaustive transport evidence.
///
/// Enabled by the `outbound-http` feature, which also selects the package's
/// `operation` bridge. The implementation remains the standalone
/// [`batter_outbound_http`] package, whose documentation holds the full contract,
/// and every item has the same type identity through the direct and facade
/// paths. Send with `PreparedRequest::send_in` under the work phase of a
/// context whose finalization is reserved; only `NotStarted` and
/// `NotDispatched` prove that an attempt wrote nothing. The facade's
/// `outbound_exchange` example is the consumer contract.
///
/// ```no_run
/// use batter::outbound_http::{Client, ConnectTimeout, Origin, ResponseLimit, http::Method};
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = Client::new(
///     Origin::https("https://provider.example")?,
///     ConnectTimeout::new(Duration::from_secs(5))?,
/// )?;
/// let request = client.request(Method::POST, ["v1", "effects"]).prepare()?;
/// let context = batter::operation::OperationOwner::new(Duration::from_secs(10))?.into_context();
/// let phases = context.reserve_finalization(Duration::from_millis(500))?;
/// let exchange = request.send_in(phases.work(), ResponseLimit::new(64 * 1024)?).await;
/// // Settle or hold the application's dispatch fence from `exchange` in
/// // `phases.finalization()`; a cancelled parent runs no recording step.
/// # let _ = exchange;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "outbound-http")]
pub mod outbound_http {
    pub use batter_outbound_http::*;
}

/// Axum request, browser, readiness, and serving boundaries.
///
/// Enabled by the `axum` feature. Native Axum and Tower types remain native;
/// this module re-exports Batter's adapter surface only.
///
/// ```
/// let _: Option<batter::axum::RequestPolicy> = None;
/// ```
///
/// `HttpBoundary` is the canonical composition and its assembly is sealed: it
/// is consumed into protected serving or into an opaque request client, with
/// one `batter` dependency and no direct adapter declaration.
///
/// ```
/// use axum::{body::Body, extract::Request, http::StatusCode, routing::get};
/// use batter::{
///     axum::{GuardedRouter, HttpBoundary, RequestPolicy, ResponseConstructionBudget},
///     lifecycle::ShutdownHandle,
/// };
/// use std::time::Duration;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let budget = ResponseConstructionBudget::new(Duration::from_secs(1))?;
/// let client = HttpBoundary::new(RequestPolicy::new(control.operation_admission(), budget))
///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
///     .await?
///     .in_process();
/// let request = Request::builder().uri("/work").body(Body::empty())?;
/// assert_eq!(client.request(request).await.status(), StatusCode::OK);
/// # Ok(()) }
/// ```
///
/// The deliberately caller-ordered middleware and registration helpers are
/// reachable only through the adapter's own `low_level` namespace, with no
/// aliases at this module's root:
///
/// ```
/// let _: fn(
///     &mut batter::lifecycle::Supervisor,
///     &'static str,
///     tokio::net::TcpListener,
///     ::axum::Router,
/// ) -> Result<(), batter::RegistrationError> = batter::axum::low_level::register_http;
/// ```
///
/// ```compile_fail,E0432
/// use batter::axum::register_http;
/// ```
#[cfg(feature = "axum")]
pub mod axum {
    pub use batter_axum::*;
}

/// Native SQLx PostgreSQL transaction, snapshot, verification, and connection
/// disposition APIs.
///
/// Enabled by the `sqlx` feature. The nested `test_support` module additionally
/// requires `sqlx-test-support` and remains owned by the SQLx adapter.
///
/// ```
/// let _: Option<batter::sqlx::PgLease> = None;
/// ```
#[cfg(feature = "sqlx")]
pub mod sqlx {
    pub use batter_sqlx::*;

    /// External-harness fixture ownership for SQLx tests.
    #[cfg(feature = "sqlx-test-support")]
    pub mod test_support {
        pub use batter_sqlx::test_support::*;
    }
}

/// Runledger initialization and native settlement translation.
///
/// Enabled by the `runledger` feature, which also exposes [`crate::sqlx`] for
/// owned PostgreSQL scopes and [`crate::runledger::run_atomic`]. Durable policy and native
/// supervision remain owned by Runledger.
///
/// `runledger::native::{core, postgres, runtime}` are the native packages
/// themselves, so one `batter` dependency reaches worker preparation, the job
/// catalog, durable intents and the migrators without a second declaration of a
/// native package. `runledger-test-support` additionally exposes
/// `runledger::native::test_support` for a consumer's database tests and
/// `runledger::test_support` for driving one
/// handler without a worker. They are
/// low-level: `register_in` remains the protected composition, and reaching a
/// native namespace does not move ownership of durable policy, storage or
/// supervision into the facade.
///
/// Register [`runledger::PhasedJob`] when work requires finalization. Its
/// callbacks borrow native services, and the adapter owns sequencing, budgets
/// and complete failure classification. Handlers join a native `JobCatalog` or
/// `JobRegistry`; `register_in` registers the prepared worker. Test the same
/// adapted handler through `runledger::test_support::HandlerInvocation`.
/// Migration: `job_phases` was removed; use the protected handler adapter.
///
/// ```
/// use batter::runledger::native::core::jobs::{
///     JobCompletion, JobExecutionError, JobFailure, JobType,
/// };
/// use batter::runledger::native::core::prelude::async_trait;
/// use batter::runledger::native::runtime::catalog::JobCatalog;
/// use batter::runledger::{
///     JobFinalization, JobWork, PhasedJob, PhasedJobFailure, PhasedJobHandler,
/// };
/// use serde_json::Value;
/// use std::time::Duration;
///
/// struct Count;
/// #[async_trait]
/// impl PhasedJobHandler for Count {
///     type Output = u64;
///     type Error = JobExecutionError;
///     fn job_type(&self) -> JobType<'static> { JobType::new("example.count") }
///     async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<u64, Self::Error> {
///         scope.execution().save_checkpoint(&1_u64).await?;
///         Ok(2)
///     }
///     async fn finalize(&self, scope: JobFinalization<'_, u64, Self::Error>)
///         -> Result<JobCompletion, Self::Error> {
///         if let Ok(count) = scope.work().classify() {
///             scope.execution().save_checkpoint(count).await?;
///         }
///         Ok(JobCompletion::success())
///     }
///     fn failed(&self, _: PhasedJobFailure<u64, Self::Error>) -> JobFailure {
///         // This example chooses terminal policy for every failed invocation.
///         JobFailure::terminal("example.count_failed", "Count phases failed.")
///     }
/// }
/// let catalog = JobCatalog::new()
///     .handler(PhasedJob::new(Count, Duration::from_millis(300))?);
/// # let _ = catalog;
/// # Ok::<(), batter::ConfigurationError>(())
/// ```
///
/// ```
/// let _: Option<batter::runledger::NativeReport> = None;
/// let _: Option<batter::runledger::JobPhasesRejection> = None;
/// let _: fn(batter::sqlx::PgScopeError<()>) -> batter::runledger::PgScopeError<()> = |e| e;
/// let _: Option<batter::sqlx::PgSession<'_>> = None;
/// let _: Option<batter::runledger::native::core::jobs::JobType<'static>> = None;
/// let _: Option<batter::runledger::native::postgres::SchemaCompatibilityError> = None;
/// let _: Option<batter::runledger::native::runtime::config::JobsConfig> = None;
/// ```
#[cfg(feature = "runledger")]
pub mod runledger {
    pub use batter_runledger::*;

    /// Drive one job handler in a test without a worker, queue or database.
    ///
    /// Enabled by `runledger-test-support` and owned by the Runledger adapter.
    /// `HandlerInvocation::new(budget)` declares the native deadline for a fresh
    /// job and `HandlerInvocation::for_context(budget, context)` for the claimed
    /// job a handler reads; `with_checkpoint` and `with_progress` declare what a
    /// retried row retains. `execute` builds the real `JobExecution` with an
    /// owned exit signal, dispatches the handler as the worker does, ends the
    /// invocation after destroying the handler future, and returns an exhaustive
    /// `HandlerExit` with the persisted progress and contained exit-hook panics.
    /// Durable outcome policy stays in the worker. Select the feature only in
    /// `[dev-dependencies]`; see `native::test_support` for the resolver rule.
    ///
    /// ```
    /// use batter::runledger::native::core::jobs::{
    ///     JobCompletion, JobExecutionError, JobFailure, JobType,
    /// };
    /// use batter::runledger::native::core::prelude::async_trait;
    /// use batter::runledger::test_support::{HandlerExit, HandlerInvocation};
    /// use batter::runledger::{
    ///     JobFinalization, JobWork, PhasedJob, PhasedJobFailure, PhasedJobHandler,
    /// };
    /// use serde_json::Value;
    /// use std::time::Duration;
    ///
    /// struct Count;
    /// #[async_trait]
    /// impl PhasedJobHandler for Count {
    ///     type Output = u64;
    ///     type Error = JobExecutionError;
    ///     fn job_type(&self) -> JobType<'static> { JobType::new("example.count") }
    ///     async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<u64, Self::Error> {
    ///         scope.execution().save_checkpoint(&1_u64).await?;
    ///         Ok(2)
    ///     }
    ///     async fn finalize(&self, scope: JobFinalization<'_, u64, Self::Error>)
    ///         -> Result<JobCompletion, Self::Error> {
    ///         if let Ok(count) = scope.work().classify() {
    ///             scope.execution().save_checkpoint(count).await?;
    ///         }
    ///         Ok(JobCompletion::success())
    ///     }
    ///     fn failed(&self, _: PhasedJobFailure<u64, Self::Error>) -> JobFailure {
    ///         // This example chooses terminal policy for every failed invocation.
    ///         JobFailure::terminal("example.count_failed", "Count phases failed.")
    ///     }
    /// }
    /// # tokio::runtime::Builder::new_current_thread().enable_time().build()?.block_on(async {
    /// let handler = PhasedJob::new(Count, Duration::from_millis(300))?;
    /// let run = HandlerInvocation::new(Duration::from_secs(5))?
    ///     .execute(&handler, serde_json::json!({})).await;
    /// assert!(matches!(run.exit, HandlerExit::Returned(Ok(_))));
    /// assert_eq!(run.persisted.checkpoint, Some(serde_json::json!(2)));
    /// assert!(run.exit_hook_panics.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(()) })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg(feature = "runledger-test-support")]
    pub mod test_support {
        pub use batter_runledger::test_support::*;
    }
}

/// Runlimit quota admission and its selected HTTP/error bridges.
///
/// Enabled by `runlimit` or one of its bridge features. The `http` module is
/// present only with `runlimit-axum`; the bare `runlimit` feature selects no
/// storage backend, so `runlimit::memory` requires `runlimit-memory` and
/// `runlimit::postgres` requires `runlimit-postgres`.
///
/// `runlimit::native`, `runlimit::memory`, `runlimit::postgres` and
/// `runlimit::native_transport::{http, axum}` are the native packages
/// themselves, so one `batter` dependency reaches validated policies, key
/// derivation, both backends with their migrations, and both native transport
/// contracts. `native_transport` needs `runlimit-native-http` or
/// `runlimit-native-axum`; neither selects `axum`, the Batter Axum adapter or
/// `runlimit::http`. It is deliberately named apart from `runlimit::http`: that
/// module is the protected quota-before-body assembly, while the native layer
/// leaves subject derivation, rejection mapping, status codes and header
/// stability to the caller.
///
/// With `runlimit-postgres`, `runlimit::maintenance::register_in` is the
/// supervised construction of a PostgreSQL limiter: it registers the store's
/// expired-row cleanup as a periodic component before returning the limiter to
/// serve with. A `runlimit::postgres` limiter used directly is the unmaintained
/// low-level form.
///
/// ```
/// let _: Option<batter::runlimit::EmptyChecks> = None;
/// let _: Option<batter::runlimit::native::PolicyId> = None;
/// ```
#[cfg(feature = "runlimit")]
pub mod runlimit {
    pub use batter_runlimit::*;
}

/// Typed flags and kill switches: declared mode and the optional control plane.
///
/// Enabled by the `flags` feature, which is experimental. The implementation
/// remains [`batter_flags`]; `flags::core` is the `batter-flags-core` catalog
/// crate itself, so handles, decisions and views keep one type identity through
/// the direct and facade paths. `flags-postgres` adds `flags::postgres` and the
/// protected `flags::control_plane` registration, the `flags::grants`
/// inventory and selects `sqlx`, so `batter::sqlx::pool_in` owns the pool the
/// control plane takes; `flags-axum` adds `flags::http` and selects `axum`;
/// `flags-runledger` adds `flags::jobs` and selects `runledger`;
/// `flags-test-support` adds `flags::test::TestFlags`, for test builds only. The
/// `rollout` example shows declared mode and the control plane.
#[cfg(feature = "flags")]
pub mod flags {
    pub use batter_flags::*;
}

/// Generic scripted outcomes and body/cleanup result combination for tests.
///
/// Enabled by the `test-support` feature. This does not select SQLx or the
/// external PostgreSQL harness.
///
/// ```
/// let _: Option<batter::test_support::Script<(), ()>> = None;
/// ```
#[cfg(feature = "test-support")]
pub mod test_support {
    pub use batter_test_support::*;
}

/// Optional bounded OTLP metrics adapter, driven by protected service completion.
#[cfg(feature = "otlp")]
pub use batter_otlp as otlp;

#[cfg(test)]
mod tests {
    use super::{BoxError, ConfigurationError, RegistrationError, lifecycle, operation};

    #[test]
    fn legacy_root_types_are_available_through_the_facade() {
        let _: fn(ConfigurationError) -> BoxError = BoxError::from;
        let _: fn(RegistrationError) -> BoxError = BoxError::from;
        let _: Option<operation::Interruption> = None;
        let _: Option<lifecycle::Readiness> = None;
    }
}
