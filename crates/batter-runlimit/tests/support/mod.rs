#![allow(dead_code)]
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{RunningSupervisor, ShutdownBudget, Supervisor},
    operation::OperationContext,
};
use batter_runlimit::quota::ConsumptionError;
use runlimit_core::{
    Allowance, BatchDecision, Check, ConsumptionStatus, Decision, FixedWindowPolicy, Limiter,
    PolicyId, ScopeId, SubjectKey,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub fn policy(scope: &str, limit: u64) -> FixedWindowPolicy {
    FixedWindowPolicy::new(
        PolicyId::new("api.submit").unwrap(),
        ScopeId::new(scope).unwrap(),
        limit,
        Duration::from_secs(60),
    )
    .unwrap()
}
pub fn subject(n: u8) -> SubjectKey {
    SubjectKey::from_digest([n; 32])
}
pub fn allowed() -> BatchDecision {
    BatchDecision::allowed(vec![
        Allowance::new(
            runlimit_core::Capacity::new(10).unwrap(),
            9,
            Duration::from_secs(60),
        )
        .unwrap(),
    ])
    .unwrap()
}
pub fn context(ms: u64) -> OperationContext {
    OperationContext::new(Duration::from_millis(ms)).unwrap()
}
pub fn memory() -> runlimit_memory::MemoryStore {
    runlimit_memory::MemoryStore::new(
        runlimit_memory::MemoryStoreConfig::new(100)
            .unwrap()
            .with_shard_count(1)
            .unwrap(),
    )
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("secret-backend-cause")]
pub struct BackendFailure(pub ConsumptionStatus);
impl ConsumptionError for BackendFailure {
    fn consumption(&self) -> ConsumptionStatus {
        self.0
    }
}

#[derive(Clone)]
pub enum Mode {
    Return(BatchDecision),
    Fail(ConsumptionStatus),
    Pending,
    Delay(Duration),
    CancelThenAllow(OperationContext),
    CancelSavedThenAllow(Arc<std::sync::Mutex<Option<OperationContext>>>),
}

#[derive(Clone)]
pub struct Backend {
    pub mode: Mode,
    pub calls: Arc<AtomicUsize>,
    pub dropped: Arc<AtomicUsize>,
    pub entered: Arc<tokio::sync::Notify>,
}
impl Backend {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            calls: Arc::default(),
            dropped: Arc::default(),
            entered: Arc::default(),
        }
    }
}
struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Limiter for Backend {
    type Policy = FixedWindowPolicy;
    type CheckError = BackendFailure;
    type CheckAllError = BackendFailure;
    async fn check(&self, _: &Check<'_, Self::Policy>) -> Result<Decision, Self::CheckError> {
        unreachable!("the adapter must use one atomic batch")
    }
    async fn check_all(
        &self,
        _: &[Check<'_, Self::Policy>],
    ) -> Result<BatchDecision, Self::CheckAllError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _drop = DropCount(self.dropped.clone());
        self.entered.notify_one();
        match &self.mode {
            Mode::Return(value) => Ok(value.clone()),
            Mode::Fail(consumption) => Err(BackendFailure(*consumption)),
            Mode::Pending => std::future::pending().await,
            Mode::Delay(duration) => {
                tokio::time::sleep(*duration).await;
                Ok(allowed())
            }
            Mode::CancelThenAllow(context) => {
                context.cancel();
                Ok(allowed())
            }
            Mode::CancelSavedThenAllow(saved) => {
                saved
                    .lock()
                    .unwrap()
                    .as_ref()
                    .expect("authenticated context")
                    .cancel();
                Ok(allowed())
            }
        }
    }
}

pub fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(
        ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap(),
    )
}
pub async fn running() -> RunningSupervisor {
    let mut supervisor = supervisor();
    supervisor
        .register("component", |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            Ok(running.stopped())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    running
}
#[cfg(feature = "axum")]
pub fn request_policy(running: &RunningSupervisor, ms: u64) -> batter_axum::RequestPolicy {
    batter_axum::RequestPolicy::new(
        running.operation_admission(),
        batter_axum::ResponseConstructionBudget::new(Duration::from_millis(ms)).unwrap(),
    )
    .with_infrastructure_json()
}
pub async fn finish(running: RunningSupervisor) {
    batter_core::lifecycle::check_shutdown(running.shutdown().await).unwrap();
}
