//! Restricted evaluation capability for one invocation.
use batter_flags_core::{
    Decision, KillSwitch, Pinnable, Pinned, PinnedResolution, SnapshotView, SwitchState,
};

/// Why a job has no admission-time pin. These are application facts, not
/// locally provable states. A rejected pin instead owns its own fallback via
/// [`batter_flags_core::PinRefusal::fall_back_to_live`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnpinnedBecause {
    /// The scheduler materialized the job without an admission-time decision.
    ScheduledJob,
    /// The application's retained payload predates its pinning protocol.
    PayloadPredatesPinning,
}

/// One invocation's frozen view. Only the wrappers construct it; no general
/// evaluation or fresh-view capture is exposed.
///
/// ```compile_fail
/// # use batter_flags::{core::{ModeFlag, Toggle}, jobs::JobFlags};
/// fn bypass(flags: &JobFlags, flag: &ModeFlag<Toggle>) { flags.evaluate(flag); }
/// ```
pub struct JobFlags(pub(super) SnapshotView);

impl JobFlags {
    /// Resolve against the invocation's view, retaining the refusal-owned
    /// fallback. Persisting a pin remains the application's transaction duty.
    ///
    /// ```
    /// use batter_flags::{core::{ModeFlag, Pinned, PinnedResolution, Toggle}, jobs::JobFlags};
    /// fn decide(flags: &JobFlags, flag: &ModeFlag<Toggle>, pin: &Pinned<Toggle>) -> Toggle {
    ///     match flags.resolve_pinned(flag, pin) {
    ///         PinnedResolution::Honoured(decision) => decision.value(),
    ///         PinnedResolution::Overridden { decision, .. } => decision.value(),
    ///         // Only if the application's policy permits a fresh decision:
    ///         PinnedResolution::Incompatible(refusal) => refusal.fall_back_to_live().value(),
    ///     }
    /// }
    /// ```
    pub fn resolve_pinned<F: Pinnable>(
        &self,
        flag: &F,
        pin: &Pinned<F::Value>,
    ) -> PinnedResolution<'_, F::Value> {
        self.0.resolve_pinned(flag, pin)
    }

    /// Evaluate only when the application declares why no pin exists.
    /// After a refusal, use its consuming `fall_back_to_live` instead.
    pub fn evaluate_unpinned<F: Pinnable>(
        &self,
        flag: &F,
        _because: UnpinnedBecause,
    ) -> Decision<F::Value> {
        self.0.evaluate(flag)
    }

    /// Read a switch from the same view that admitted this invocation.
    pub fn switch(&self, switch: &KillSwitch) -> Decision<SwitchState> {
        self.0.switch(switch)
    }
}
