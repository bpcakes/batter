//! Batter's public facade.
//!
//! The operational implementation lives in [`batter_core`]. This crate keeps
//! the established `batter::...` paths source-compatible and exposes optional
//! adapter namespaces without changing native ownership.
//!
//! The default feature set is empty. Enable only the namespaces an application
//! uses: `at-rest`, `axum`, `metrics`, `otlp`, `sqlx`, `runledger`, `runlimit`,
//! `test-support`, or the narrower bridge features `runlimit-memory`, `runlimit-postgres`,
//! `runlimit-axum`, `runlimit-native-http`, `runlimit-native-axum`,
//! `runledger-test-support`, and `sqlx-test-support`.
//!
//! # Important limits
//!
//! Deadlines drop futures; they do not undo external effects. Cancellation is
//! cooperative, not preemption. Supervision owns directly registered tasks,
//! not tasks secretly spawned by a component. Owned startup, command and service
//! drivers retain registered cleanup independently of borrowed waiters. Direct
//! `CleanupStack::close` driving remains caller-owned. See `docs/guarantees.md`.
//!
//! # Choosing between supported paths
//!
//! Each row is a policy choice between supported APIs, not a gap in the
//! protected path. Namespaces behind a feature are named with that feature.
//!
//! | Need | Use | Not |
//! | --- | --- | --- |
//! | A long-running service | [`startup::Startup::scoped`], `with_unix_signals`, `start`, then [`lifecycle::RunningSupervisor::wait_checked`]; with metrics export, [`service::start`] and `otlp::prepare` (feature `otlp`) | driving [`lifecycle::Supervisor::start`] yourself, or awaiting the raw report without the checked form |
//! | Finite work: setup, migration, an operator command | [`command::Command::new`] or [`command::Command::within`], then [`command::check_command`] | `Startup`, which reports `EmptySupervisor` for a process with no critical component |
//! | A PostgreSQL pool the service owns | `sqlx::pool_in` after `reserve_cleanup` (feature `sqlx`); `sqlx::PgProfiledPool` when a session profile is declared; `runledger::RunledgerDatabase` when Runledger runs on the same pool (feature `runledger`) | a pool built outside startup with a manual close |
//! | A transaction write | `sqlx::run_atomic_in`, or `sqlx::run_atomic_with_in` for an exhaustive failure policy; use their `run_atomic_profiled_in` / `run_atomic_profiled_with_in` counterparts with a profiled pool. For a durable job in the same transaction, wrap `runledger::run_atomic` in the caller's `context.run` | `sqlx::low_level::PgAtomicTransaction`, which leaves commit confirmation to the caller |
//! | Quota admission | `runlimit::http::HttpQuota` when Batter orders deadline, authentication and quota before the body (feature `runlimit-axum`); `runlimit::Quota::run` inside your own work; `runlimit::native_transport` when the application owns subject derivation and rejection mapping (features `runlimit-native-http`, `runlimit-native-axum`) | the native Tower layer under `HttpBoundary`, which receives no deadline or ordering from it |
//! | A job handler's budget | `runledger::job_phases(execution, reserve)`: provider work under `work()`, the final-state write under `finalization()`; a positive reserve keeps time for that write and `Duration::ZERO` keeps none | a fresh root sized from the remaining budget, which never sees the invocation's exit |
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
/// `runledger::native::test_support` for a consumer's own tests. They are
/// low-level: `register_in` remains the protected composition, and reaching a
/// native namespace does not move ownership of durable policy, storage or
/// supervision into the facade.
///
/// Inside a native handler, `runledger::job_phases` derives work and
/// finalization contexts from the invocation: the worker's own deadline bounds
/// them and the invocation's exit cancels them. Handlers join a native
/// `JobCatalog` or `JobRegistry`; `register_in` registers the prepared worker,
/// not a handler.
///
/// ```
/// use batter::operation::OperationError;
/// use batter::runledger::native::core::jobs::{
///     JobCompletion, JobExecution, JobExecutionHandler, JobFailure, JobType,
/// };
/// use batter::runledger::native::core::prelude::async_trait;
/// use batter::runledger::native::runtime::catalog::JobCatalog;
/// use batter::runledger::{JobPhasesRejection, job_phases};
/// use serde_json::Value;
/// use std::time::Duration;
///
/// /// Time kept inside the native deadline for recording the outcome.
/// const RECORD_RESERVE: Duration = Duration::from_millis(300);
///
/// struct Notify;
///
/// #[async_trait]
/// impl JobExecutionHandler for Notify {
///     fn job_type(&self) -> JobType<'static> {
///         JobType::new("example.notify")
///     }
///
///     async fn execute(
///         &self,
///         execution: JobExecution<'_>,
///         _payload: Value,
///     ) -> Result<JobCompletion, JobFailure> {
///         let phases = job_phases(execution, RECORD_RESERVE).map_err(|rejection| {
///             match rejection {
///                 JobPhasesRejection::Exhausted | JobPhasesRejection::Ended => {
///                     JobFailure::timeout("example.no_work_time", "No work time remained.")
///                 }
///                 JobPhasesRejection::Unsupported | JobPhasesRejection::Reserve(_) => {
///                     JobFailure::terminal("example.unsupported", "Invocation phases unavailable.")
///                 }
///             }
///         })?;
///         let sent = phases
///             .work()
///             .run("example.send", |_scope| async { Ok::<_, std::io::Error>("receipt") })
///             .await;
///         let outcome = match sent {
///             Ok(_receipt) => Ok(JobCompletion::success()),
///             Err(OperationError::Failed(_)) => {
///                 Err(JobFailure::retryable("example.refused", "The provider refused."))
///             }
///             Err(OperationError::Interrupted(_)) => {
///                 Err(JobFailure::timeout("example.unknown", "Delivery is unknown."))
///             }
///         };
///         phases
///             .finalization()
///             .run("example.record", |_scope| async { Ok::<_, std::io::Error>(()) })
///             .await
///             .map_err(|_| JobFailure::retryable("example.unrecorded", "Not recorded."))?;
///         outcome
///     }
/// }
///
/// let _catalog = JobCatalog::new().handler(Notify.into_job_handler());
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
/// ```
/// let _: Option<batter::runlimit::EmptyChecks> = None;
/// let _: Option<batter::runlimit::native::PolicyId> = None;
/// ```
#[cfg(feature = "runlimit")]
pub mod runlimit {
    pub use batter_runlimit::*;
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
