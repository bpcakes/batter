use super::{PgFailurePolicy, PgPolicyScope, resolve};
use crate::{
    PgAtomicScope, PgRollbackConfirmed, PgScopeError, PgScopeFailure, PgScopedSql,
    PgSessionProfile,
    atomic::{PgAtomicTransaction, fast::FastOperation, scope::ScopeSavepoint},
};
use sqlx::PgPool;
use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

/// The transaction was already rolled back after a fail-fast SQL rejection.
///
/// The original error was returned by that SQL call. If the body catches it,
/// subsequent SQL calls return this evidence through the consumer's `From`
/// implementation; returning `Ok` from the body does the same. A caught error
/// cannot restore work or commit authority. This is acknowledged rollback, not
/// uncertainty or permission to replay. Nontransactional/external effects are
/// not reversed. The evidence cannot be constructed by consumers.
///
/// ```compile_fail,E0451
/// let fabricated = batter_sqlx::PgScopeRolledBack { _confirmed: todo!() };
/// ```
#[derive(Clone)]
pub struct PgScopeRolledBack {
    _confirmed: Arc<PgRollbackConfirmed>,
}

impl fmt::Debug for PgScopeRolledBack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for PgScopeRolledBack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgreSQL transaction already rolled back after scope rejection")
    }
}
impl Error for PgScopeRolledBack {}

/// Execute with fail-fast SQL: an ordinary SQL/body rejection acknowledges
/// rollback of the whole transaction before returning its consumer error.
///
/// `scope.sql(...)` does not create a per-operation savepoint. One private guard
/// is created at transaction birth so failed SQL cannot hide a replaced boundary.
/// Boundary/validation loss, interruption and failed cleanup remain terminal and
/// uncertain. Success still requires acknowledged commit. Use
/// [`PgPolicyScope::recoverable_sql`] for an explicitly recoverable operation, or
/// [`super::run_atomic_with`] for the existing recoverable default.
///
/// `P::Error: From<PgScopeRolledBack>` handles attempts to continue after a caught
/// rejection. The required uncertainty policy is unchanged. Application-created
/// savepoints persist across successful fast calls, until transaction completion
/// or rejection; use a recoverable scope when savepoint confinement is needed.
///
/// ```no_run
/// use batter_sqlx::{PgFailurePolicy, PgScopeRolledBack};
/// async fn append<P: PgFailurePolicy<i64>>(
///     pool: &sqlx::PgPool, policy: &P,
/// ) -> Result<i64, P::Error>
/// where P::Error: From<sqlx::Error> + From<PgScopeRolledBack> {
///     batter_sqlx::run_atomic_fail_fast_with(pool, policy, async |mut scope| {
///         scope.sql(async |sql| {
///             let id = sqlx::query_scalar::<_, i64>(
///                 "INSERT INTO audit_events DEFAULT VALUES RETURNING id"
///             ).fetch_one(sql.executor()).await?;
///             Ok(id)
///         }).await
///     }).await
/// }
/// ```
/// Caught rejection handling must be selected at construction:
/// ```compile_fail,E0277
/// async fn missing<P: batter_sqlx::PgFailurePolicy<()>>(pool: &sqlx::PgPool, policy: &P) {
///     batter_sqlx::run_atomic_fail_fast_with(pool, policy, async |_| Ok(())).await;
/// }
/// ```
pub async fn run_atomic_fail_fast_with<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, P::Error>
where
    P::Error: From<PgScopeRolledBack>,
{
    run(pool, None, policy, work, RollbackSlot::default()).await
}

/// Profiled fail-fast workflow. Profiled calls retain the opening catalog check:
/// another session can revoke schema authority between operations.
pub async fn run_atomic_profiled_fail_fast_with<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    profile: &PgSessionProfile,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, P::Error>
where
    P::Error: From<PgScopeRolledBack>,
{
    run(pool, Some(profile), policy, work, RollbackSlot::default()).await
}

pub(super) async fn run<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    profile: Option<&PgSessionProfile>,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
    rolled_back: RollbackSlot,
) -> Result<T, P::Error>
where
    P::Error: From<PgScopeRolledBack>,
{
    let mut owner = match profile {
        Some(profile) => PgAtomicTransaction::begin_profiled(pool, profile).await,
        None => PgAtomicTransaction::begin(pool).await,
    }
    .map_err(|cause| policy.begin_failed(cause))?;
    let guard = owner
        .begin_fast_guard()
        .await
        .map_err(|cause| policy.begin_failed(cause))?;
    let mut scope = FastScope {
        inner: PgAtomicScope::new(owner),
        guard,
        rolled_back,
    };
    let result = work(PgPolicyScope::new_fast(&mut scope, policy)).await;
    match scope.rolled_back.get() {
        Some(confirmed) => match result {
            Err(rejection) => Err(rejection),
            Ok(_) => Err(confirmed.into()),
        },
        None => resolve(policy, scope.inner.finish(result).await),
    }
}

// Shared with the budget resolver: acknowledgement must survive callback cancellation.
#[derive(Clone, Default)]
pub(super) struct RollbackSlot(Arc<Mutex<Option<PgScopeRolledBack>>>);

impl RollbackSlot {
    pub(super) fn get(&self) -> Option<PgScopeRolledBack> {
        self.0.lock().expect("private rollback slot").clone()
    }

    fn set(&self, confirmed: PgScopeRolledBack) {
        *self.0.lock().expect("private rollback slot") = Some(confirmed);
    }
}

pub(super) struct FastScope {
    inner: PgAtomicScope,
    guard: ScopeSavepoint,
    rolled_back: RollbackSlot,
}

pub(super) enum FastError<E> {
    Application(E),
    Terminal(PgScopeFailure<E>),
    RolledBack(PgScopeRolledBack),
}

impl FastScope {
    pub(super) async fn sql<T, E>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<T, FastError<E>> {
        if let Some(confirmed) = self.rolled_back.get() {
            return Err(FastError::RolledBack(confirmed));
        }
        let owner = self
            .inner
            .take_owner()
            .map_err(|cause| FastError::Terminal(cause.into()))?;
        match owner.fast_operation(&self.guard, work).await {
            Ok(FastOperation::Continue(owner, value)) => {
                self.inner.restore(owner);
                Ok(value)
            }
            Ok(FastOperation::Rejected(confirmed, error)) => {
                self.rolled_back.set(PgScopeRolledBack {
                    _confirmed: Arc::new(confirmed),
                });
                Err(FastError::Application(error))
            }
            Err(failure) => {
                self.inner.poison(failure.loss());
                Err(FastError::Terminal(failure))
            }
        }
    }

    pub(super) async fn recoverable<T, E>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<T, FastError<E>> {
        if let Some(confirmed) = self.rolled_back.get() {
            return Err(FastError::RolledBack(confirmed));
        }
        self.inner
            .application(work)
            .await
            .map_err(|error| match error {
                PgScopeError::Application(error) => FastError::Application(error),
                PgScopeError::Terminal(failure) => FastError::Terminal(failure),
            })
    }
}
