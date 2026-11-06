//! A single coalesced request, owned and closed by the periodic driver.

use super::PeriodicPolicy;
use crate::{ConfigurationError, validation};
use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::Notify;

/// Experimental validated policy for [`super::register_wakeable_in`].
///
/// The positive spacing is a minimum between **all** invocation starts,
/// including cadence ticks. It may exceed the interval; due ticks then wait
/// for spacing instead of being discarded. See the registration example.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WakeablePolicy {
    periodic: PeriodicPolicy,
    spacing: Duration,
}

impl WakeablePolicy {
    /// Add a positive, representable spacing floor to a validated policy.
    pub fn new(
        periodic: PeriodicPolicy,
        wake_spacing: Duration,
    ) -> Result<Self, ConfigurationError> {
        validation::positive(wake_spacing, "periodic wake spacing")?;
        Ok(Self {
            periodic,
            spacing: wake_spacing,
        })
    }

    /// The underlying cadence, run budget, startup policy and stopping class.
    pub fn periodic(self) -> PeriodicPolicy {
        self.periodic
    }

    /// Minimum elapsed time between successive invocation starts.
    pub fn wake_spacing(self) -> Duration {
        self.spacing
    }
}

/// Experimental, cloneable request capability for one registered component.
///
/// Only [`super::register_wakeable_in`] can create it. It owns no work, cannot
/// alter timing or admission, and does not keep the component alive. Dropping
/// every handle leaves the ordinary cadence running. See the registration
/// example for construction and use.
#[derive(Clone, Debug)]
pub struct PeriodicWake {
    shared: Weak<Shared>,
}

impl PeriodicWake {
    /// Request one bounded invocation, coalescing with any pending request or
    /// due cadence tick. Wakes during a run request a later serial invocation.
    ///
    /// Wakes before first execution coalesce with the immediate first run.
    /// During initialization they remain subject to its original allowance.
    /// This call does no application work, grants no acknowledgement of work,
    /// and is a no-op after the component ends or its registration is dropped.
    pub fn wake(&self) {
        if let Some(shared) = self.shared.upgrade() {
            let notify = {
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                match *state {
                    WakeState::Idle => {
                        *state = WakeState::Pending;
                        true
                    }
                    WakeState::Pending | WakeState::Ended => false,
                }
            };
            if notify {
                shared.changed.notify_one();
            }
        }
    }
}

#[derive(Debug)]
enum WakeState {
    Idle,
    Pending,
    Ended,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<WakeState>,
    changed: Notify,
}

pub(super) struct WakeOwner(Arc<Shared>);

impl WakeOwner {
    pub(super) fn new() -> (Self, PeriodicWake) {
        let shared = Arc::new(Shared {
            state: Mutex::new(WakeState::Idle),
            changed: Notify::new(),
        });
        let handle = PeriodicWake {
            shared: Arc::downgrade(&shared),
        };
        (Self(shared), handle)
    }

    /// Cancellation leaves the pending request intact until admission.
    pub(super) async fn pending(&self) {
        loop {
            let changed = self.0.changed.notified();
            if matches!(
                *self.0.state.lock().unwrap_or_else(|e| e.into_inner()),
                WakeState::Pending
            ) {
                return;
            }
            changed.await;
        }
    }

    pub(super) fn consume(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        match *state {
            WakeState::Idle | WakeState::Pending => *state = WakeState::Idle,
            WakeState::Ended => unreachable!("only owner destruction ends wake admission"),
        }
    }
}

impl Drop for WakeOwner {
    fn drop(&mut self) {
        *self.0.state.lock().unwrap_or_else(|e| e.into_inner()) = WakeState::Ended;
    }
}
