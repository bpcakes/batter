use super::*;
use serde_json::json;
use uuid::Uuid;

#[test]
fn terminal_projection_covers_every_retained_state_and_native_budget_pair() {
    use crate::delivery::{DeliveryState as Job, ProviderEffectState as Effect};

    // Explicit oracle: native termination cannot overwrite terminal provider
    // truth, and exhaustion requires both dead-lettering and a spent budget.
    for (job, attempts_spent, unresolved) in [
        (Job::Pending, false, None),
        (Job::Pending, true, None),
        (Job::InFlight, false, None),
        (Job::InFlight, true, None),
        (Job::Succeeded, false, Some(Effect::ManualResolution)),
        (Job::Succeeded, true, Some(Effect::ManualResolution)),
        (Job::Cancelled, false, Some(Effect::ManualResolution)),
        (Job::Cancelled, true, Some(Effect::ManualResolution)),
        (Job::DeadLettered, false, Some(Effect::ManualResolution)),
        (Job::DeadLettered, true, Some(Effect::Exhausted)),
    ] {
        for retained in [
            Effect::AwaitingAttempt,
            Effect::RetryableUndispatched,
            Effect::ReconcileNeeded,
        ] {
            assert_eq!(
                retained.project(job, attempts_spent),
                unresolved.unwrap_or(retained),
                "retained={retained:?}, job={job:?}, attempts_spent={attempts_spent}"
            );
        }
        for retained in [
            Effect::Confirmed,
            Effect::BusinessDenied,
            Effect::ManualResolution,
            Effect::Exhausted,
        ] {
            assert_eq!(
                retained.project(job, attempts_spent),
                retained,
                "retained={retained:?}, job={job:?}, attempts_spent={attempts_spent}"
            );
        }
    }
}

fn snapshot(
    state: ProviderEffectState,
    retention_active: bool,
    current_generation: Option<i64>,
) -> EffectSnapshot {
    let provider_payload = ProviderEffectRequest {
        version: crate::provider::EFFECT_PROTOCOL_VERSION,
        effect_id: Uuid::from_u128(1),
        owner_id: Uuid::from_u128(2),
        record_id: Uuid::from_u128(3),
        record_generation: 4,
        payload: json!({"channel": "planner"}),
    };
    EffectSnapshot {
        provider_key: provider_payload.idempotency_key(),
        provider_payload,
        state,
        retention_active,
        retry_delay: Duration::ZERO,
        current_generation,
    }
}

#[test]
fn durable_retry_delay_precedes_dispatch_but_not_terminal_or_generation_decisions() {
    for state in [
        ProviderEffectState::AwaitingAttempt,
        ProviderEffectState::RetryableUndispatched,
    ] {
        let mut retained = snapshot(state, false, Some(4));
        retained.retry_delay = Duration::from_secs(60);
        assert_eq!(retained.plan(4), EffectPlan::Defer(Duration::from_secs(60)));
        retained.current_generation = None;
        assert_eq!(
            retained.plan(4),
            EffectPlan::DenyGeneration { expected: state }
        );
        retained.state = ProviderEffectState::Confirmed;
        assert_eq!(retained.plan(4), EffectPlan::Complete);
    }
    assert_eq!(remaining_retry_delay(-1), Duration::ZERO);
    assert_eq!(remaining_retry_delay(0), Duration::ZERO);
    assert_eq!(remaining_retry_delay(1), Duration::from_millis(1));
}

#[test]
fn retained_state_exhaustively_selects_work_before_provider_admission() {
    let generation = 4;
    for (state, retention_active, current_generation, expected) in [
        (
            ProviderEffectState::Confirmed,
            true,
            Some(generation),
            EffectPlan::Complete,
        ),
        (
            ProviderEffectState::BusinessDenied,
            true,
            Some(generation),
            EffectPlan::Complete,
        ),
        (
            ProviderEffectState::ManualResolution,
            true,
            Some(generation),
            EffectPlan::Manual,
        ),
        (
            ProviderEffectState::Exhausted,
            true,
            Some(generation),
            EffectPlan::Exhausted,
        ),
        (
            ProviderEffectState::ReconcileNeeded,
            false,
            Some(generation),
            EffectPlan::ExpireReconciliation,
        ),
        (
            ProviderEffectState::ReconcileNeeded,
            true,
            None,
            EffectPlan::Reconcile,
        ),
        (
            ProviderEffectState::AwaitingAttempt,
            true,
            Some(generation + 1),
            EffectPlan::DenyGeneration {
                expected: ProviderEffectState::AwaitingAttempt,
            },
        ),
        (
            ProviderEffectState::RetryableUndispatched,
            true,
            None,
            EffectPlan::DenyGeneration {
                expected: ProviderEffectState::RetryableUndispatched,
            },
        ),
        (
            ProviderEffectState::AwaitingAttempt,
            false,
            Some(generation),
            EffectPlan::Dispatch {
                expected: ProviderEffectState::AwaitingAttempt,
            },
        ),
        (
            ProviderEffectState::RetryableUndispatched,
            false,
            Some(generation),
            EffectPlan::Dispatch {
                expected: ProviderEffectState::RetryableUndispatched,
            },
        ),
    ] {
        assert_eq!(
            snapshot(state, retention_active, current_generation).plan(generation),
            expected,
            "unexpected plan for {state:?}"
        );
    }
}

#[test]
fn future_dispatch_eligibility_never_defers_required_reconciliation() {
    for generation in [Some(4), Some(5), None] {
        let mut retained = snapshot(ProviderEffectState::ReconcileNeeded, true, generation);
        retained.retry_delay = Duration::from_secs(24 * 60 * 60);
        assert_eq!(retained.plan(4), EffectPlan::Reconcile);
        retained.retention_active = false;
        assert_eq!(retained.plan(4), EffectPlan::ExpireReconciliation);
    }
}

#[test]
fn retryable_outcomes_become_terminal_only_after_persisted_exhaustion() {
    assert_eq!(
        retry_or_exhausted(
            ProviderEffectState::RetryableUndispatched,
            CODE_PROVIDER_UNDISPATCHED,
            "retry",
            None,
        )
        .kind,
        runledger_core::jobs::JobFailureKind::Retryable
    );
    let exhausted = retry_or_exhausted(
        ProviderEffectState::Exhausted,
        CODE_PROVIDER_UNDISPATCHED,
        "retry",
        None,
    );
    assert_eq!(
        exhausted.kind,
        runledger_core::jobs::JobFailureKind::Terminal
    );
    assert_eq!(exhausted.code, CODE_ATTEMPTS_EXHAUSTED);
}

#[test]
fn admission_failures_preserve_distinct_native_causes() {
    for (error, code, interrupted) in [
        (AdmissionError::Overloaded, CODE_ADMISSION_OVERLOADED, false),
        (AdmissionError::Closed, CODE_ADMISSION_CLOSED, false),
        (
            AdmissionError::Interrupted(batter::operation::Interruption::DeadlineExceeded),
            CODE_ADMISSION_INTERRUPTED,
            true,
        ),
    ] {
        let (actual, _, actual_interrupted) = admission_failure_details(&error);
        assert_eq!(actual, code);
        assert_eq!(actual_interrupted, interrupted);
    }
}
