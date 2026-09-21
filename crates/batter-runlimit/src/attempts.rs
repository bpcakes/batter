//! Pre-authentication admission, bounded verification, and atomic outcome writes.
//!
//! [`AttemptRunner`](crate::attempts::AttemptRunner) reserves one native attempt before invoking verification.
//! Verification receives the original operation lineage, but no database scope.
//! The application makes its final decision inside the atomic callback, where it
//! can check replay state and current account status. Both accepted and rejected
//! decisions commit their application writes together with native attempt state.
//! Operational errors instead roll back. No automatic replay or quota refund.
//!
//! ```no_run
//! use batter_core::operation::OperationContext;
//! use batter_runlimit::{attempts::{AttemptRunner, Authentication}, native::{
//!     KeyHasher, PolicyId, ScopeId, QuotaPeriod, attempts::AttemptPolicy,
//! }};
//! use std::{convert::Infallible, time::Duration};
//! # async fn example(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
//! let period = |seconds| QuotaPeriod::new(Duration::from_secs(seconds));
//! let policy = AttemptPolicy::new(PolicyId::new("credential.verify")?,
//!     ScopeId::new("identifier")?, period(1)?, period(60)?, period(300)?, period(30)?)?;
//! let hasher = KeyHasher::new([7; 32])?; // Example only; inject the application's key.
//! let subject = hasher.hash_attempt_for(&policy, "normalized-identifier");
//! let context = OperationContext::new(Duration::from_secs(2))?;
//! let runner = AttemptRunner::new(pool);
//! let result = runner.run(&context, subject,
//!     |_scope| async { Ok::<_, Infallible>(true) }, // Bounded credential verification.
//!     async |sql, credential_matches| {
//!         // Final replay and account-status checks also belong in this transaction.
//!         sqlx::query("INSERT INTO access_audit (accepted) VALUES ($1)")
//!             .bind(credential_matches).execute(sql.executor()).await?;
//!         Ok::<Authentication<(), ()>, sqlx::Error>(if credential_matches {
//!             Authentication::Accepted(())
//!         } else {
//!             Authentication::Rejected(())
//!         })
//!     }).await?;
//! match result.into_authentication() {
//!     Authentication::Accepted(()) => { /* publish the committed session */ }
//!     Authentication::Rejected(()) => { /* publish the generic rejection */ }
//! }
//! # Ok(()) }
//! ```
//!
//! Attempt receipts and separate completion cannot escape the runner:
//! ```compile_fail
//! fn extract(runner: batter_runlimit::attempts::AttemptRunner) {
//!     let receipt = runner.receipt();
//! }
//! ```
//! An application callback must distinguish durable rejection from operational error:
//! ```compile_fail
//! # async fn wrong(runner: &batter_runlimit::attempts::AttemptRunner,
//! # context: &batter_core::operation::OperationContext,
//! # subject: batter_runlimit::native::attempts::AttemptSubject<'_>) {
//! runner.run(context, subject, |_| async { Ok::<_, ()>(()) },
//!     async |_, ()| Ok::<bool, ()>(false)).await;
//! # }
//! ```

use batter_core::operation::{Interruption, OperationContext, OperationError};
use batter_sqlx::{PgAtomicError, PgScopeError, PgScopedSql};
use runlimit_core::attempts::{
    AttemptAdmission, AttemptCompletion, AttemptDenial, AttemptObservation, AttemptObserver,
    AttemptOutcome, AttemptSubject, StagedAttemptCompletion, observe_attempt_safely,
};
use runlimit_postgres::{
    CheckError, PostgresConfig,
    attempts::{PgAttemptClaimResult, PostgresAttemptLimiter},
};
use sqlx::PgPool;
use std::{error::Error, fmt, future::Future, sync::Arc};

/// The application's final decision, made under the owned database transaction.
/// A rejected authentication is a durable business outcome, not an instruction
/// to roll back its failure audit or consecutive-failure transition.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Authentication<T, R> {
    /// Fully accepted after all transactional checks.
    Accepted(T),
    /// Rejected; failure audit and retry-state writes must still commit.
    Rejected(R),
}

impl<T, R> fmt::Debug for Authentication<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Accepted(_) => "Authentication::Accepted",
            Self::Rejected(_) => "Authentication::Rejected",
        })
    }
}

impl<T, R> Authentication<T, R> {
    fn outcome(&self) -> AttemptOutcome {
        match self {
            Self::Accepted(_) => AttemptOutcome::Success,
            Self::Rejected(_) => AttemptOutcome::Failure,
        }
    }
}

/// Application output and native retry-state evidence from the same transaction.
/// It is confirmed only as the runner's `Ok` result. In `PgAtomicError::Uncertain`
/// it remains provisional, just like every other atomic body output.
pub struct AttemptCompletionOutput<T, R> {
    authentication: Authentication<T, R>,
    completion: AttemptCompletion,
}

impl<T, R> AttemptCompletionOutput<T, R> {
    /// Inspect the application decision without losing its category.
    pub fn authentication(&self) -> &Authentication<T, R> {
        &self.authentication
    }

    /// Inspect the native consecutive-failure transition.
    pub fn completion(&self) -> AttemptCompletion {
        self.completion
    }

    /// Consume the output while preserving accepted versus rejected meaning.
    pub fn into_authentication(self) -> Authentication<T, R> {
        self.authentication
    }
}

impl<T, R> fmt::Debug for AttemptCompletionOutput<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AttemptCompletionOutput")
    }
}

/// Which phase was interrupted. No phase implies that remote work rolled back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptPhase {
    /// A reservation may have committed without acknowledgement.
    Admission,
    /// A reservation was acknowledged, but verification did not complete.
    Verification,
    /// The atomic workflow's disposition was not observed.
    Completion,
}

/// Failure inside the owned transaction. Authentication denial belongs in
/// [`Authentication::Rejected`]; these causes require rollback instead.
pub enum AttemptWriteError<E> {
    /// The receipt expired or no longer belongs to the live reservation. The
    /// application callback is never invoked for a stale claim.
    Stale,
    /// Native attempt storage failed.
    Storage(sqlx::Error),
    /// The application could not durably decide/write its outcome.
    Application(E),
}

impl<E> fmt::Debug for AttemptWriteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stale => "AttemptWriteError::Stale",
            Self::Storage(_) => "AttemptWriteError::Storage",
            Self::Application(_) => "AttemptWriteError::Application",
        })
    }
}

impl<E> fmt::Display for AttemptWriteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl<E: Error + 'static> Error for AttemptWriteError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Stale => None,
            Self::Storage(error) => Some(error),
            Self::Application(error) => Some(error),
        }
    }
}

/// Every stopped phase is explicit, and uncertain atomic output is retained.
/// No error authorizes retrying credential verification or application writes.
pub enum AttemptError<T, R, V, E> {
    /// Native admission denied; neither application factory ran.
    Denied(AttemptDenial),
    /// Native reservation could not be acknowledged; inspect its certainty.
    Admission(CheckError),
    /// Credential verification failed operationally. The reservation remains
    /// active until native expiry, which conservatively counts abandonment.
    Verification(V),
    /// Cancellation or total budget exhaustion before acknowledged completion.
    Interrupted {
        /// Operational interruption.
        reason: Interruption,
        /// Last operation phase entered.
        phase: AttemptPhase,
    },
    /// Native transaction disposition, including provisional output on uncertainty.
    Atomic(AttemptAtomicError<T, R, E>),
}

/// Exact SQLx atomic outcome used by the runner; known rejection and uncertainty
/// retain their native separate categories.
pub type AttemptAtomicError<T, R, E> =
    PgAtomicError<AttemptCompletionOutput<T, R>, PgScopeError<AttemptWriteError<E>>>;

impl<T, R, V, E> fmt::Debug for AttemptError<T, R, V, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Denied(_) => "AttemptError::Denied",
            Self::Admission(_) => "AttemptError::Admission",
            Self::Verification(_) => "AttemptError::Verification",
            Self::Interrupted { .. } => "AttemptError::Interrupted",
            Self::Atomic(_) => "AttemptError::Atomic",
        })
    }
}

impl<T, R, V, E> fmt::Display for AttemptError<T, R, V, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl<T: 'static, R: 'static, V: Error + 'static, E: Error + 'static> Error
    for AttemptError<T, R, V, E>
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Denied(_) | Self::Interrupted { .. } => None,
            Self::Admission(error) => Some(error),
            Self::Verification(error) => Some(error),
            Self::Atomic(error) => Some(error),
        }
    }
}

/// Canonical PostgreSQL attempt owner, independent of authenticated HTTP quota.
/// Construction is inert. The caller explicitly installs the native attempts
/// migration and supplies a pool with the application tables in the same database.
///
/// The runner owns reserve -> verify -> claim -> decide/write -> finish -> commit.
/// A claimed receipt locks its native row until transaction disposition; expiry
/// after a successful claim does not invalidate that transaction's decision.
///
/// A never-polled run invokes nothing. Cancellation during verification leaves
/// a native lease for conservative recovery; cancellation during commit cannot
/// establish rollback. Acknowledged results are retained before the operation
/// resolver runs, without any intervening await. Dropping the outer future still
/// loses the caller's result; detached work and arbitrary external effects are
/// not supervised or rolled back.
pub struct AttemptRunner {
    limiter: PostgresAttemptLimiter,
    observer: Option<Arc<dyn AttemptObserver>>,
}

impl AttemptRunner {
    /// Use one pool for native admission and the application transaction.
    pub fn new(pool: PgPool) -> Self {
        Self {
            limiter: PostgresAttemptLimiter::new(pool),
            observer: None,
        }
    }

    /// Select native storage budgets/capacity before execution.
    pub fn with_config(mut self, config: PostgresConfig) -> Self {
        self.limiter = self.limiter.with_config(config);
        self
    }

    /// Observe native admission and committed completion without exposing raw
    /// subjects or application data. Completion is emitted only after commit
    /// acknowledgement. Observer panics cannot replace the durable outcome.
    pub fn with_observer(mut self, observer: Arc<dyn AttemptObserver>) -> Self {
        self.limiter = self.limiter.with_observer(observer.clone());
        self.observer = Some(observer);
        self
    }

    /// Admit before expensive verification, then decide and persist atomically.
    ///
    /// `verify` returns a candidate, including ordinary invalid-credential
    /// evidence as appropriate. `decide` performs final replay/status checks and
    /// application writes. Its `Ok(Authentication::Rejected(..))` commits failure
    /// state and audit; its `Err(..)` rolls the whole workflow back. No application
    /// SQL capability exists during verification. Policies, subject normalization,
    /// credential meaning, session and audit SQL remain application decisions.
    pub async fn run<C, T, R, V, E, Verify, Verified>(
        &self,
        context: &OperationContext,
        subject: AttemptSubject<'_>,
        verify: Verify,
        decide: impl AsyncFnOnce(&mut PgScopedSql<'_>, C) -> Result<Authentication<T, R>, E>,
    ) -> Result<AttemptCompletionOutput<T, R>, AttemptError<T, R, V, E>>
    where
        Verify: FnOnce(OperationContext) -> Verified,
        Verified: Future<Output = Result<C, V>>,
    {
        batter_core::telemetry::with_current_dispatch(async move {
            let admitted = context
                .run("attempt.admit", |_| self.limiter.admit(subject))
                .await
                .map_err(|error| match error {
                    OperationError::Failed(error) => AttemptError::Admission(error),
                    OperationError::Interrupted(reason) => AttemptError::Interrupted {
                        reason,
                        phase: AttemptPhase::Admission,
                    },
                })?;
            let receipt = match admitted {
                AttemptAdmission::Admitted(receipt) => receipt,
                AttemptAdmission::Denied(denial) => return Err(AttemptError::Denied(denial)),
            };
            let candidate =
                context
                    .run("attempt.verify", verify)
                    .await
                    .map_err(|error| match error {
                        OperationError::Failed(error) => AttemptError::Verification(error),
                        OperationError::Interrupted(reason) => AttemptError::Interrupted {
                            reason,
                            phase: AttemptPhase::Verification,
                        },
                    })?;
            let result = batter_sqlx::run_atomic_in(
                self.limiter.pool(),
                context,
                "attempt.complete",
                async |scope| {
                    scope
                        .application(async |sql| {
                            let claim = match runlimit_postgres::attempts::low_level::claim_in(
                                sql.executor(),
                                receipt,
                            )
                            .await
                            .map_err(AttemptWriteError::Storage)?
                            {
                                PgAttemptClaimResult::Claimed(claim) => claim,
                                PgAttemptClaimResult::Stale => {
                                    return Err(AttemptWriteError::Stale);
                                }
                            };
                            let authentication = decide(sql, candidate)
                                .await
                                .map_err(AttemptWriteError::Application)?;
                            let completion =
                                match runlimit_postgres::attempts::low_level::finish_in(
                                    sql.executor(),
                                    claim,
                                    authentication.outcome(),
                                )
                                .await
                                .map_err(AttemptWriteError::Storage)?
                                {
                                    StagedAttemptCompletion::Applied(completion) => completion,
                                    StagedAttemptCompletion::Stale => {
                                        return Err(AttemptWriteError::Stale);
                                    }
                                };
                            Ok(AttemptCompletionOutput {
                                authentication,
                                completion,
                            })
                        })
                        .await
                },
            )
            .await
            .map_err(|error| match error {
                OperationError::Failed(error) => AttemptError::Atomic(error),
                OperationError::Interrupted(reason) => AttemptError::Interrupted {
                    reason,
                    phase: AttemptPhase::Completion,
                },
            });
            if let Some(observer) = &self.observer {
                let event = match &result {
                    Ok(output) => Some(AttemptObservation::Completed(output.completion)),
                    Err(AttemptError::Atomic(PgAtomicError::Uncertain(_)))
                    | Err(AttemptError::Interrupted { .. }) => {
                        Some(AttemptObservation::CommitUncertain)
                    }
                    Err(AttemptError::Atomic(PgAtomicError::Rejected(
                        PgScopeError::Application(AttemptWriteError::Stale),
                    ))) => Some(AttemptObservation::Stale),
                    _ => None,
                };
                if let Some(event) = event {
                    observe_attempt_safely(observer.as_ref(), event);
                }
            }
            result
        })
        .await
    }
}

#[cfg(test)]
mod tests;
