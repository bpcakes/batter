//! Explicit replay policy and budget-aware exponential backoff.
//!
//! There is no automatic retry for timeouts, cancellation, or panics. There is
//! no default assertion that a side effect is replay-safe. Use [`execute`] for
//! deterministic backoff or [`execute_with_jitter`] with a per-execution
//! sampler. The options entrypoint can additionally cap each attempt without
//! extending the original total context.

use crate::{
    ConfigurationError,
    operation::{Interruption, OperationContext, OperationError},
    telemetry::Outcome,
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

/// Optional policies for the extensible retry execution boundary.
///
/// The default has no per-attempt cap and does not alter the total context.
/// Configure an attempt maximum before calling [`execute_with_options`]. Use
/// [`RetryOptions::with_jitter`] when that execution also needs equal jitter.
///
/// Options are consumed by one execution and intentionally are not clonable.
/// Construct a fresh value, with an independently seeded sampler, for every
/// concurrent execution so cloning sampler state cannot synchronize retries.
///
/// ```
/// use batter_core::retry::RetryOptions;
///
/// let mut state = 0_u64;
/// let options = RetryOptions::new().with_jitter(move || {
///     state += 1;
///     state
/// });
/// let _ = options;
/// ```
///
/// ```compile_fail
/// use batter_core::retry::RetryOptions;
///
/// let mut state = 0_u64;
/// let options = RetryOptions::new().with_jitter(move || {
///     state += 1;
///     state
/// });
/// let duplicate = options.clone();
/// # let _ = duplicate;
/// ```
#[derive(Debug)]
pub struct RetryOptions<S = fn() -> u64> {
    attempt_maximum: Option<Duration>,
    sample: Option<S>,
}

impl RetryOptions {
    /// Create options that add no policy to the original retry behavior.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<S> RetryOptions<S> {
    /// Cap every started attempt without renewing the total context.
    ///
    /// The maximum must be positive, no greater than 365 days, and
    /// representable by the runtime clock. The actual attempt deadline is the
    /// earlier of this maximum from the attempt's start and the deadline of the
    /// context passed to execution.
    pub fn with_attempt_maximum(mut self, maximum: Duration) -> Result<Self, ConfigurationError> {
        self.attempt_maximum = Some(validation::positive(maximum, "retry attempt maximum")?);
        Ok(self)
    }

    /// The configured cap for each attempt, if one was selected.
    pub fn attempt_maximum(&self) -> Option<Duration> {
        self.attempt_maximum
    }

    /// Add equal jitter from an explicitly supplied per-execution sample stream.
    ///
    /// Sampling follows [`execute_with_jitter`]: it happens once after replay
    /// and classification authorize another attempt, before applying any
    /// provider delay floor. The sampler must not block. Construct and seed it
    /// independently for each execution; do not copy another execution's state.
    pub fn with_jitter<T>(self, sample: T) -> RetryOptions<T>
    where
        T: FnMut() -> u64,
    {
        RetryOptions {
            attempt_maximum: self.attempt_maximum,
            sample: Some(sample),
        }
    }
}

impl Default for RetryOptions {
    fn default() -> Self {
        Self {
            attempt_maximum: None,
            sample: None,
        }
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
        /// Cancellation observed in the total or current attempt scope, or the
        /// total deadline.
        reason: Interruption,
        /// Last returned failure, retained even when interrupted in backoff.
        #[source]
        last_error: Option<E>,
    },
}

/// Failure from the extensible retry execution boundary.
///
/// Unlike [`RetryError`], this type can report a per-attempt deadline without
/// changing the legacy interruption behavior. The enum is non-exhaustive so
/// additional opt-in policies can add typed terminal outcomes without breaking
/// consumers.
///
/// ```
/// use batter_core::retry::RetryExecutionError;
///
/// fn inspect(error: RetryExecutionError<&'static str>) {
///     match error {
///         RetryExecutionError::Stopped { .. }
///         | RetryExecutionError::Interrupted { .. }
///         | RetryExecutionError::AttemptDeadlineExceeded { .. } => {}
///         _ => {}
///     }
/// }
/// ```
///
/// ```compile_fail
/// use batter_core::retry::RetryExecutionError;
///
/// fn inspect(error: RetryExecutionError<&'static str>) {
///     match error {
///         RetryExecutionError::Stopped { .. }
///         | RetryExecutionError::Interrupted { .. }
///         | RetryExecutionError::AttemptDeadlineExceeded { .. } => {}
///     }
/// }
/// ```
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RetryExecutionError<E> {
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
    /// Cancellation observed in the total or current attempt scope, or
    /// expiration of the original total context.
    #[error("retry execution interrupted after {attempts} attempt(s): {reason}")]
    Interrupted {
        /// Number of factories started; may be zero.
        attempts: u32,
        /// Cancellation observed in the total or current attempt scope, or the
        /// total deadline.
        reason: Interruption,
        /// Last returned failure, retained even when interrupted in backoff.
        #[source]
        last_error: Option<E>,
    },
    /// A configured per-attempt deadline expired while it was the tighter cap.
    ///
    /// This compares absolute deadlines. Runtime scheduling can delay
    /// observation until the original total deadline has also elapsed, so this
    /// outcome does not promise that total budget remains when it is returned.
    #[error("retry attempt deadline exceeded after {attempts} attempt(s)")]
    AttemptDeadlineExceeded {
        /// Number of factories started; may be zero if the cap expires before
        /// a very short first attempt can invoke its factory.
        attempts: u32,
        /// Last failure returned by an earlier attempt, when one exists.
        #[source]
        last_error: Option<E>,
    },
}

impl<E> RetryExecutionError<E> {
    fn into_legacy(self) -> RetryError<E> {
        match self {
            Self::Stopped {
                attempts,
                reason,
                error,
            } => RetryError::Stopped {
                attempts,
                reason,
                error,
            },
            Self::Interrupted {
                attempts,
                reason,
                last_error,
            } => RetryError::Interrupted {
                attempts,
                reason,
                last_error,
            },
            Self::AttemptDeadlineExceeded { .. } => {
                unreachable!("legacy execution does not configure an attempt deadline")
            }
        }
    }
}

/// Per-attempt information, including a child of the execution context.
///
/// An opt-in attempt maximum can further shorten that child's deadline.
#[derive(Clone, Debug)]
pub struct Attempt {
    /// One-based attempt number.
    pub number: u32,
    /// Cannot extend the original deadline. Cancelled when this attempt ends.
    /// The callback observes cancellation here; it cannot request cancellation
    /// through this context. A controller must retain owner authority separately.
    pub context: OperationContext,
}

/// Execute a factory with explicit replay authorization and error classification.
///
/// Each attempt gets a fresh future. There is one total deadline, including
/// backoff. Provider delay is never shortened. A timeout or cancellation stops
/// the entire sequence. Cancellation observed in the input lineage or current
/// [`Attempt`] scope before result reconciliation returns
/// [`RetryError::Interrupted`], even when the factory returned `Ok` in the same
/// poll. The callback must not block or create detached work. Choose ONE retry
/// owner across service/client/job layers.
///
/// A dedicated child owner lets a controller stop this sequence without
/// cancelling its caller or sibling operations. The `Attempt` argument supplies
/// only an observation and execution context; the closure explicitly retains
/// the child owner:
///
/// ```
/// use batter_core::{
///     operation::{Interruption, OperationOwner},
///     retry::{self, ReplaySafety, RetryDecision, RetryError, RetryPolicy},
/// };
/// use std::time::Duration;
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let caller = OperationOwner::new(Duration::from_secs(5))?;
/// let retry_owner = caller.context().child(Duration::from_secs(2))?;
/// let sibling = caller.context().child(Duration::from_secs(2))?;
/// let policy = RetryPolicy::new(1, Duration::from_millis(1), Duration::from_millis(1))?;
/// let result = retry::execute(
///     retry_owner.context(),
///     "provider.read",
///     ReplaySafety::Idempotent,
///     &policy,
///     |_| {
///         retry_owner.cancel();
///         async { Ok::<u8, std::io::Error>(7) }
///     },
///     |_| RetryDecision::Stop,
/// ).await;
/// assert!(matches!(result, Err(RetryError::Interrupted {
///     reason: Interruption::Cancelled, ..
/// })));
/// assert!(caller.context().check().is_ok());
/// assert!(sibling.context().check().is_ok());
/// # Ok(())
/// # }
/// ```
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
        ExecutionSettings::new(context, operation, safety, policy, None),
        |delay| delay,
        factory,
        classify,
    )
    .await
    .map_err(RetryExecutionError::into_legacy)
}

/// Execute with opt-in policies and one total retry context.
///
/// A configured attempt maximum is measured freshly immediately before every
/// factory. It is capped by the deadline of `context`, including when `context`
/// is already a shortened work phase. Expiration of the tighter attempt cap is
/// terminal and returns [`RetryExecutionError::AttemptDeadlineExceeded`]; it is
/// not classified or retried. Cancellation observed in the input lineage or
/// current [`Attempt`] scope and expiration of the original total deadline
/// remain distinct [`RetryExecutionError::Interrupted`] outcomes. A caller that
/// must stop the sequence retains ownership of the input context or its parent.
///
/// Attempt interruption drops the owned future and cancels its child scope. It
/// does not establish a remote effect's outcome or join detached work.
/// Factory construction must not block, and a non-yielding future poll cannot
/// be preempted. If either overruns a deadline, its late result can be accepted
/// when the completion branch becomes ready before the timer branch is observed;
/// this is not a hard wall-clock cap. Cancellation of the current
/// [`Attempt::context`] is reconciled before a returned result is classified.
/// Attempt-versus-total classification compares their absolute deadlines; a
/// stalled runtime can observe the earlier cap only after total time has also
/// elapsed, so callers must check `context` before starting follow-up work.
/// Backoff is deterministic unless `options` was configured with
/// [`RetryOptions::with_jitter`].
pub async fn execute_with_options<T, E, F, Fut, C, S>(
    context: &OperationContext,
    operation: &'static str,
    safety: ReplaySafety,
    policy: &RetryPolicy,
    options: RetryOptions<S>,
    factory: F,
    classify: C,
) -> Result<T, RetryExecutionError<E>>
where
    F: FnMut(Attempt) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: FnMut(&E) -> RetryDecision,
    S: FnMut() -> u64,
{
    let RetryOptions {
        attempt_maximum,
        mut sample,
    } = options;
    execute_with_delay(
        ExecutionSettings::new(context, operation, safety, policy, attempt_maximum),
        |delay| match &mut sample {
            Some(sample) => equal_jitter(delay, sample()),
            None => delay,
        },
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
/// install a global RNG, alter replay policy, or retry interrupted attempts. As
/// with [`execute`], cancellation observed in the input lineage or current
/// [`Attempt`] scope before result reconciliation returns
/// [`RetryError::Interrupted`], even when the factory returned `Ok` in the same
/// poll.
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
        ExecutionSettings::new(context, operation, safety, policy, None),
        |delay| equal_jitter(delay, sample()),
        factory,
        classify,
    )
    .await
    .map_err(RetryExecutionError::into_legacy)
}

fn equal_jitter(upper: Duration, sample: u64) -> Duration {
    let lower = (upper / 2).max(Duration::from_nanos(1));
    let width = (upper - lower).as_nanos() + 1;
    // Validated backoffs are at most one year, so this product fits u128 and
    // its scaled nanosecond offset fits u64. Use the full sample range.
    let offset = (u128::from(sample) * width) >> 64;
    lower + Duration::from_nanos(offset as u64)
}

struct ExecutionSettings<'a> {
    context: &'a OperationContext,
    operation: &'static str,
    safety: ReplaySafety,
    policy: &'a RetryPolicy,
    attempt_maximum: Option<Duration>,
}

impl<'a> ExecutionSettings<'a> {
    fn new(
        context: &'a OperationContext,
        operation: &'static str,
        safety: ReplaySafety,
        policy: &'a RetryPolicy,
        attempt_maximum: Option<Duration>,
    ) -> Self {
        Self {
            context,
            operation,
            safety,
            policy,
            attempt_maximum,
        }
    }
}

enum AttemptFailure<E> {
    Application(E),
    Interrupted {
        reason: Interruption,
        returned_error: Option<E>,
    },
    DeadlineExceeded,
}

enum AttemptCompletion<T, E> {
    Returned(Result<T, E>),
    Cancelled(Option<E>),
}

fn attempt_outcome<T, E>(
    result: &Result<AttemptCompletion<T, E>, OperationError<Infallible>>,
) -> Outcome {
    match result {
        Ok(AttemptCompletion::Returned(Ok(_))) => Outcome::Succeeded,
        Ok(AttemptCompletion::Returned(Err(_))) => Outcome::Failed,
        Ok(AttemptCompletion::Cancelled(_)) => Outcome::Cancelled,
        Err(OperationError::Interrupted(Interruption::Cancelled)) => Outcome::Cancelled,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
            Outcome::DeadlineExceeded
        }
        Err(OperationError::Failed(never)) => match *never {},
    }
}

async fn run_attempt<T, E, F, Fut>(
    settings: &ExecutionSettings<'_>,
    attempts: &mut u32,
    factory: &mut F,
) -> Result<T, AttemptFailure<E>>
where
    F: FnMut(Attempt) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let attempt_context = settings
        .attempt_maximum
        .map(|maximum| settings.context.scoped_child_with_maximum(maximum));
    let attempt_context = attempt_context.as_ref().unwrap_or(settings.context);
    let attempt_deadline_is_tighter = attempt_context.deadline() < settings.context.deadline();
    // Count inside the factory, not before run's cancellation preflight.
    match attempt_context
        .run_with_outcome(
            settings.operation,
            |scope| {
                *attempts += 1;
                tracing::debug!(target: "batter", attempt = *attempts, "attempt started");
                let completion_scope = scope.clone();
                let future = factory(Attempt {
                    number: *attempts,
                    context: scope,
                });
                async move {
                    let result = future.await;
                    let completion =
                        if matches!(completion_scope.check(), Err(Interruption::Cancelled)) {
                            AttemptCompletion::Cancelled(result.err())
                        } else {
                            AttemptCompletion::Returned(result)
                        };
                    Ok::<_, Infallible>(completion)
                }
            },
            attempt_outcome,
        )
        .await
    {
        Ok(AttemptCompletion::Returned(Ok(value))) => Ok(value),
        Ok(AttemptCompletion::Returned(Err(error))) => Err(AttemptFailure::Application(error)),
        Ok(AttemptCompletion::Cancelled(returned_error)) => Err(AttemptFailure::Interrupted {
            reason: Interruption::Cancelled,
            returned_error,
        }),
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
            if attempt_deadline_is_tighter =>
        {
            Err(AttemptFailure::DeadlineExceeded)
        }
        Err(OperationError::Interrupted(reason)) => Err(AttemptFailure::Interrupted {
            reason,
            returned_error: None,
        }),
        Err(OperationError::Failed(never)) => match never {},
    }
}

async fn execute_with_delay<T, E, F, Fut, C, D>(
    settings: ExecutionSettings<'_>,
    mut delay_for: D,
    mut factory: F,
    mut classify: C,
) -> Result<T, RetryExecutionError<E>>
where
    F: FnMut(Attempt) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: FnMut(&E) -> RetryDecision,
    D: FnMut(Duration) -> Duration,
{
    let mut attempts = 0;
    let mut last_error = None;
    loop {
        if let Err(reason) = settings.context.check() {
            return Err(RetryExecutionError::Interrupted {
                attempts,
                reason,
                last_error,
            });
        }
        let error = match run_attempt(&settings, &mut attempts, &mut factory).await {
            Ok(value) => return Ok(value),
            Err(AttemptFailure::DeadlineExceeded) => {
                return Err(RetryExecutionError::AttemptDeadlineExceeded {
                    attempts,
                    last_error,
                });
            }
            Err(AttemptFailure::Interrupted {
                reason,
                returned_error,
            }) => {
                return Err(RetryExecutionError::Interrupted {
                    attempts,
                    reason,
                    last_error: returned_error.or(last_error),
                });
            }
            Err(AttemptFailure::Application(error)) => error,
        };
        if settings.safety == ReplaySafety::Never {
            return Err(RetryExecutionError::Stopped {
                attempts,
                reason: StopReason::ReplayForbidden,
                error,
            });
        }
        let decision = classify(&error);
        if decision == RetryDecision::Stop {
            return Err(RetryExecutionError::Stopped {
                attempts,
                reason: StopReason::NotRetryable,
                error,
            });
        }
        if attempts >= settings.policy.max_attempts {
            return Err(RetryExecutionError::Stopped {
                attempts,
                reason: StopReason::AttemptsExhausted,
                error,
            });
        }
        if let Err(reason) = settings.context.check() {
            return Err(RetryExecutionError::Interrupted {
                attempts,
                reason,
                last_error: Some(error),
            });
        }
        let backoff = delay_for(settings.policy.delay(attempts));
        let delay = match decision {
            RetryDecision::RetryAfter(provider) => backoff.max(provider),
            _ => backoff,
        };
        if delay >= settings.context.remaining() {
            return Err(RetryExecutionError::Stopped {
                attempts,
                reason: StopReason::InsufficientBudget,
                error,
            });
        }
        last_error = Some(error);
        tracing::debug!(target: "batter", attempt = attempts, delay_ms = delay.as_secs_f64() * 1_000.0, "retry scheduled");
        match settings
            .context
            .run("batter.retry.backoff", |_| async {
                tokio::time::sleep(delay).await;
                Ok::<(), Infallible>(())
            })
            .await
        {
            Ok(()) => {}
            Err(OperationError::Interrupted(reason)) => {
                return Err(RetryExecutionError::Interrupted {
                    attempts,
                    reason,
                    last_error,
                });
            }
            Err(OperationError::Failed(never)) => match never {},
        }
    }
}
