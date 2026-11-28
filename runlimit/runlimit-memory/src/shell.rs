//! The one private shell behind both public quota stores.
//!
//! [`MemoryStore`](crate::MemoryStore) and [`GcraStore`](crate::GcraStore)
//! differ only in their algorithm, which names their policy, stored entry and
//! public errors. The clock, bounded shards and observer they own, the way
//! they are built, and the way they check, report, clear, recover and format
//! themselves exist once, here.

use std::{fmt, sync::Arc};

use runlimit_core::{BatchDecision, Check, Decision, Observer};

use crate::{
    Clock, MemoryStoreConfig, MemoryStoreStats, PoisonedShardError, SystemClock,
    shards::{
        BatchAlgorithm, BatchSessionError, BoundedShards, CheckSessionError, QuotaAlgorithm,
        execute_batch, execute_check,
    },
};

/// An algorithm a public store exposes, and how its session failures surface
/// as that store's public errors.
pub(crate) trait StoreAlgorithm: BatchAlgorithm {
    type CheckError;
    type CheckAllError;

    fn check_error(error: CheckSessionError<Self::EvalError>) -> Self::CheckError;
    fn check_all_error(error: BatchSessionError<Self::EvalError>) -> Self::CheckAllError;
}

/// The clock, chosen before any quota state exists, and the configuration a
/// store is built with.
pub(crate) struct ShellBuilder<C> {
    config: MemoryStoreConfig,
    clock: C,
}

impl ShellBuilder<SystemClock> {
    pub(crate) fn new(config: MemoryStoreConfig) -> Self {
        Self {
            config,
            clock: SystemClock::new(),
        }
    }
}

impl<C> ShellBuilder<C> {
    pub(crate) fn with_clock<D: Clock>(self, clock: D) -> ShellBuilder<D> {
        ShellBuilder {
            config: self.config,
            clock,
        }
    }
}

impl<C: Clock> ShellBuilder<C> {
    /// Builds empty shards whose timestamps use the selected clock.
    pub(crate) fn build<A: QuotaAlgorithm>(self) -> StoreShell<A, C> {
        StoreShell {
            clock: self.clock,
            shards: BoundedShards::new(self.config),
            observer: None,
        }
    }
}

/// The state every quota store owns. Clock readings are coordinates for every
/// stored timestamp, so the clock is fixed when the shell is built.
pub(crate) struct StoreShell<A: QuotaAlgorithm, C> {
    clock: C,
    shards: BoundedShards<A::Entry>,
    observer: Option<Arc<dyn Observer>>,
}

impl<A: QuotaAlgorithm, C> StoreShell<A, C> {
    pub(crate) fn set_observer(&mut self, observer: Arc<dyn Observer>) {
        self.observer = Some(observer);
    }

    /// Formats the store called `name`. Counter state is never printed and
    /// the observer is reported only as present or absent, so the output is
    /// marked non-exhaustive.
    pub(crate) fn fmt_debug(&self, name: &str, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(name)
            .field("config", self.shards.config())
            .field("has_observer", &self.observer.is_some())
            .field("poisoned_shards", &self.shards.poisoned_count())
            .field("shard_state", &Redacted)
            .finish_non_exhaustive()
    }

    pub(crate) fn stats(&self) -> Result<MemoryStoreStats, PoisonedShardError> {
        self.shards.stats()
    }

    pub(crate) fn clear(&self) -> Result<(), PoisonedShardError> {
        self.shards.clear()
    }

    pub(crate) fn recover_poisoned(&self) -> usize {
        self.shards.recover_poisoned()
    }

    #[cfg(test)]
    pub(crate) const fn shards(&self) -> &BoundedShards<A::Entry> {
        &self.shards
    }
}

impl<A: StoreAlgorithm, C: Clock> StoreShell<A, C> {
    pub(crate) fn check(&self, check: &Check<'_, A::Policy>) -> Result<Decision, A::CheckError> {
        execute_check::<A, C>(&self.shards, &self.clock, self.observer.as_deref(), check)
            .map_err(A::check_error)
    }

    pub(crate) fn check_all(
        &self,
        checks: &[Check<'_, A::Policy>],
    ) -> Result<BatchDecision, A::CheckAllError> {
        execute_batch::<A, C>(&self.shards, &self.clock, self.observer.as_deref(), checks)
            .map_err(A::check_all_error)
    }
}

/// Prints `[REDACTED]` without quotes in place of shard state.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use runlimit_core::{Check, FixedWindowPolicy, GcraPolicy, PolicyId, ScopeId, SubjectKey};

    use crate::{GcraStore, MemoryStore, MemoryStoreConfig};

    #[test]
    fn both_stores_print_one_redacted_shape() {
        let config = MemoryStoreConfig::new(8).unwrap();
        let fixed_window = MemoryStore::new(config.clone());
        let gcra = GcraStore::new(config);
        let subject = SubjectKey::from_digest([0xab; 32]);
        let fixed_window_policy = FixedWindowPolicy::new(
            PolicyId::new("shell.debug").unwrap(),
            ScopeId::new("client").unwrap(),
            2,
            Duration::from_mins(1),
        )
        .unwrap();
        let gcra_policy = GcraPolicy::new(
            PolicyId::new("shell.debug").unwrap(),
            ScopeId::new("client").unwrap(),
            2,
            Duration::from_mins(1),
            2,
        )
        .unwrap();
        assert!(
            fixed_window
                .check(&Check::new(subject.bind(&fixed_window_policy)))
                .unwrap()
                .permits_request()
        );
        assert!(
            gcra.check(&Check::new(subject.bind(&gcra_policy)))
                .unwrap()
                .permits_request()
        );

        let fixed_window = format!("{fixed_window:?}");
        let gcra = format!("{gcra:?}");

        assert_eq!(
            fixed_window.strip_prefix("MemoryStore "),
            gcra.strip_prefix("GcraStore ")
        );
        assert!(
            gcra.ends_with("poisoned_shards: 0, shard_state: [REDACTED], .. }"),
            "{gcra}"
        );
    }
}
