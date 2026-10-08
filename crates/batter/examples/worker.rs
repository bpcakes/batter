//! Library-owned recurring maintenance. Batter owns the serial schedule, the
//! per-run budget, startup acknowledgement and shutdown ordering; this example
//! owns only the bounded operations and what they mean. Durable work across
//! process death belongs in Runledger.
//!
//! Two stopping classes are demonstrated. Pruning stops at drain, because
//! nothing else needs it afterwards. Lease renewal supports other work through
//! drain, so admitted finite work can finish while its lease stays renewed.
mod support;

use batter::{
    BoxError,
    lifecycle::{Fatal, ProcessCapacity, ShutdownSuccess, Supervisor},
    operation::OperationContext,
    periodic::{
        PeriodicPolicy, PeriodicReader, PeriodicRun, PeriodicShutdown, PeriodicStartup,
        register_periodic_in,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const PRUNE_INTERVAL: Duration = Duration::from_millis(200);
const RENEWAL_INTERVAL: Duration = Duration::from_millis(150);
const RUN_BUDGET: Duration = Duration::from_millis(500);
/// Renewal must succeed once before the process reports itself ready.
const RENEWAL_INITIALIZATION: Duration = Duration::from_secs(2);

/// A transient maintenance failure. Propagating it with `?` keeps the schedule
/// running; only an explicit escalation would drain the process.
#[derive(Debug, thiserror::Error)]
#[error("maintenance store is temporarily unavailable")]
struct StoreUnavailable;

/// Generic expiring local state, owned by the application, not by Batter.
#[derive(Default)]
struct Store {
    entries: Mutex<BTreeMap<u64, u64>>,
    clock: AtomicU64,
    pruned: AtomicU64,
    unavailable_until: AtomicU64,
}

impl Store {
    fn insert(&self, id: u64, lifetime: u64) {
        let expires_at = self.clock.load(Ordering::Acquire) + lifetime;
        self.entries
            .lock()
            .expect("store is not poisoned")
            .insert(id, expires_at);
    }

    /// Remove every expired entry, or report the store as unavailable.
    fn prune(&self) -> Result<u64, StoreUnavailable> {
        let now = self.clock.fetch_add(1, Ordering::AcqRel) + 1;
        if now <= self.unavailable_until.load(Ordering::Acquire) {
            return Err(StoreUnavailable);
        }
        let mut entries = self.entries.lock().expect("store is not poisoned");
        let expired: Vec<u64> = entries
            .iter()
            .filter(|(_, expires_at)| **expires_at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            entries.remove(id);
        }
        let removed = expired.len() as u64;
        self.pruned.fetch_add(removed, Ordering::Release);
        Ok(removed)
    }
}

/// A renewable local lease. Batter does not verify the remote effect of a
/// renewal, nor that a lease is still held; that stays application policy.
#[derive(Default)]
struct Lease {
    generation: AtomicU64,
}

/// One pruning run. The library created `scope`, so its deadline and
/// cancellation already belong to this invocation.
async fn prune(scope: OperationContext, store: Arc<Store>) -> PeriodicRun<StoreUnavailable> {
    scope.check().map_err(|_| StoreUnavailable)?;
    let removed = store.prune()?;
    tracing::debug!(removed, "pruned expired entries");
    Ok(())
}

/// One renewal run. Its success is what acknowledges startup the first time.
async fn renew(scope: OperationContext, lease: Arc<Lease>) -> PeriodicRun<StoreUnavailable> {
    scope.check().map_err(|_| StoreUnavailable)?;
    let generation = lease.generation.fetch_add(1, Ordering::AcqRel) + 1;
    tracing::debug!(generation, "renewed lease");
    Ok(())
}

/// What one bounded demonstration observed.
struct Observed {
    success: ShutdownSuccess,
    pruning: PeriodicReader,
    renewal: PeriodicReader,
    prunings_during_drain: u64,
    renewals_during_drain: u64,
    pruned_entries: u64,
}

async fn demonstrate(window: Duration, signals: bool) -> Result<Observed, BoxError> {
    let store = Arc::new(Store::default());
    // One injected transient window shows retained, bounded failure evidence.
    store.unavailable_until.store(2, Ordering::Release);
    let lease = Arc::new(Lease::default());
    let mut supervisor =
        Supervisor::with_process_capacity(support::shutdown_budget(), ProcessCapacity::new(4)?);
    if signals {
        support::register_signals(&mut supervisor)?;
    }
    let pruning = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        PeriodicPolicy::new(
            PRUNE_INTERVAL,
            RUN_BUDGET,
            PeriodicStartup::immediate(),
            PeriodicShutdown::StopAtDrain,
        )?,
        {
            let store = store.clone();
            move |scope| prune(scope, store.clone())
        },
    )?;
    let renewal = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        PeriodicPolicy::new(
            RENEWAL_INTERVAL,
            RUN_BUDGET,
            PeriodicStartup::after_first_success(RENEWAL_INITIALIZATION)?,
            PeriodicShutdown::SupportThroughDrain,
        )?,
        {
            let lease = lease.clone();
            move |scope| renew(scope, lease.clone())
        },
    )?;
    let process = supervisor
        .process_handle()
        .expect("configured process capacity");
    let running = supervisor.start();
    // Readiness waits for the first successful renewal, not merely for a loop.
    running
        .status()
        .wait_ready()
        .await
        .map_err(|state| format!("process did not become ready: {state:?}"))?;
    let writing = store.clone();
    let receipt = process.try_spawn("entries.write", move |scope| async move {
        for id in 0..8u64 {
            writing.insert(id, 2);
            tokio::time::sleep(Duration::from_millis(100)).await;
            if scope.signal().is_draining() {
                break;
            }
        }
        // Finishing a batch during drain is exactly what support exists for.
        scope.signal().draining().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        writing.insert(100, 1);
        Ok::<_, Fatal<StoreUnavailable>>(())
    })?;
    tokio::time::sleep(window).await;
    let prunings_before = pruning.snapshot().invocations;
    let renewals_before = renewal.snapshot().succeeded;
    let success = running.shutdown_checked().await?;
    receipt.wait().await?;
    Ok(Observed {
        prunings_during_drain: pruning.snapshot().invocations - prunings_before,
        renewals_during_drain: renewal.snapshot().succeeded - renewals_before,
        pruned_entries: store.pruned.load(Ordering::Acquire),
        success,
        pruning,
        renewal,
    })
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let observed = demonstrate(Duration::from_millis(900), true).await?;
    let pruning = observed.pruning.snapshot();
    let renewal = observed.renewal.snapshot();
    tracing::info!(
        prune_invocations = pruning.invocations,
        prune_failures = pruning.recoverable_failures,
        prune_unsampled = pruning.unsampled_failures,
        pruned_entries = observed.pruned_entries,
        prunings_during_drain = observed.prunings_during_drain,
        renewals = renewal.succeeded,
        renewals_during_drain = observed.renewals_during_drain,
        "maintenance finished"
    );
    // The same bounded evidence is in the report, without a second join.
    for record in &observed.success.report().periodic {
        tracing::info!(
            component = record.name,
            invocations = record.summary.invocations,
            ?record.summary.completion,
            "retained periodic evidence"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::periodic::PeriodicCompletion;

    #[tokio::test]
    async fn support_renews_through_drain_while_pruning_stops_at_it() {
        let observed = demonstrate(Duration::from_millis(900), false)
            .await
            .expect("the bounded demonstration completes successfully");
        // Pruning admitted nothing after the stop transition.
        assert_eq!(observed.prunings_during_drain, 0);
        // Renewal kept supporting the finite batch that was still draining.
        assert!(
            observed.renewals_during_drain >= 1,
            "support must renew during drain, observed {}",
            observed.renewals_during_drain,
        );
        let pruning = observed.pruning.snapshot();
        assert!(observed.pruned_entries >= 1);
        // The injected transient window is retained as bounded evidence and
        // did not drain the process or fail checked completion.
        assert!(pruning.recoverable_failures >= 1);
        assert!(pruning.first_failure.is_some());
        for record in &observed.success.report().periodic {
            assert_eq!(record.summary.completion, PeriodicCompletion::Stopped);
        }
        assert!(
            !format!("{:?}", observed.success.report())
                .contains("maintenance store is temporarily unavailable")
        );
    }
}
