//! Optional Runledger lifecycle translation. Native supervision and durable job
//! policy stay in Runledger; Batter owns process registration and dependency cleanup.
//!
//! [`register`] starts prepared native work only after validated registration and
//! driver startup. Native local-loop acknowledgement controls component readiness;
//! dependency health and explicit application approval remain separate. No durable
//! startup control job, termination gate or application report channel is required.

#![forbid(unsafe_code)]

use batter::{
    BoxError, RegistrationError,
    lifecycle::{ManagedComponent, ManagedSettlement, Supervisor},
    operation::OperationContext,
    registration::{Registration, RegistrationTarget},
};
use runledger_runtime::{PreparedSupervisor, RuntimeShutdownBudget, RuntimeShutdownReport};
use std::{
    future::{Future, pending},
    pin::Pin,
    task::{Context, Poll},
};

/// Original native shutdown evidence. The process report retains this concrete
/// type under `managed[*].outcome.settlement`; use `downcast_ref::<NativeReport>()`
/// for explicit inspection. Default formatting uses native redacted diagnostics.
#[derive(Debug)]
pub struct NativeReport {
    /// First cause, every observed native loop/descendant outcome and unresolved work.
    pub native: RuntimeShutdownReport,
}

impl ManagedSettlement for NativeReport {
    fn is_success(&self) -> bool {
        self.native.is_success()
    }
    fn allows_dependency_cleanup(&self) -> bool {
        self.native.is_cooperatively_stopped()
    }
}

/// Register owned, validated native preparation. This accepts no live supervisor
/// or application factory. Name validation precedes transfer, and native tasks
/// start only when the process driver takes ownership.
/// Schema/catalog synchronization and dependency acquisition belong to owned
/// application startup before registration.
///
/// ```no_run
/// use batter::{BoxError, cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor},
///     operation::OperationContext};
/// use runledger_runtime::{config::JobsConfig, registry::JobRegistry};
/// use std::time::Duration;
/// # async fn example(pool: sqlx::PgPool, config: JobsConfig, registry: JobRegistry) -> Result<(), BoxError> {
/// let second = Duration::from_secs(1);
/// let cleanup = CleanupBudget::new(second, second, second)?;
/// let mut process = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
/// let native = runledger_runtime::Supervisor::builder(&pool, config)?
///     .with_registry(registry).prepare()?;
/// batter_runledger::register(&mut process, "worker", OperationContext::new(second)?, native)?;
/// // Separate fresh dependency health and application approval belong here.
/// let running = process.start();
/// running.handle().mark_ready();
/// running.handle().wait_ready().await.unwrap();
/// batter::lifecycle::check_shutdown(running.shutdown().await)?;
/// # Ok(()) }
/// ```
/// A live native supervisor cannot be passed through the protected boundary:
///
/// ```compile_fail,E0308
/// # fn example(process: &mut batter::lifecycle::Supervisor, context: batter::operation::OperationContext, live: runledger_runtime::Supervisor) {
/// batter_runledger::register(process, "worker", context, live).unwrap();
/// # }
/// ```
/// A closure cannot hide an already-running supervisor either:
///
/// ```compile_fail,E0308
/// # fn example(process: &mut batter::lifecycle::Supervisor, context: batter::operation::OperationContext, live: runledger_runtime::Supervisor) {
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
                inner: Box::pin(native.run_until_shutdown_report(pending(), budget)),
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
        self.inner
            .as_mut()
            .poll(cx)
            .map(|native| NativeReport { native })
    }
}
