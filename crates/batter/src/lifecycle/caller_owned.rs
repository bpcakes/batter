use super::{LifecycleCoordinator, ReadinessApproval};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

pub(super) struct SupervisorOwnership {
    // First field: abandonment is published before the approval Arc is released.
    emergency: EmergencyShutdown,
    approval: ReadinessApproval,
}

impl SupervisorOwnership {
    pub(super) fn new(coordinator: LifecycleCoordinator, approval: ReadinessApproval) -> Self {
        Self {
            emergency: EmergencyShutdown(coordinator),
            approval,
        }
    }

    pub(super) fn into_driver<F>(self, future: F) -> (ReadinessApproval, CallerOwnedDriver<F>) {
        let Self {
            emergency,
            approval,
        } = self;
        (
            approval,
            CallerOwnedDriver {
                _ownership: emergency,
                future,
            },
        )
    }
}

struct EmergencyShutdown(LifecycleCoordinator);

impl Drop for EmergencyShutdown {
    fn drop(&mut self) {
        self.0.shared.force_cancel();
    }
}

pin_project_lite::pin_project! {
    // Struct field order makes signaling precede inner-future destruction,
    // including before the first poll; async capture drop order is not needed.
    pub(super) struct CallerOwnedDriver<F> {
        _ownership: EmergencyShutdown,
        #[pin]
        future: F,
    }
}

impl<F: Future> Future for CallerOwnedDriver<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().future.poll(cx)
    }
}

/// A caller-owned lifecycle driver whose application approval is still pending.
///
/// This linear value keeps the sole [`ReadinessApproval`] with the driver.
/// Awaiting it drives shutdown without approving readiness. The inner future is
/// pinned independently, so this outer value remains movable after polling and
/// [`Self::approve_readiness`] can consume it at the later policy boundary.
///
/// ```no_run
/// use batter::lifecycle::{Supervisor, UnapprovedDriver};
///
/// # async fn example(supervisor: Supervisor) {
/// let pending: UnapprovedDriver<_> =
///     supervisor.run_until_unapproved(std::future::pending());
/// let report = pending.approve_readiness().await;
/// # let _ = report;
/// # }
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::UnapprovedDriver;
/// fn cannot_clone<F>(driver: UnapprovedDriver<F>) {
///     let duplicate = driver.clone();
/// }
/// ```
///
/// ```compile_fail,E0382
/// use batter::lifecycle::UnapprovedDriver;
/// fn cannot_approve_twice<F>(driver: UnapprovedDriver<F>) {
///     let approved = driver.approve_readiness();
///     let again = driver.approve_readiness();
/// }
/// ```
#[must_use = "retain the caller-owned driver and decide whether to approve readiness"]
pub struct UnapprovedDriver<F> {
    // First field: driver abandonment is published before approval is released.
    driver: Pin<Box<F>>,
    approval: ReadinessApproval,
}

impl<F> UnapprovedDriver<F> {
    pub(super) fn new(driver: F, approval: ReadinessApproval) -> Self {
        Self {
            driver: Box::pin(driver),
            approval,
        }
    }

    /// Consume the process's only application-readiness decision and return the
    /// same caller-owned driver. Approval does not poll the driver or bypass
    /// pending component startup.
    pub fn approve_readiness(self) -> Pin<Box<F>> {
        let Self { driver, approval } = self;
        approval.approve();
        driver
    }
}

impl<F: Future> Future for UnapprovedDriver<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().driver.as_mut().poll(cx)
    }
}
