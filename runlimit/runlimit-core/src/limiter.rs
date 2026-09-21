use std::{error::Error, future::Future};

use crate::{BatchDecision, Check, Decision, RateLimitPolicy};

/// An asynchronous, backend-independent rate limiter.
///
/// Implementations evaluate one check or an atomic batch using their own
/// authoritative time source. The returned futures are [`Send`], so adapters
/// can await them on a multithreaded executor without Runlimit depending on a
/// particular async runtime.
///
/// Single checks and batches have separate error types. A single check has
/// no batch structure to validate, so its error type never carries a variant
/// such as a duplicate key that only a batch can produce.
///
/// This trait uses return-position `impl Future` for static dispatch without
/// requiring a boxed future. It is intentionally not object-safe. Applications
/// that need runtime backend selection can implement `Limiter` for an
/// application-owned enum and delegate to each variant. The executor
/// portability guarantee also requires limiter and error types to be [`Send`]
/// and [`Sync`], excluding deliberately single-thread-only implementations.
pub trait Limiter: Send + Sync {
    /// Policy algorithm supported by this backend.
    type Policy: RateLimitPolicy;

    /// Backend-specific failure of a single check.
    type CheckError: Error + Send + Sync + 'static;

    /// Backend-specific failure of an atomic batch, including structural
    /// batch validation.
    type CheckAllError: Error + Send + Sync + 'static;

    /// Evaluates and, when allowed, consumes one check.
    ///
    /// Implementations must not evaluate the check or consume quota until the
    /// returned future is first polled.
    fn check(
        &self,
        check: &Check<'_, Self::Policy>,
    ) -> impl Future<Output = Result<Decision, Self::CheckError>> + Send;

    /// Evaluates a nonempty batch atomically.
    ///
    /// If any check is denied, no check consumes quota. An allowed batch
    /// carries one allowance per check in the caller's input order. An empty
    /// batch is rejected with [`crate::BatchError::EmptyBatch`] rather than
    /// vacuously allowed.
    ///
    /// Implementations must not evaluate the checks or consume quota until the
    /// returned future is first polled.
    fn check_all(
        &self,
        checks: &[Check<'_, Self::Policy>],
    ) -> impl Future<Output = Result<BatchDecision, Self::CheckAllError>> + Send;
}
