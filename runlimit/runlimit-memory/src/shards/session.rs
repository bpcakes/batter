use std::{sync::MutexGuard, time::Instant};

use runlimit_core::{
    AdmissionObservation, Allowance, BatchDecision, CapacityObservation, Check, CleanupObservation,
    CleanupOutcome, ConsumptionStatus, CounterKey, Decision, Denial, Observation, Observer,
    QuotaDenial, QuotaMode, RateLimitPolicy, observe_safely,
};

use super::{BoundedShards, CleanupEffect, Shard, ShardEffect, usize_to_u64};
use crate::{Clock, MemoryBatchError, PoisonedShardError};

mod batch;
#[cfg(test)]
mod tests;

use batch::{BatchAlgorithm, BatchEffects, LockedBatch};

/// Outcome of read-only quota evaluation for one check.
pub(crate) enum QuotaEvaluation<T> {
    Allowed(T),
    Denied(QuotaDenial),
}

/// Single-use, non-`Copy` consumption token produced only by evaluation.
pub(crate) trait PendingToken: Sized {
    type Entry;
    fn counter_key(&self) -> CounterKey;
    fn allowance(&self) -> Allowance;
    fn into_write(self) -> EntryWrite<Self::Entry>;
}

/// A proposed entry change. Only the terminal commit module can apply it.
pub(crate) enum EntryWrite<E> {
    UpdateActive(E),
    Replace { entry: E, expires_at_millis: u128 },
}

/// Algorithm-specific read-only evaluation and single-use consumption.
pub(crate) trait QuotaAlgorithm {
    type Policy: RateLimitPolicy;
    type Entry;
    type Token: PendingToken<Entry = Self::Entry>;
    type EvalError;

    /// Evaluates one check without mutating shard state.
    fn evaluate(
        shard: &Shard<Self::Entry>,
        check: &Check<'_, Self::Policy>,
        now_millis: u128,
    ) -> Result<QuotaEvaluation<Self::Token>, Self::EvalError>;
}

#[derive(Debug)]
pub(crate) enum CheckSessionError<E> {
    PoisonedShard(PoisonedShardError),
    Evaluation(E),
}

#[derive(Debug)]
pub(crate) enum BatchSessionError<E> {
    Store(MemoryBatchError),
    Evaluation(E),
}

impl<E> From<MemoryBatchError> for BatchSessionError<E> {
    fn from(error: MemoryBatchError) -> Self {
        Self::Store(error)
    }
}

/// Runs one check through the consuming shard session and emits observations
/// after the shard lock releases.
pub(crate) fn execute_check<A: QuotaAlgorithm, C: Clock>(
    shards: &BoundedShards<A::Entry>,
    clock: &C,
    observer: Option<&dyn Observer>,
    check: &Check<'_, A::Policy>,
) -> Result<Decision, CheckSessionError<A::EvalError>> {
    let started = observer.map(|_| Instant::now());
    let result = run_single_session::<A, C>(shards, clock, observer.is_some(), check);
    let elapsed = started.map_or_default(|start| start.elapsed());

    if let Some(observer) = observer {
        match &result {
            Ok((decision, effect)) => {
                observe_shard_effect(observer, effect);
                observe_admission(observer, || {
                    AdmissionObservation::from_check(check, decision, elapsed)
                });
            }
            Err(_) => observe_admission(observer, || {
                AdmissionObservation::failed_check(check, ConsumptionStatus::NotConsumed, elapsed)
            }),
        }
    }

    result.map(|(decision, _)| decision)
}

/// Runs one atomic batch through the consuming batch session and emits
/// observations after every shard lock releases.
pub(crate) fn execute_batch<A: BatchAlgorithm, C: Clock>(
    shards: &BoundedShards<A::Entry>,
    clock: &C,
    observer: Option<&dyn Observer>,
    checks: &[Check<'_, A::Policy>],
) -> Result<BatchDecision, BatchSessionError<A::EvalError>> {
    let started = observer.map(|_| Instant::now());
    let result = LockedBatch::<A>::lock(shards, checks, clock)
        .map_err(BatchSessionError::Store)
        .and_then(|locked| locked.run(observer.is_some()));
    let elapsed = started.map_or_default(|start| start.elapsed());

    if let Some(observer) = observer {
        match &result {
            Ok((decision, effects)) => {
                if let BatchEffects::Recorded(effects) = effects {
                    for effect in effects {
                        observe_shard_effect(observer, effect);
                    }
                }
                observe_admission(observer, || {
                    AdmissionObservation::from_batch(checks, decision, elapsed)
                });
            }
            Err(_) => observe_admission(observer, || {
                AdmissionObservation::failed_batch(checks, ConsumptionStatus::NotConsumed, elapsed)
            }),
        }
    }

    result.map(|(decision, _)| decision)
}

fn run_single_session<A: QuotaAlgorithm, C: Clock>(
    shards: &BoundedShards<A::Entry>,
    clock: &C,
    observe: bool,
    check: &Check<'_, A::Policy>,
) -> Result<(Decision, ShardEffect), CheckSessionError<A::EvalError>> {
    let locked =
        LockedShard::<A>::lock(shards, check, clock).map_err(CheckSessionError::PoisonedShard)?;
    let evaluated = locked.evaluate().map_err(CheckSessionError::Evaluation)?;
    let session = evaluated.advance_and_prune(observe);
    Ok(match session.preflight() {
        SinglePreflight::Preflighted(preflighted) => preflighted.commit(),
        SinglePreflight::Denied { denial, effect } => (Decision::denied(denial), effect),
        SinglePreflight::ShadowDenied { denial, effect } => {
            (Decision::shadow_denied(denial), effect)
        }
    })
}

// ---------------------------------------------------------------------------
// Single-shard consuming session:
// LockedShard -> EvaluatedShard -> ShardSession -> PreflightedSingle -> commit
// ---------------------------------------------------------------------------

struct LockedShard<'a, A: QuotaAlgorithm> {
    shard_index: usize,
    guard: MutexGuard<'a, Shard<A::Entry>>,
    check: &'a Check<'a, A::Policy>,
    now_millis: u128,
    cleanup_limit: usize,
}

impl<'a, A: QuotaAlgorithm> LockedShard<'a, A> {
    fn lock<C: Clock>(
        shards: &'a BoundedShards<A::Entry>,
        check: &'a Check<'a, A::Policy>,
        clock: &C,
    ) -> Result<Self, PoisonedShardError> {
        let shard_index = shards.shard_index(&check.counter_key());
        let guard = shards.lock_shard(shard_index)?;
        let observed_millis = clock.now().as_millis();
        let now_millis = observed_millis.max(guard.latest_observed_millis());
        Ok(Self {
            shard_index,
            guard,
            check,
            now_millis,
            cleanup_limit: shards.config().max_expired_removals_per_check(),
        })
    }

    /// Evaluates quota before recording observed time or pruning expired
    /// entries, so a fallible arithmetic preflight leaves the shard untouched.
    fn evaluate(self) -> Result<EvaluatedShard<'a, A>, A::EvalError> {
        let outcome = A::evaluate(&self.guard, self.check, self.now_millis)?;
        Ok(EvaluatedShard {
            locked: self,
            outcome,
        })
    }
}

struct EvaluatedShard<'a, A: QuotaAlgorithm> {
    locked: LockedShard<'a, A>,
    outcome: QuotaEvaluation<A::Token>,
}

impl<'a, A: QuotaAlgorithm> EvaluatedShard<'a, A> {
    fn advance_and_prune(mut self, time_cleanup: bool) -> ShardSession<'a, A> {
        let cleanup = self.locked.guard.advance_and_prune(
            self.locked.now_millis,
            self.locked.cleanup_limit,
            time_cleanup,
        );
        ShardSession {
            locked: self.locked,
            cleanup,
            outcome: self.outcome,
        }
    }
}

struct ShardSession<'a, A: QuotaAlgorithm> {
    locked: LockedShard<'a, A>,
    cleanup: CleanupEffect,
    outcome: QuotaEvaluation<A::Token>,
}

enum SinglePreflight<'a, A: QuotaAlgorithm> {
    Preflighted(PreflightedSingle<'a, A>),
    Denied {
        denial: Denial,
        effect: ShardEffect,
    },
    ShadowDenied {
        denial: QuotaDenial,
        effect: ShardEffect,
    },
}

impl<'a, A: QuotaAlgorithm> ShardSession<'a, A> {
    fn preflight(self) -> SinglePreflight<'a, A> {
        match self.outcome {
            QuotaEvaluation::Denied(denial) => {
                let mode = self.locked.check.policy().quota_mode();
                let effect = self.locked.effect(self.cleanup);
                match mode {
                    QuotaMode::Enforce => SinglePreflight::Denied {
                        denial: denial.into(),
                        effect,
                    },
                    QuotaMode::Shadow => SinglePreflight::ShadowDenied { denial, effect },
                }
            }
            QuotaEvaluation::Allowed(token) => {
                let guard = &self.locked.guard;
                if !guard.contains_key(token.counter_key())
                    && guard.used_entries() >= guard.capacity()
                {
                    let denial = guard.storage_capacity_denial(self.locked.now_millis);
                    return SinglePreflight::Denied {
                        denial,
                        effect: self.locked.effect(self.cleanup),
                    };
                }
                SinglePreflight::Preflighted(PreflightedSingle {
                    locked: self.locked,
                    cleanup: self.cleanup,
                    token,
                })
            }
        }
    }
}

struct PreflightedSingle<'a, A: QuotaAlgorithm> {
    locked: LockedShard<'a, A>,
    cleanup: CleanupEffect,
    token: A::Token,
}

impl<A: QuotaAlgorithm> LockedShard<'_, A> {
    fn effect(self, cleanup: CleanupEffect) -> ShardEffect {
        ShardEffect {
            shard_index: self.shard_index,
            cleanup,
            used: self.guard.used_entries(),
            capacity: self.guard.capacity(),
        }
    }
}

fn observe_shard_effect(observer: &dyn Observer, effect: &ShardEffect) {
    observe_safely(
        observer,
        &Observation::Cleanup(CleanupObservation::new(
            effect.cleanup.requested,
            CleanupOutcome::Confirmed {
                removed: usize_to_u64(effect.cleanup.removed),
            },
            effect.cleanup.elapsed,
        )),
    );
    observe_safely(
        observer,
        &Observation::Capacity(CapacityObservation::new(
            usize_to_u64(effect.used),
            usize_to_u64(effect.capacity),
            effect.shard_index,
        )),
    );
}

fn observe_admission<'a>(
    observer: &dyn Observer,
    build_observation: impl FnOnce() -> AdmissionObservation<'a>,
) {
    let admission = build_observation();
    observe_safely(observer, &Observation::Admission(admission));
}
