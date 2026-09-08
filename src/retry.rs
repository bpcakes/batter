//! Explicit replay policy and budget-aware exponential backoff.
//!
//! There is no automatic retry for timeouts, cancellation, or panics. There is
//! no default assertion that a side effect is replay-safe. Use `execute` for
//! deterministic backoff or `execute_with_jitter` with a per-execution sampler.

use crate::{
    ConfigurationError,
    operation::{Interruption, OperationContext, OperationError},
    validation,
};
use std::{convert::Infallible, future::Future, time::Duration};

/// A caller assertion about replay semantics, not a library proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaySafety {
    /// Never start a second attempt.
    Never,
    /// Repetition is intrinsically safe for this operation.
    Idempotent,
    /// A verified external protocol deduplicates repeated effects.
    Deduplicated,
}

/// Application-owned classification of a returned error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Return the error without another attempt.
    Stop,
    /// Use the policy's exponential delay.
    Retry,
    /// Provider delay is a LOWER bound, combined with backoff using max.
    RetryAfter(Duration),
}

/// Validated attempt count and deterministic exponential delay.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    max_attempts: u32,
    initial: Duration,
    maximum: Duration,
}

impl RetryPolicy {
    /// `max_attempts` includes the initial call. All delays must be positive.
    pub fn new(
        max_attempts: u32,
        initial: Duration,
        maximum: Duration,
    ) -> Result<Self, ConfigurationError> {
        if max_attempts == 0 {
            return Err(ConfigurationError::Zero("max attempts"));
        }
        validation::positive(initial, "initial backoff")?;
        validation::positive(maximum, "maximum backoff")?;
        if maximum < initial {
            return Err(ConfigurationError::InvalidBackoff);
        }
        Ok(Self {
            max_attempts,
            initial,
            maximum,
        })
    }

    /// Total permitted attempts, not additional retries.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    fn delay(&self, failed_attempt: u32) -> Duration {
        let mut delay = self.initial;
        // Duration has a finite representation; saturation ends this loop
        // quickly even with a large, externally configured attempt limit.
        for _ in 1..failed_attempt {
            delay = delay.saturating_mul(2).min(self.maximum);
            if delay == self.maximum {
                break;
            }
        }
        delay
    }
}

/// Why an application error was returned instead of retried again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The classifier rejected a retry.
    NotRetryable,
    /// Replay was not permitted by the caller.
    ReplayForbidden,
    /// The configured total attempt limit was reached.
    AttemptsExhausted,
    /// The required delay leaves no budget to begin another attempt.
    InsufficientBudget,
}

/// Retry failure preserves the latest application error when one exists.
#[derive(Debug, thiserror::Error)]
pub enum RetryError<E> {
    /// Terminal application failure.
    #[error("retry execution stopped after {attempts} attempt(s): {reason:?}")]
    Stopped {
        /// Number of factories started.
        attempts: u32,
        /// Policy reason, distinct from the application's cause.
        reason: StopReason,
        /// Latest returned application failure.
        #[source]
        error: E,
    },
    /// Interruption is not classified as an automatically replayable error.
    #[error("retry execution interrupted after {attempts} attempt(s): {reason}")]
    Interrupted {
        /// Number of factories started; may be zero.
        attempts: u32,
        /// Cancellation or total deadline.
        reason: Interruption,
        /// Last returned failure, retained even when interrupted in backoff.
        #[source]
        last_error: Option<E>,
    },
}

/// Per-attempt information, including a child of the original total context.
#[derive(Clone, Debug)]
pub struct Attempt {
    /// One-based attempt number.
    pub number: u32,
    /// Cannot extend the original deadline. Cancelled when this attempt ends.
    pub context: OperationContext,
}

/// Execute a factory with explicit replay authorization and error classification.
///
/// Each attempt gets a fresh future. There is one total deadline, including
/// backoff. Provider delay is never shortened. A timeout or cancellation stops
/// the entire sequence. The callback must not block or create detached work.
/// Choose ONE retry owner across service/client/job layers.
pub async fn execute<T, E, F, Fut, C>(
    context: &OperationContext,
    operation: &'static str,
    safety: ReplaySafety,
    policy: &RetryPolicy,
    factory: F,
    classify: C,
) -> Result<T, RetryError<E>>
where
    F: FnMut(Attempt) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: FnMut(&E) -> RetryDecision,
{
    execute_with_delay(
        context,
        operation,
        safety,
        policy,
        |delay| delay,
        factory,
        classify,
    )
    .await
}

/// Execute with equal jitter from an explicitly supplied random sample stream.
///
/// Each candidate retry samples once. Its exponential backoff is varied in
/// `[max(1 ns, backoff / 2), backoff]`, so the delay remains positive and never
/// exceeds the policy cap. A provider's RetryAfter is applied afterward as a
/// lower bound and can exceed the policy cap. All waits use the original total
/// context, including a shortened work context with a finalization reserve.
///
/// Supply uniformly distributed `u64` values from an independently seeded
/// generator for each production execution; reusing a fixed seed across a
/// fleet recreates synchronized retries. Tests can inject reproducible streams
/// or endpoint samples. The sampler must not block; it is invoked only after
/// replay and classification authorize another attempt. This function does not
/// install a global RNG, alter replay policy, or retry interrupted attempts.
pub async fn execute_with_jitter<T, E, F, Fut, C, S>(
    context: &OperationContext,
    operation: &'static str,
    safety: ReplaySafety,
    policy: &RetryPolicy,
    mut sample: S,
    factory: F,
    classify: C,
) -> Result<T, RetryError<E>>
where
    F: FnMut(Attempt) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: FnMut(&E) -> RetryDecision,
    S: FnMut() -> u64,
{
    execute_with_delay(
        context,
        operation,
        safety,
        policy,
        |delay| equal_jitter(delay, sample()),
        factory,
        classify,
    )
    .await
}

fn equal_jitter(upper: Duration, sample: u64) -> Duration {
    let lower = (upper / 2).max(Duration::from_nanos(1));
    let width = (upper - lower).as_nanos() + 1;
    // Validated backoffs are at most one year, so this product fits u128 and
    // its scaled nanosecond offset fits u64. Use the full sample range.
    let offset = (u128::from(sample) * width) >> 64;
    lower + Duration::from_nanos(offset as u64)
}

async fn execute_with_delay<T, E, F, Fut, C, D>(
    context: &OperationContext,
    operation: &'static str,
    safety: ReplaySafety,
    policy: &RetryPolicy,
    mut delay_for: D,
    mut factory: F,
    mut classify: C,
) -> Result<T, RetryError<E>>
where
    F: FnMut(Attempt) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: FnMut(&E) -> RetryDecision,
    D: FnMut(Duration) -> Duration,
{
    let mut attempts = 0;
    let mut last_error = None;
    loop {
        if let Err(reason) = context.check() {
            return Err(RetryError::Interrupted {
                attempts,
                reason,
                last_error,
            });
        }
        // Count inside the factory, not before run's cancellation preflight.
        let result = context
            .run(operation, |scope| {
                attempts += 1;
                tracing::debug!(target: "batter", attempt = attempts, "attempt started");
                factory(Attempt {
                    number: attempts,
                    context: scope,
                })
            })
            .await;
        let error = match result {
            Ok(value) => return Ok(value),
            Err(OperationError::Interrupted(reason)) => {
                return Err(RetryError::Interrupted {
                    attempts,
                    reason,
                    last_error,
                });
            }
            Err(OperationError::Failed(error)) => error,
        };
        if safety == ReplaySafety::Never {
            return Err(RetryError::Stopped {
                attempts,
                reason: StopReason::ReplayForbidden,
                error,
            });
        }
        let decision = classify(&error);
        if decision == RetryDecision::Stop {
            return Err(RetryError::Stopped {
                attempts,
                reason: StopReason::NotRetryable,
                error,
            });
        }
        if attempts >= policy.max_attempts {
            return Err(RetryError::Stopped {
                attempts,
                reason: StopReason::AttemptsExhausted,
                error,
            });
        }
        if let Err(reason) = context.check() {
            return Err(RetryError::Interrupted {
                attempts,
                reason,
                last_error: Some(error),
            });
        }
        let backoff = delay_for(policy.delay(attempts));
        let delay = match decision {
            RetryDecision::RetryAfter(provider) => backoff.max(provider),
            _ => backoff,
        };
        if delay >= context.remaining() {
            return Err(RetryError::Stopped {
                attempts,
                reason: StopReason::InsufficientBudget,
                error,
            });
        }
        last_error = Some(error);
        tracing::debug!(target: "batter", attempt = attempts, delay_ms = delay.as_secs_f64() * 1_000.0, "retry scheduled");
        match context
            .run("batter.retry.backoff", |_| async {
                tokio::time::sleep(delay).await;
                Ok::<(), Infallible>(())
            })
            .await
        {
            Ok(()) => {}
            Err(OperationError::Interrupted(reason)) => {
                return Err(RetryError::Interrupted {
                    attempts,
                    reason,
                    last_error,
                });
            }
            Err(OperationError::Failed(never)) => match never {},
        }
    }
}
