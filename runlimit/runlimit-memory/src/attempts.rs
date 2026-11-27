//! Bounded process-local outcome-aware attempts. Dropped receipts expire as failures.
//!
//! Every lease rule is `runlimit_core::attempts::AttemptState`'s. This module
//! owns only storage: the bounded map, receipt generations and the clock clamp.
use crate::{Clock, SystemClock};
use runlimit_core::attempts::{
    AdmitEvaluation, AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptObservation,
    AttemptObserver, AttemptOutcome, AttemptPolicy, AttemptState, AttemptSubject,
    CompleteEvaluation, observe_attempt_safely,
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
    /// Creates a limiter with an injected monotonic clock.
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
    /// Returns an error on poisoned state or exhausted receipt sequence.
    pub fn admit(
        &self,
        subject: AttemptSubject<'_>,
    ) -> Result<AttemptAdmission<MemoryAttemptReceipt>, MemoryAttemptError> {
        let policy = subject.policy();
        let key = (
            policy.fingerprint().into_bytes(),
            subject.into_unbound_subject_key().into_bytes(),
        );
        let mut state = self
            .state
            .lock()
            .map_err(|_| MemoryAttemptError::Poisoned)?;
        state.now = state.now.max(self.clock.now());
        let now = state.now;
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
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(MemoryAttemptError::SequenceExhausted)?;
        let generation = state.generation;
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
        state.now = state.now.max(self.clock.now());
        let now = state.now;
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
mod tests {
    use super::{AttemptState, Clock, MemoryAttemptLimiter};
    use runlimit_core::{
        KeyHasher, PolicyId, QuotaPeriod, ScopeId,
        attempts::{
            AttemptAdmission, AttemptCompletionResult, AttemptDenial, AttemptObservation,
            AttemptObserver, AttemptOutcome, AttemptPolicy, FailedAttempts,
        },
    };
    use std::{
        num::{NonZeroU32, NonZeroUsize},
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    #[derive(Clone, Default)]
    struct TestClock(Arc<AtomicU64>);
    impl Clock for TestClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
    }
    #[derive(Default)]
    struct Recorder(Mutex<Vec<AttemptObservation>>);
    impl AttemptObserver for Recorder {
        fn observe(&self, observation: AttemptObservation) {
            self.0.lock().unwrap().push(observation);
        }
    }
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    /// Initial delay 10 ms, cap 40 ms, quiet period 100 ms, lease 20 ms.
    fn policy() -> AttemptPolicy {
        let period = |value| QuotaPeriod::new(ms(value)).unwrap();
        AttemptPolicy::new(
            PolicyId::new("auth").unwrap(),
            ScopeId::new("identifier").unwrap(),
            period(10),
            period(40),
            period(100),
            period(20),
        )
        .unwrap()
    }

    #[test]
    fn cleanup_stores_the_failure_of_an_expired_lease_it_retains() {
        let policy = policy();
        let clock = TestClock::default();
        let limiter =
            MemoryAttemptLimiter::with_clock(NonZeroUsize::new(2).unwrap(), clock.clone());
        let hasher = KeyHasher::new([7; 32]).unwrap();
        let AttemptAdmission::Admitted(_abandoned) = limiter
            .admit(hasher.hash_attempt_for(&policy, b"first"))
            .unwrap()
        else {
            panic!("first admission was denied");
        };
        // Another subject's admission scans the first entry after its 20 ms
        // deadline and inside its quiet period, so it is retained as a failure.
        clock.0.store(25, Ordering::SeqCst);
        let AttemptAdmission::Admitted(_held) = limiter
            .admit(hasher.hash_attempt_for(&policy, b"second"))
            .unwrap()
        else {
            panic!("second admission was denied");
        };
        let state = limiter.state.lock().unwrap();
        let expired = AttemptState::Failed(FailedAttempts::new(NonZeroU32::MIN, ms(20), ms(30)));
        let mut states: Vec<_> = state.entries.values().map(|entry| entry.state).collect();
        states.retain(|state| !matches!(state, AttemptState::Leased(_)));
        assert_eq!(state.entries.len(), 2);
        assert_eq!(states, [expired]);
    }

    #[test]
    fn completing_an_expired_receipt_counts_the_expiry_without_waiting_for_an_admission() {
        let policy = policy();
        let clock = TestClock::default();
        let recorder = Arc::new(Recorder::default());
        let limiter = MemoryAttemptLimiter::with_clock(NonZeroUsize::MIN, clock.clone())
            .with_observer(recorder.clone());
        let subject = KeyHasher::new([7; 32])
            .unwrap()
            .hash_attempt_for(&policy, b"user");
        let stored = |limiter: &MemoryAttemptLimiter<TestClock>| {
            let state = limiter.state.lock().unwrap();
            assert!(state.entries.len() <= 1);
            state.entries.values().next().map(|entry| entry.state)
        };

        let AttemptAdmission::Admitted(expired) = limiter.admit(subject).unwrap() else {
            panic!("first admission was denied");
        };
        clock.0.store(25, Ordering::SeqCst);
        assert_eq!(
            limiter.complete(expired, AttemptOutcome::Success).unwrap(),
            AttemptCompletionResult::Stale
        );
        // The stale completion did not apply its success, but the entry no
        // longer claims a lease: the expiry is the first failure, dated at the
        // 20 ms deadline rather than at the 25 ms it was noticed.
        assert_eq!(
            stored(&limiter),
            Some(AttemptState::Failed(FailedAttempts::new(
                NonZeroU32::MIN,
                ms(20),
                ms(30)
            )))
        );
        let backoff = AttemptDenial::Backoff {
            retry_after: runlimit_core::Delay::new(ms(5)),
        };
        assert!(matches!(
            limiter.admit(subject).unwrap(),
            AttemptAdmission::Denied(denial) if denial == backoff
        ));

        // A stale completion that finds the quiet period over releases the
        // capacity slot at once instead of leaving it to a later cleanup scan.
        clock.0.store(30, Ordering::SeqCst);
        let AttemptAdmission::Admitted(quiet) = limiter.admit(subject).unwrap() else {
            panic!("admission after the delay was denied");
        };
        clock.0.store(150, Ordering::SeqCst);
        assert_eq!(
            limiter.complete(quiet, AttemptOutcome::Failure).unwrap(),
            AttemptCompletionResult::Stale
        );
        assert_eq!(stored(&limiter), None);
        assert_eq!(
            *recorder.0.lock().unwrap(),
            [
                AttemptObservation::Admitted,
                AttemptObservation::Stale,
                AttemptObservation::Denied(backoff),
                AttemptObservation::Admitted,
                AttemptObservation::Stale,
            ]
        );
    }
}
