//! Worker ownership of each handler invocation's exit signal.

use runledger_core::jobs::{JobInvocation, JobInvocationOwner};

use crate::RuntimeCallbackFailure;
use crate::panic_payload::panic_payload_message;
use crate::settlement::TaskRegistry;

// Preserve the existing callback classification for all exit notifications,
// including a waiter wake that fails before the exit hooks run.
const EXIT_HOOK_CALLBACK: &str = "job_invocation_exit_hook";

/// Ends one handler invocation's exit signal when dropped.
///
/// The worker drops it after destroying the handler future and before it
/// persists the outcome, on every returned path: result, panic, deadline, and
/// lease loss or lease-maintenance failure. Aborting or destroying the task
/// that drives the invocation drops it too. A graceful stop request does not,
/// so draining keeps an active invocation running until it finishes or native
/// escalation abandons it. Contained waiter-wake and exit-hook panics are retained
/// as callback evidence, because the observers they should have notified may still run.
pub(super) struct InvocationExit {
    owner: Option<JobInvocationOwner>,
    invocation: JobInvocation,
    settlement: TaskRegistry,
}

impl InvocationExit {
    pub(super) fn new(settlement: TaskRegistry) -> Self {
        let owner = JobInvocationOwner::new();
        Self {
            invocation: owner.invocation(),
            owner: Some(owner),
            settlement,
        }
    }

    pub(super) fn observe(&self) -> JobInvocation {
        self.invocation.clone()
    }
}

impl Drop for InvocationExit {
    fn drop(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        let panics = owner.end();
        for payload in panics.payloads() {
            self.settlement
                .record_callback(RuntimeCallbackFailure::Panicked {
                    callback: EXIT_HOOK_CALLBACK,
                    message: panic_payload_message(payload),
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Wake, Waker};

    use super::*;
    use crate::shutdown::ShutdownSignal;

    #[test]
    fn contained_exit_hook_panics_are_retained_as_callback_evidence() {
        let (shutdown, _stop) = ShutdownSignal::channel();
        let registry = TaskRegistry::supervised(shutdown.clone());
        let exit = InvocationExit::new(registry.clone());
        let invocation = exit.observe();
        let notified = Arc::new(AtomicBool::new(false));
        invocation.on_end(|| panic!("exit hook failure"));
        let flag = Arc::clone(&notified);
        invocation.on_end(move || flag.store(true, Ordering::SeqCst));
        drop(exit);
        assert!(invocation.has_ended());
        assert!(notified.load(Ordering::SeqCst), "later hooks still run");
        let (failures, prior) = registry.callback_snapshot();
        assert!(failures.is_empty());
        assert_eq!(prior, 1, "a panic before shutdown remains sticky evidence");

        shutdown.request();
        let exit = InvocationExit::new(registry.clone());
        exit.observe()
            .on_end(|| panic!("exit hook failure after shutdown"));
        drop(exit);
        let (failures, _) = registry.callback_snapshot();
        assert!(matches!(
            failures.as_slice(),
            [RuntimeCallbackFailure::Panicked {
                callback: EXIT_HOOK_CALLBACK,
                ..
            }]
        ));
    }

    struct PanickingWaker;

    impl Wake for PanickingWaker {
        fn wake(self: Arc<Self>) {
            panic!("waiter wake failure");
        }
    }

    #[test]
    fn contained_waiter_panics_are_retained_as_callback_evidence() {
        let (shutdown, _stop) = ShutdownSignal::channel();
        let registry = TaskRegistry::supervised(shutdown.clone());
        shutdown.request();
        let exit = InvocationExit::new(registry.clone());
        let invocation = exit.observe();
        let notified = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&notified);
        invocation.on_end(move || flag.store(true, Ordering::SeqCst));
        let waker = Waker::from(Arc::new(PanickingWaker));
        let mut waiter = invocation.ended();
        assert!(
            Pin::new(&mut waiter)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );

        drop(exit);

        assert!(invocation.has_ended());
        assert!(
            notified.load(Ordering::SeqCst),
            "cancellation hooks still run"
        );
        let (failures, prior) = registry.callback_snapshot();
        assert_eq!(prior, 0);
        assert!(matches!(
            failures.as_slice(),
            [RuntimeCallbackFailure::Panicked { callback: EXIT_HOOK_CALLBACK, message }]
                if message == "waiter wake failure"
        ));
    }

    struct PanicOnDrop(Arc<AtomicBool>);

    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
            panic!("opaque payload destruction");
        }
    }

    struct PayloadWaker(Arc<AtomicBool>);

    impl Wake for PayloadWaker {
        fn wake(self: Arc<Self>) {
            std::panic::panic_any(PanicOnDrop(Arc::clone(&self.0)));
        }
    }

    #[test]
    fn opaque_payloads_cannot_escape_or_skip_later_diagnostics() {
        let (shutdown, _stop) = ShutdownSignal::channel();
        let registry = TaskRegistry::supervised(shutdown.clone());
        shutdown.request();
        let exit = InvocationExit::new(registry.clone());
        let destroyed = Arc::new(AtomicBool::new(false));
        let waker = Waker::from(Arc::new(PayloadWaker(Arc::clone(&destroyed))));
        let invocation = exit.observe();
        let mut waiter = invocation.ended();
        assert!(
            Pin::new(&mut waiter)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        invocation.on_end(|| panic!("later hook failure"));

        drop(exit);

        assert!(invocation.has_ended());
        assert!(
            !destroyed.load(Ordering::SeqCst),
            "opaque allocation retained"
        );
        let (failures, prior) = registry.callback_snapshot();
        assert_eq!(prior, 0);
        assert!(matches!(
            failures.as_slice(),
            [RuntimeCallbackFailure::Panicked { callback: EXIT_HOOK_CALLBACK, message: first },
             RuntimeCallbackFailure::Panicked { callback: EXIT_HOOK_CALLBACK, message: second }]
                if first == "non-string panic payload" && second == "later hook failure"
        ));
    }
}
