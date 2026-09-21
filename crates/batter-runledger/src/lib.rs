//! Optional Runledger lifecycle translation. Native supervision and durable job
//! policy stay in Runledger; Batter owns process registration and dependency cleanup.
//!
//! [`register_in`] starts prepared native work only after validated protected
//! registration and driver startup. Native local-loop acknowledgement controls component readiness;
//! dependency health and explicit application approval remain separate. No durable
//! startup control job, termination gate or application report channel is required.

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
    AcceptedIntentOutcome, AcceptedIntentState, PgAtomicError, PgAtomicUncertainty, PgIntentScope,
    PgQueueScope, PgScopeFailure, PgScopeLoss, PgSessionProfile, RequiredIntentError,
    RunledgerDatabase, SchemaCompatibilitySnapshot,
    ensure_schema_compatible_after_idempotency_cutover as verify_schema, run_atomic,
};

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
///         OperationContext::new(Duration::from_secs(1))?,
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
