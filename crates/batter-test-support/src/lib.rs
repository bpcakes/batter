//! Small test utilities, not an alternate database harness or mocking framework.
//! PostgreSQL provisioning remains owned by postgres-test-harness.
#![forbid(unsafe_code)]

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Both failures remain inspectable when a test and its teardown fail.
#[derive(Debug, thiserror::Error)]
pub enum TestFailure<E, C> {
    /// The body failed; teardown completed.
    #[error("test body failed")]
    Body(#[source] E),
    /// The body succeeded; teardown failed.
    #[error("test cleanup failed")]
    Cleanup(#[source] C),
    /// Neither result is discarded by an early question-mark operator.
    #[error("test body and cleanup both failed")]
    Both {
        /// Primary test error.
        #[source]
        body: E,
        /// Independent teardown error.
        cleanup: C,
    },
}

/// Combine results AFTER driving both body and cleanup to completion.
///
/// This function cannot make teardown run after a panic or cancellation.
/// Capture those at an appropriate task boundary when the test needs it.
pub fn finish<T, E, C>(body: Result<T, E>, cleanup: Result<(), C>) -> Result<T, TestFailure<E, C>> {
    match (body, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(body), Ok(())) => Err(TestFailure::Body(body)),
        (Ok(_), Err(cleanup)) => Err(TestFailure::Cleanup(cleanup)),
        (Err(body), Err(cleanup)) => Err(TestFailure::Both { body, cleanup }),
    }
}

/// Distinguish a scripted dependency error from a broken test script.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError<E> {
    /// The next scripted outcome was an error.
    #[error("scripted dependency failure")]
    Expected(#[source] E),
    /// More calls occurred than the test authorized.
    #[error("script exhausted")]
    Exhausted,
    /// A prior test panic poisoned script state.
    #[error("script state poisoned")]
    Poisoned,
}

struct ScriptState<T, E> {
    outcomes: Mutex<VecDeque<Result<T, E>>>,
    calls: AtomicUsize,
}

/// Deterministic, concurrency-safe scripted results with shared call counts.
///
/// A synchronous primitive: the application fake adds any needed async delay.
/// It never repeats the last result and never fabricates a success.
pub struct Script<T, E> {
    state: Arc<ScriptState<T, E>>,
}

impl<T, E> Clone for Script<T, E> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<T, E> Script<T, E> {
    /// Build the exact sequence of authorized outcomes.
    pub fn new(outcomes: impl IntoIterator<Item = Result<T, E>>) -> Self {
        Self {
            state: Arc::new(ScriptState {
                outcomes: Mutex::new(outcomes.into_iter().collect()),
                calls: AtomicUsize::new(0),
            }),
        }
    }

    /// Consume one outcome. The count includes exhausted/poisoned attempts.
    pub fn next(&self) -> Result<T, ScriptError<E>> {
        self.state.calls.fetch_add(1, Ordering::Relaxed);
        let mut outcomes = self
            .state
            .outcomes
            .lock()
            .map_err(|_| ScriptError::Poisoned)?;
        outcomes
            .pop_front()
            .ok_or(ScriptError::Exhausted)?
            .map_err(ScriptError::Expected)
    }

    /// Total invocations across all clones. Read after joining concurrent work.
    pub fn calls(&self) -> usize {
        self.state.calls.load(Ordering::Relaxed)
    }
}
