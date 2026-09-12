use batter::{
    BoxError, RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{ManagedComponent, ManagedSettlement, ShutdownBudget, Supervisor},
    operation::OperationContext,
    registration::RegistrationTarget,
};
use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn supervisor() -> Supervisor {
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

struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct NativeReport;
impl ManagedSettlement for NativeReport {
    fn is_success(&self) -> bool {
        true
    }
    fn allows_dependency_cleanup(&self) -> bool {
        true
    }
}

#[test]
fn invalid_and_duplicate_component_registration_never_invoke_rejected_factories() {
    let invoked = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    supervisor
        .registration()
        .register("worker", |_| async { Ok(()) })
        .unwrap();

    for expected in [
        RegistrationError::InvalidName,
        RegistrationError::Duplicate("worker"),
    ] {
        let calls = invoked.clone();
        let captured = Dropped(dropped.clone());
        let name = match expected {
            RegistrationError::InvalidName => "",
            RegistrationError::Duplicate(_) => "worker",
        };
        let error = supervisor
            .registration()
            .register(name, move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                drop(captured);
                async { Ok(()) }
            })
            .unwrap_err();
        assert_eq!(error, expected);
    }
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
}

#[test]
fn invalid_and_duplicate_managed_registration_release_inert_captures() {
    let invoked = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    let context = || OperationContext::new(Duration::from_secs(1)).unwrap();
    supervisor
        .registration()
        .register_managed("native", context(), |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                |started| started,
                async { NativeReport },
            ))
        })
        .unwrap();

    for name in ["", "native"] {
        let calls = invoked.clone();
        let captured = Dropped(dropped.clone());
        let result = supervisor
            .registration()
            .register_managed(name, context(), move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                drop(captured);
                Ok(ManagedComponent::new(
                    async { Ok(()) },
                    pending(),
                    |started| started,
                    async { NativeReport },
                ))
            });
        assert!(result.is_err());
    }
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
}

#[test]
fn registration_reborrows_and_reserves_cleanup_without_process_control() {
    let mut supervisor = supervisor();
    let mut registration = supervisor.registration();
    registration
        .registration()
        .register("first", |_| async { Ok::<_, BoxError>(()) })
        .unwrap();
    registration
        .register("second", |_| async { Ok::<_, BoxError>(()) })
        .unwrap();
    registration
        .reserve_cleanup("resource")
        .unwrap()
        .register(|| async { Ok(()) });
}
