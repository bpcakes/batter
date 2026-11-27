//! The scripted `Limiter` the layer tests drive.

use std::{
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use runlimit_core::{
    BatchDecision, Check, Decision, FixedWindowPolicy, Limiter, QuotaDenial, QuotaMode,
    RateLimitPolicy, SubjectKey,
};

#[derive(Clone, Copy)]
pub(super) enum StubOutcome {
    Decision(Decision),
    BackendError,
    DecisionFromPolicyMode,
}

#[derive(Clone)]
pub(super) struct StubLimiter {
    pub(super) calls: Arc<AtomicUsize>,
    pub(super) subjects: Arc<Mutex<Vec<SubjectKey>>>,
    outcome: StubOutcome,
}

impl StubLimiter {
    pub(super) fn returning(decision: Decision) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            subjects: Arc::new(Mutex::new(Vec::new())),
            outcome: StubOutcome::Decision(decision),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            subjects: Arc::new(Mutex::new(Vec::new())),
            outcome: StubOutcome::BackendError,
        }
    }

    pub(super) fn deciding_from_policy_mode() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            subjects: Arc::new(Mutex::new(Vec::new())),
            outcome: StubOutcome::DecisionFromPolicyMode,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StubError;

impl fmt::Display for StubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("stub backend failure")
    }
}

impl Error for StubError {}

// Like every `Limiter`, the stub does no work before its future is first
// polled: `stub_limiter_defers_its_work_to_first_poll` holds it to that.
impl Limiter for StubLimiter {
    type Policy = FixedWindowPolicy;
    type CheckError = StubError;
    type CheckAllError = StubError;

    #[expect(
        clippy::unused_async_trait_impl,
        reason = "the async body defers the stub's recording and decision to first poll, as \
                  `Limiter` requires; `future::ready` would evaluate them eagerly"
    )]
    async fn check(&self, check: &Check<'_, Self::Policy>) -> Result<Decision, Self::CheckError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.subjects.lock().unwrap().push(check.subject());
        match self.outcome {
            StubOutcome::Decision(decision) => Ok(decision),
            StubOutcome::BackendError => Err(StubError),
            StubOutcome::DecisionFromPolicyMode => {
                let denial = QuotaDenial::new(check.policy().capacity(), Duration::from_secs(1));
                Ok(match check.policy().quota_mode() {
                    QuotaMode::Enforce => Decision::denied(denial),
                    QuotaMode::Shadow => Decision::shadow_denied(denial),
                })
            }
        }
    }

    #[expect(
        clippy::unused_async_trait_impl,
        reason = "the async body defers the stub's failure to first poll, as `Limiter` requires"
    )]
    async fn check_all(
        &self,
        _checks: &[Check<'_, Self::Policy>],
    ) -> Result<BatchDecision, Self::CheckAllError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Err(StubError)
    }
}
