use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ManagedSettlement, ShutdownBudget, Supervisor},
    operation::OperationContext,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct NativeReport {
    pub joined: bool,
    pub detail: &'static str,
}

impl ManagedSettlement for NativeReport {
    fn is_success(&self) -> bool {
        self.joined
    }
    fn allows_dependency_cleanup(&self) -> bool {
        self.joined
    }
}

pub struct Dropped(pub Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
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

pub fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(10)).unwrap()
}

pub fn cleanup(supervisor: &mut Supervisor, prerequisite: Arc<AtomicBool>) -> Arc<AtomicBool> {
    let closed = Arc::new(AtomicBool::new(false));
    let closing = closed.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            assert!(
                prerequisite.load(Ordering::SeqCst),
                "native descendant still uses dependency"
            );
            closing.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    closed
}
