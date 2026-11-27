//! Bounded process-local outcome-aware attempts. Dropped receipts expire as failures.
//!
//! Every lease rule is `runlimit_core::attempts::AttemptState`'s. This module
//! owns only storage: the bounded map, receipt generations and the clock clamp.
use crate::{Clock, SystemClock};
use runlimit_core::attempts::{
    AdmitEvaluation, AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptObservation,
    AttemptObserver, AttemptOutcome, AttemptPolicy, AttemptState, AttemptSubject,
    CompleteEvaluation, UnleasedAttempt, observe_attempt_safely,
};
use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use thiserror::Error;

type Key = ([u8; 32], [u8; 32]);
/// One retained subject. A lease's token is the generation of its receipt. A
/// subject that becomes [`AttemptState::Clear`] is removed, not retained.
struct Entry {
    policy: AttemptPolicy,
    state: AttemptState<u64>,
}
#[derive(Default)]
struct State {
    entries: BTreeMap<Key, Entry>,
    generation: u64,
    now: Duration,
    cursor: Option<Key>,
}
impl State {
    /// The evaluation instant for a clock sample. It never decreases.
    fn advance(&mut self, sample: Duration) -> Duration {
        self.now = self.now.max(sample);
        self.now
    }
}

/// A consuming reservation, bound to the creating store and exact policy key.
pub struct MemoryAttemptReceipt {
    store: Weak<Mutex<State>>,
    key: Key,
    generation: u64,
}
impl std::fmt::Debug for MemoryAttemptReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryAttemptReceipt([REDACTED])")
    }
}

/// A bounded attempt store. Admission cleanup examines at most 16 entries.
pub struct MemoryAttemptLimiter<C = SystemClock> {
    state: Arc<Mutex<State>>,
    clock: C,
    capacity: NonZeroUsize,
    observer: Option<Arc<dyn AttemptObserver>>,
}

/// Process-local storage failures; all fail closed without new admission.
#[derive(Debug, Error)]
pub enum MemoryAttemptError {
    /// A previous operation panicked while holding state.
    #[error("attempt state mutex poisoned")]
    Poisoned,
    /// The process exhausted its unique receipt sequence.
    #[error("attempt receipt sequence exhausted")]
    SequenceExhausted,
    /// The clock is within one lease of the end of representable time, so the
    /// lease deadline of an otherwise admitted attempt cannot be held. Only an
    /// injected clock reaches it; denials are still reported there.
    #[error("attempt clock exhausted its representable range")]
    ClockExhausted,
}

/// The only process-local completion failure: poisoned attempt state.
/// Completion consumes an existing receipt and allocates no sequence number.
#[derive(Debug, Error)]
#[error("attempt state mutex poisoned")]
pub struct MemoryAttemptCompletionError;

impl MemoryAttemptLimiter {
    /// Creates a hard-bounded limiter using monotonic system time.
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self::with_clock(capacity, SystemClock::new())
    }
}
impl<C: Clock> MemoryAttemptLimiter<C> {
    /// Creates a limiter with an injected monotonic clock. A sample earlier
    /// than one already seen is read as that one.
    pub fn with_clock(capacity: NonZeroUsize, clock: C) -> Self {
        Self {
            state: Arc::default(),
            clock,
            capacity,
            observer: None,
        }
    }
    /// Installs a panic-isolated observer.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn AttemptObserver>) -> Self {
        self.observer = Some(observer);
        self
    }
    fn observe(&self, observation: AttemptObservation) {
        if let Some(observer) = &self.observer {
            observe_attempt_safely(observer.as_ref(), observation);
        }
    }
    /// Reserves exactly one attempt. Expired reservations count as failures.
    ///
    /// # Errors
    /// Returns an error on poisoned state, an exhausted receipt sequence, or a
    /// clock too close to the end of representable time to hold a lease.
    pub fn admit(
        &self,
        subject: AttemptSubject<'_>,
    ) -> Result<AttemptAdmission<MemoryAttemptReceipt>, MemoryAttemptError> {
        let policy = subject.policy();
        let key = {
            let key = subject.counter_key();
            (key.fingerprint().into_bytes(), key.subject().into_bytes())
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| MemoryAttemptError::Poisoned)?;
        let now = state.advance(self.clock.now());
        cleanup(&mut state, now);
        let full = state.entries.len() >= self.capacity.get();
        let stored = state.entries.get_mut(&key);
        let evaluated = if stored.is_none() && full {
            Err(AttemptDenial::StorageCapacity)
        } else {
            // A subject without an entry is `Clear`.
            let evaluation = match &stored {
                Some(entry) => entry.state.admit(&entry.policy, now),
                None => AttemptState::Clear.admit(policy, now),
            };
            match evaluation {
                AdmitEvaluation::Grant(pending) => Ok(pending),
                AdmitEvaluation::Denied(denied) => {
                    let denial = denied.denial().into();
                    // The denial's own held state is what the entry retains.
                    if let Some(entry) = stored {
                        entry.state = denied.held().into();
                    }
                    Err(denial)
                }
                AdmitEvaluation::DeadlineOverflow(unleased) => {
                    retain(&mut state.entries, &key, unleased);
                    return Err(MemoryAttemptError::ClockExhausted);
                }
            }
        };
        let pending = match evaluated {
            Ok(pending) => pending,
            Err(denial) => {
                drop(state);
                self.observe(AttemptObservation::Denied(denial));
                return Ok(AttemptAdmission::Denied(denial));
            }
        };
        let Some(generation) = state.generation.checked_add(1) else {
            retain(&mut state.entries, &key, pending.replaces());
            return Err(MemoryAttemptError::SequenceExhausted);
        };
        state.generation = generation;
        let leased = AttemptState::Leased(pending.issue(generation));
        state
            .entries
            .entry(key)
            .and_modify(|entry| entry.state = leased)
            .or_insert_with(|| Entry {
                policy: policy.clone(),
                state: leased,
            });
        drop(state);
        self.observe(AttemptObservation::Admitted);
        Ok(AttemptAdmission::Admitted(MemoryAttemptReceipt {
            store: Arc::downgrade(&self.state),
            key,
            generation,
        }))
    }
    /// Completes a reservation once. A stale receipt never applies its outcome.
    /// A receipt presented after its lease expired is stale: the expiry has
    /// already counted as a failure at the lease deadline.
    ///
    /// # Errors
    /// Returns [`MemoryAttemptCompletionError`] when the state mutex is poisoned.
    #[allow(clippy::needless_pass_by_value)] // A receipt is deliberately single-use.
    pub fn complete(
        &self,
        receipt: MemoryAttemptReceipt,
        outcome: AttemptOutcome,
    ) -> Result<AttemptCompletionResult, MemoryAttemptCompletionError> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| MemoryAttemptCompletionError)?;
        let state = &mut *guard;
        let now = state.advance(self.clock.now());
        // A receipt from another store names no reservation here, whatever
        // generation it carries.
        let same_store = Weak::ptr_eq(&receipt.store, &Arc::downgrade(&self.state));
        let result = match state.entries.get_mut(&receipt.key) {
            Some(entry) if same_store => {
                let evaluation =
                    entry
                        .state
                        .complete(&entry.policy, now, &receipt.generation, outcome);
                // The state stored and the result reported both come from this
                // one evaluation.
                let next = match evaluation {
                    CompleteEvaluation::Applied(applied) => applied.released().into(),
                    CompleteEvaluation::Stale { settled } => settled,
                };
                match next {
                    AttemptState::Clear => {
                        state.entries.remove(&receipt.key);
                    }
                    AttemptState::Leased(_) | AttemptState::Failed(_) => entry.state = next,
                }
                match evaluation {
                    CompleteEvaluation::Applied(applied) => {
                        AttemptCompletionResult::Applied(applied.completion())
                    }
                    CompleteEvaluation::Stale { .. } => AttemptCompletionResult::Stale,
                }
            }
            Some(_) | None => AttemptCompletionResult::Stale,
        };
        drop(guard);
        self.observe(match result {
            AttemptCompletionResult::Applied(completion) => {
                AttemptObservation::Completed(completion)
            }
            AttemptCompletionResult::Stale => AttemptObservation::Stale,
        });
        Ok(result)
    }
}
/// Retains the state of a subject whose reservation could not be made, as
/// every other evaluation retains its own. A `Clear` subject is removed.
fn retain(entries: &mut BTreeMap<Key, Entry>, key: &Key, unleased: UnleasedAttempt) {
    match unleased {
        UnleasedAttempt::Clear => {
            entries.remove(key);
        }
        UnleasedAttempt::Failed(failed) => {
            if let Some(entry) = entries.get_mut(key) {
                entry.state = AttemptState::Failed(failed);
            }
        }
    }
}
/// Keys one admission's cleanup examines, and the size of its stack scratch.
const CLEANUP_SCAN: usize = 16;
/// Settles the next keys after the cursor and reclaims those that reach
/// [`AttemptState::Clear`]: no live lease and a full quiet period since the
/// last failure. A retained entry keeps its settled state, so an expired lease
/// is stored as its failure. The cursor rests on the last examined key, or
/// wraps when nothing follows it.
fn cleanup(state: &mut State, now: Duration) {
    use std::ops::Bound::{Excluded, Unbounded};
    let mut scratch = [([0; 32], [0; 32]); CLEANUP_SCAN];
    let mut scanned = 0;
    let start = state.cursor.map_or(Unbounded, Excluded);
    for (slot, (key, _)) in scratch
        .iter_mut()
        .zip(state.entries.range((start, Unbounded)))
    {
        *slot = *key;
        scanned += 1;
    }
    let keys = &scratch[..scanned];
    state.cursor = keys.last().copied();
    for key in keys {
        let Some(entry) = state.entries.get_mut(key) else {
            continue;
        };
        entry.state = entry.state.settle(&entry.policy, now);
        if matches!(entry.state, AttemptState::Clear) {
            state.entries.remove(key);
        }
    }
}

#[cfg(test)]
mod tests;
