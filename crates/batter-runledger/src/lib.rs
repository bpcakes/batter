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
use batter_sqlx::{PgExecutor, PgSession};
use runledger_runtime::{
    PreparedSupervisor, RuntimeSettlement, RuntimeShutdownBudget, RuntimeShutdownReport,
    RuntimeShutdownSignal,
};
use sqlx::{Executor, Postgres};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Opaque Batter transaction accepted by Runledger's transactional enqueue APIs.
///
/// Application SQL and Runledger enqueueing share this exact transaction, but
/// neither caller can extract or replace its SQLx connection. Explicit commit
/// and rollback remain application-owned.
///
/// ```no_run
/// use batter_runledger::RunledgerTransaction;
/// use runledger_core::jobs::JobType;
/// use runledger_postgres::jobs::{
///     JobEnqueue, enqueue_job_with_outcome_in_transaction,
/// };
/// use serde_json::json;
///
/// # async fn submit(session: &mut batter_sqlx::PgSession<'_>) -> Result<(), Box<dyn std::error::Error>> {
/// let mut transaction = RunledgerTransaction::begin(session).await?;
/// sqlx::query("INSERT INTO application_audit (message) VALUES ('queued')")
///     .execute(transaction.executor())
///     .await?;
/// let payload = json!({"kind": "example"});
/// enqueue_job_with_outcome_in_transaction(&mut transaction, &JobEnqueue {
///     job_type: JobType::new("example.job"),
///     organization_id: None,
///     payload: &payload,
///     priority: None,
///     max_attempts: None,
///     timeout_seconds: None,
///     next_run_at: None,
///     idempotency_key: Some("example-job-1"),
///     stage: None,
/// }).await?;
/// transaction.commit().await?;
/// # Ok(()) }
/// ```
#[must_use = "explicitly commit or roll back the transaction before completing the lease work"]
pub struct RunledgerTransaction<'connection> {
    transaction: batter_sqlx::PgTransaction<'connection>,
}

impl<'connection> RunledgerTransaction<'connection> {
    /// Begin one explicit transaction from the lease's opaque session.
    pub async fn begin(session: &'connection mut PgSession<'_>) -> Result<Self, sqlx::Error> {
        Ok(Self {
            transaction: session.begin().await?,
        })
    }

    /// Borrow this exact transaction for one native SQLx operation.
    pub fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.transaction.executor()
    }

    /// Commit and consume the transaction.
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        self.transaction.commit().await
    }

    /// Roll back and consume the transaction.
    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        self.transaction.rollback().await
    }
}

impl PgExecutor for RunledgerTransaction<'_> {
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.executor()
    }
}

impl runledger_postgres::PgTransactionExecutor for RunledgerTransaction<'_> {
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.executor()
    }
}

impl std::fmt::Debug for RunledgerTransaction<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RunledgerTransaction")
    }
}

/// Run native schema compatibility checks through an opaque Batter session.
///
/// The caller keeps this future inside `PgLease::with_connection` and its
/// operation budget. No connection is exposed or acquired by this bridge;
/// native Runledger owns the schema contract and Batter owns lease disposition.
pub async fn verify_schema(
    session: &mut PgSession<'_>,
) -> Result<(), runledger_postgres::SchemaCompatibilityError> {
    runledger_postgres::ensure_schema_compatible_after_idempotency_cutover_with_executor(
        &mut SchemaSession(session),
    )
    .await
}

struct SchemaSession<'a, 'c>(&'a mut PgSession<'c>);
impl runledger_postgres::PgSessionExecutor for SchemaSession<'_, '_> {
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.0.executor()
    }
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
