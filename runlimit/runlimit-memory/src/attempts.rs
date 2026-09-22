//! Bounded process-local outcome-aware attempts. Dropped receipts expire as failures.
use crate::{Clock, SystemClock};
use runlimit_core::{
    Delay,
    attempts::{
        AttemptAdmission, AttemptCompletion, AttemptCompletionResult, AttemptDenial,
        AttemptObservation, AttemptObserver, AttemptOutcome, AttemptPolicy, AttemptSubject,
        observe_attempt_safely,
    },
};
use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use thiserror::Error;

type Key = ([u8; 32], [u8; 32]);
struct Entry {
    policy: AttemptPolicy,
    failures: u32,
    last_failure: Duration,
    retry_at: Duration,
    lease: Option<(u64, Duration)>,
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
        let policy = subject.policy().clone();
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
        let denial = if let Some(entry) = state.entries.get_mut(&key) {
            reconcile(entry, now);
            deny(entry, now)
        } else if state.entries.len() >= self.capacity.get() {
            Some(AttemptDenial::StorageCapacity)
        } else {
            None
        };
        if let Some(denial) = denial {
            drop(state);
            self.observe(AttemptObservation::Denied(denial));
            return Ok(AttemptAdmission::Denied(denial));
        }
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(MemoryAttemptError::SequenceExhausted)?;
        let generation = state.generation;
        let entry = state.entries.entry(key).or_insert(Entry {
            policy,
            failures: 0,
            last_failure: now,
            retry_at: now,
            lease: None,
        });
        entry.lease = Some((
            generation,
            now.saturating_add(entry.policy.lease().duration()),
        ));
        drop(state);
        self.observe(AttemptObservation::Admitted);
        Ok(AttemptAdmission::Admitted(MemoryAttemptReceipt {
            store: Arc::downgrade(&self.state),
            key,
            generation,
        }))
    }
    /// Completes a reservation once. Stale receipts never mutate current state.
    ///
    /// # Errors
    /// Returns [`MemoryAttemptCompletionError`] when the state mutex is poisoned.
    #[allow(clippy::needless_pass_by_value)] // A receipt is deliberately single-use.
    pub fn complete(
        &self,
        receipt: MemoryAttemptReceipt,
        outcome: AttemptOutcome,
    ) -> Result<AttemptCompletionResult, MemoryAttemptCompletionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MemoryAttemptCompletionError)?;
        state.now = state.now.max(self.clock.now());
        let now = state.now;
        let valid_store = Weak::ptr_eq(&receipt.store, &Arc::downgrade(&self.state));
        let result = match state.entries.get_mut(&receipt.key) {
            Some(entry)
                if valid_store
                    && entry.lease.is_some_and(|(generation, until)| {
                        generation == receipt.generation && now < until
                    }) =>
            {
                entry.lease = None;
                let (failures, delay) = if outcome == AttemptOutcome::Success {
                    (0, Duration::ZERO)
                } else {
                    fail(entry, now);
                    (entry.failures, entry.policy.delay(entry.failures))
                };
                if outcome == AttemptOutcome::Success {
                    state.entries.remove(&receipt.key);
                }
                AttemptCompletionResult::Applied(completion(outcome, failures, delay))
            }
            _ => AttemptCompletionResult::Stale,
        };
        drop(state);
        self.observe(match result {
            AttemptCompletionResult::Applied(completion) => {
                AttemptObservation::Completed(completion)
            }
            AttemptCompletionResult::Stale => AttemptObservation::Stale,
        });
        Ok(result)
    }
}
fn completion(outcome: AttemptOutcome, failures: u32, delay: Duration) -> AttemptCompletion {
    AttemptCompletion::new(outcome, failures, Delay::new(delay))
        .expect("policy transition produces consistent completion metadata")
}
fn fail(entry: &mut Entry, now: Duration) {
    entry.failures = entry.failures.saturating_add(1);
    entry.last_failure = now;
    entry.retry_at = now.saturating_add(entry.policy.delay(entry.failures));
}
fn reconcile(entry: &mut Entry, now: Duration) {
    if let Some((_, until)) = entry.lease
        && now >= until
    {
        entry.lease = None;
        fail(entry, until);
    }
    if entry.lease.is_none()
        && now
            >= entry
                .last_failure
                .saturating_add(entry.policy.quiet_period().duration())
    {
        entry.failures = 0;
        entry.retry_at = now;
    }
}
fn deny(entry: &Entry, now: Duration) -> Option<AttemptDenial> {
    if let Some((_, until)) = entry.lease {
        return Some(AttemptDenial::Busy {
            retry_after: Delay::new(until.saturating_sub(now)),
        });
    }
    (now < entry.retry_at).then(|| AttemptDenial::Backoff {
        retry_after: Delay::new(entry.retry_at.saturating_sub(now)),
    })
}
fn cleanup(state: &mut State, now: Duration) {
    use std::ops::Bound::{Excluded, Unbounded};
    let keys: Vec<Key> = match state.cursor {
        Some(cursor) => state
            .entries
            .range((Excluded(cursor), Unbounded))
            .take(16)
            .map(|(key, _)| *key)
            .collect(),
        None => state.entries.keys().take(16).copied().collect(),
    };
    state.cursor = keys.last().copied();
    for key in keys {
        if state.entries.get(&key).is_some_and(|entry| {
            let origin = entry.lease.map_or(entry.last_failure, |(_, until)| until);
            now >= origin.saturating_add(entry.policy.quiet_period().duration())
        }) {
            state.entries.remove(&key);
        }
    }
}
