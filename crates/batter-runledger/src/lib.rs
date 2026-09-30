//! Optional Runledger lifecycle translation. Native supervision and durable job
//! policy stay in Runledger; Batter owns process registration and dependency cleanup.
//!
//! [`register_in`] starts prepared native work only after validated protected
//! registration and driver startup. Native local-loop acknowledgement controls component readiness;
//! dependency health and explicit application approval remain separate. No durable
//! startup control job, termination gate or application report channel is required.
//! After startup transfers the running owner, await
//! [`batter_core::lifecycle::RunningSupervisor::wait_checked`] for the complete
//! managed settlement and process cleanup classification. A native settlement
//! cannot by itself prove remote effects or arbitrary detached work stopped.
//!
//! Direct adapter consumers can implement [`PgFailurePolicy`] using only this
//! crate's exports, retaining the concrete causes and provisional outcomes:
//!
//! ```
//! use batter_runledger::{PgFailurePolicy, PgScopeFailure, PgScopeLoss, PgTransactionError};
//!
//! enum Failure {
//!     Begin(PgTransactionError),
//!     Scope(Box<PgScopeFailure<Failure>>),
//!     Commit(u64, PgTransactionError),
//!     Rollback(Box<Failure>, PgTransactionError),
//!     Lost(Result<u64, Box<Failure>>, PgScopeLoss),
//! }
//! struct Policy;
//! impl PgFailurePolicy<u64> for Policy {
//!     type Error = Failure;
//!     fn begin_failed(&self, cause: PgTransactionError) -> Failure {
//!         Failure::Begin(cause)
//!     }
//!     fn scope_lost(&self, failure: PgScopeFailure<Failure>) -> Failure {
//!         Failure::Scope(Box::new(failure))
//!     }
//!     fn commit_unconfirmed(&self, output: u64, cause: PgTransactionError) -> Failure {
//!         Failure::Commit(output, cause)
//!     }
//!     fn rollback_unconfirmed(&self, rejection: Failure, cause: PgTransactionError) -> Failure {
//!         Failure::Rollback(Box::new(rejection), cause)
//!     }
//!     fn scope_lost_after_body(&self, result: Result<u64, Failure>, cause: PgScopeLoss) -> Failure {
//!         Failure::Lost(result.map_err(Box::new), cause)
//!     }
//! }
//! # let _ = Policy;
//! ```

#![forbid(unsafe_code)]

use batter_core::{
    BoxError, RegistrationError,
    lifecycle::{ManagedComponent, ManagedSettlement, Supervisor},
    operation::OperationContext,
    registration::{Registration, RegistrationTarget},
};
use runledger_runtime::{
    PreparedSupervisor, RuntimeSettlement, RuntimeShutdownBudget, RuntimeShutdownReport,
    RuntimeShutdownSignal,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

pub use runledger_postgres::{
    AcceptedIntentOutcome, AcceptedIntentState, PgAtomicError, PgAtomicUncertainty,
    PgFailurePolicy, PgIntentScope, PgPolicyIntentScope, PgPolicyQueueScope, PgQueueScope,
    PgScopeFailure, PgScopeLoss, PgScopeRolledBack, PgSessionProfile, PgTransactionError,
    RequiredIntentError, RunledgerDatabase, SchemaCompatibilitySnapshot,
    ensure_schema_compatible_after_idempotency_cutover as verify_schema, run_atomic,
    run_atomic_fail_fast_with, run_atomic_with,
};

/// The exact native Runledger packages this adapter resolves.
///
/// These namespaces exist so one `batter` dependency can reach the native
/// worker, catalog, configuration, queue and migration APIs that this adapter
/// does not translate. They are the native packages themselves, so every type
/// has the same identity through the direct and facade paths.
///
/// They are deliberately low-level. Runledger keeps ownership of durable job
/// policy, persistence, scheduling and descendant supervision; nothing here is
/// wrapped, re-validated or brought under Batter's protected lifecycle. A caller
/// that builds a live native supervisor itself takes on the obligations
/// [`register_in`] otherwise discharges: observing initialization,
/// requesting shutdown within a budget, classifying the settlement and ordering
/// dependency cleanup after it. Use [`register_in`] with an inert
/// [`native::runtime::PreparedSupervisor`] for the protected composition; reach for
/// these namespaces for the preparation, schema and queue operations that have
/// no Batter-owned equivalent.
///
/// Runledger's fifth package, the `runledger-tui` operator binary, has no
/// library target and therefore no namespace here.
///
/// ```
/// use batter_runledger::native::{core, postgres, runtime};
/// use std::time::Duration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let payload = serde_json::json!({"name": "example"});
/// let intent = postgres::jobs::JobEnqueueIntent::new(
///     core::jobs::JobType::new("example.greeting"),
///     &payload,
///     "example-greeting-1",
/// )
/// .with_max_attempts(1);
/// let config = runtime::config::IntentPromoterConfig::new(Duration::from_millis(50), 10);
/// config.validate()?;
/// # let _ = intent;
/// # Ok(())
/// # }
/// # example().unwrap();
/// ```
pub mod native {
    /// Native contracts: job types, handlers, payload specs and failure kinds.
    pub use runledger_core as core;
    /// Native persistence: migrations, queue operations and durable intents.
    pub use runledger_postgres as postgres;
    /// Native execution: worker configuration, catalog, registry and supervisor.
    pub use runledger_runtime as runtime;

    /// Runledger's own PostgreSQL test support, selected with `test-support`.
    ///
    /// Test-only reachability so an external consumer's tests need no second
    /// native declaration. Docker provisioning, the shared container, the
    /// ephemeral-database connection budget and teardown remain Runledger's;
    /// this is not a Batter fixture harness and callers still own creating,
    /// consuming and dropping each disposable database. Never select it in a
    /// deployed graph. Selecting it only in `[dev-dependencies]` requires Cargo
    /// resolver 2 or 3 to keep it out of ordinary builds; set `resolver = "3"`
    /// under `[workspace]` at a virtual workspace root. Resolver 1 unifies the
    /// development features into normal dependencies too. Other selected
    /// features or dependencies must not enable it in the deployed graph either.
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), sqlx::Error> {
    /// let database =
    ///     batter_runledger::native::test_support::create_ephemeral_database("example").await?;
    /// // The caller owns the database for as long as it needs it, then drops it.
    /// let _url = database.url().to_owned();
    /// database.teardown().await
    /// # }
    /// ```
    #[cfg(feature = "test-support")]
    pub use runledger_test_support as test_support;
}

/// Original native shutdown evidence. The process report retains this concrete
/// type under `managed[*].outcome.settlement`; use `downcast_ref::<NativeReport>()`
/// for explicit inspection. Default formatting uses native redacted diagnostics.
/// The classification is owned and cannot be forged or relabeled by consumers:
///
/// ```compile_fail
/// # fn forge(settlement: runledger_runtime::RuntimeSettlement) {
/// let report = batter_runledger::NativeReport { settlement };
/// # }
/// ```
#[derive(Debug)]
pub struct NativeReport {
    settlement: RuntimeSettlement,
}

impl NativeReport {
    /// Complete native evidence, retained inside its consuming classification.
    pub fn native(&self) -> &RuntimeShutdownReport {
        self.settlement.report()
    }

    /// Borrow the unforgeable native settlement without extracting its authority.
    pub fn settlement(&self) -> &RuntimeSettlement {
        &self.settlement
    }
}

impl ManagedSettlement for NativeReport {
    fn is_success(&self) -> bool {
        matches!(self.settlement, RuntimeSettlement::Clean(_))
    }
    fn allows_dependency_cleanup(&self) -> bool {
        matches!(
            self.settlement,
            RuntimeSettlement::Clean(_) | RuntimeSettlement::StoppedWithFailures(_)
        )
    }
}

/// Register owned, validated native preparation. This accepts no live supervisor
/// or application factory. Name validation precedes transfer, and native tasks
/// start only when the process driver takes ownership.
/// Schema/catalog synchronization and dependency acquisition belong to owned
/// application startup before registration.
///
/// Prefer [`register_in`] inside canonical protected startup. This signature
/// remains for lower-level direct-supervisor composition.
/// A live native supervisor cannot be passed through the protected boundary:
///
/// ```compile_fail,E0308
/// # fn example(process: &mut batter_core::lifecycle::Supervisor, context: batter_core::operation::OperationContext, live: runledger_runtime::Supervisor) {
/// batter_runledger::register(process, "worker", context, live).unwrap();
/// # }
/// ```
/// A closure cannot hide an already-running supervisor either:
///
/// ```compile_fail,E0308
/// # fn example(process: &mut batter_core::lifecycle::Supervisor, context: batter_core::operation::OperationContext, live: runledger_runtime::Supervisor) {
/// batter_runledger::register(process, "worker", context, move || Ok(live)).unwrap();
/// # }
/// ```
pub fn register(
    process: &mut Supervisor,
    name: &'static str,
    startup: OperationContext,
    prepared: PreparedSupervisor,
) -> Result<(), RegistrationError> {
    register_impl(process.registration(), name, startup, prepared)
}

/// Register owned native preparation through constrained registration authority.
///
/// The protected target never receives a live native supervisor and exposes no
/// process-start or cleanup-extraction operations. Native initialization, stop,
/// and complete settlement remain identical to [`register`].
///
/// ```no_run
/// use batter_core::{BoxError, operation::OperationContext,
///     startup::ProtectedStartupScope};
/// use runledger_runtime::{config::JobsConfig, registry::JobRegistry};
/// use std::time::Duration;
///
/// fn register(
///     scope: &mut ProtectedStartupScope,
///     pool: &sqlx::PgPool,
///     config: JobsConfig,
///     registry: JobRegistry,
/// ) -> Result<(), BoxError> {
///     let native = runledger_runtime::Supervisor::builder(pool, config)?
///         .with_registry(registry).prepare()?;
///     batter_runledger::register_in(
///         scope,
///         "worker",
///         batter_core::operation::OperationOwner::new(Duration::from_secs(1))?.into_context(),
///         native,
///     )?;
///     Ok(())
/// }
/// ```
pub fn register_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    name: &'static str,
    startup: OperationContext,
    prepared: PreparedSupervisor,
) -> Result<(), RegistrationError> {
    register_impl(target.registration(), name, startup, prepared)
}

fn register_impl(
    mut registration: Registration<'_>,
    name: &'static str,
    startup: OperationContext,
    prepared: PreparedSupervisor,
) -> Result<(), RegistrationError> {
    registration.register_managed(name, startup, move |budget| {
        let budget = RuntimeShutdownBudget::new(budget.graceful(), budget.abort())?;
        let native = prepared.start();
        let initialization = native.startup_observer();
        let stop = native.shutdown_handle();
        let stopping = stop.clone();
        Ok(ManagedComponent::new(
            async move {
                initialization
                    .wait_initialized()
                    .await
                    .map_err(|error| Box::new(error) as BoxError)
            },
            async move { stopping.requested().await },
            move |started| stop.request_shutdown_since(started),
            ReportFuture {
                inner: Box::pin(
                    native.run_until_shutdown_report(RuntimeShutdownSignal::pending(), budget),
                ),
            },
        ))
    })
}

// Preserve the native report before destroying its completed driver future.
// An async mapping with an owned await temporary could drop it before transfer.
struct ReportFuture<F> {
    inner: Pin<Box<F>>,
}
impl<F: Future<Output = RuntimeShutdownReport>> Future for ReportFuture<F> {
    type Output = NativeReport;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.as_mut().poll(cx).map(|native| NativeReport {
            settlement: native.classify(),
        })
    }
}
