//! Exact, storage-independent GCRA transition arithmetic for backend authors.
//!
//! Time is whole milliseconds in a backend-owned epoch. The backend must use
//! one clock authority, clamp regressing observations, serialize transitions,
//! and persist an allowance only after every member of an atomic batch passes.

use std::time::Duration;

use thiserror::Error;

use crate::{Allowance, Check, GcraPolicy, QuotaDenial};

/// Exact arithmetic could not represent the supplied backend state or clock.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("GCRA arithmetic exceeded the supported exact range")]
pub struct ArithmeticOverflow;

/// The provisional result of one pure GCRA evaluation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Evaluation {
    /// Admission is possible; persist this transition only after batch preflight.
    Allowed(PendingAllowance),
    /// Admission is unavailable and no state may be consumed.
    Denied(QuotaDenial),
}

/// An evaluated but not yet persisted GCRA transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingAllowance {
    /// New theoretical arrival time, scaled by the policy quota.
    pub tat_scaled: u128,
    /// Whole-millisecond instant at which the counter becomes unnecessary.
    pub expires_at_millis: u128,
    /// Validated decision metadata at the evaluation instant.
    pub allowance: Allowance,
}

/// Evaluates one validated check without changing any state.
///
/// `tat_scaled` is the previously persisted theoretical arrival time for the
/// same configuration fingerprint, or `None` for a missing/expired counter.
///
/// # Errors
///
/// Returns [`ArithmeticOverflow`] for unrepresentable clock-relative state.
pub fn evaluate(
    check: &Check<'_, GcraPolicy>,
    now_millis: u128,
    tat_scaled: Option<u128>,
) -> Result<Evaluation, ArithmeticOverflow> {
    let policy = check.policy();
    let quota = u128::from(policy.quota().get());
    let period = u128::from(policy.period().millis());
    let capacity = policy.burst_capacity();
    let scaled_now = now_millis.checked_mul(quota).ok_or(ArithmeticOverflow)?;
    let active_tat = tat_scaled.unwrap_or(scaled_now).max(scaled_now);
    let increment = u128::from(check.cost())
        .checked_mul(period)
        .ok_or(ArithmeticOverflow)?;
    let candidate = active_tat
        .checked_add(increment)
        .ok_or(ArithmeticOverflow)?;
    let burst_span = u128::from(capacity.get())
        .checked_mul(period)
        .ok_or(ArithmeticOverflow)?;
    let ceiling = scaled_now
        .checked_add(burst_span)
        .ok_or(ArithmeticOverflow)?;

    if candidate > ceiling {
        let retry = (candidate - ceiling).div_ceil(quota);
        return Ok(Evaluation::Denied(QuotaDenial::new(
            capacity,
            duration(retry)?,
        )));
    }

    let available =
        u64::try_from((ceiling - candidate) / period).map_err(|_| ArithmeticOverflow)?;
    let replenish = (candidate - scaled_now).div_ceil(quota);
    Ok(Evaluation::Allowed(PendingAllowance {
        tat_scaled: candidate,
        expires_at_millis: now_millis
            .checked_add(replenish)
            .ok_or(ArithmeticOverflow)?,
        allowance: Allowance::new(capacity, available, duration(replenish)?)
            .map_err(|_| ArithmeticOverflow)?,
    }))
}

fn duration(millis: u128) -> Result<Duration, ArithmeticOverflow> {
    Ok(Duration::from_millis(
        u64::try_from(millis).map_err(|_| ArithmeticOverflow)?,
    ))
}
