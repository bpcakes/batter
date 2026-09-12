//! The only owner of lifecycle state transitions and admission decisions.
//!
//! All transitions share one mutex. An atomic snapshot keeps readiness reads
//! available while admission is held; publishing it requires that same guard.
//! Callers cannot write readiness or startup/shutdown flags.

#[cfg(test)]
mod tests;

use super::{ProcessAdmissionError, Readiness};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;

pub(super) struct Shared {
    admission: Mutex<AdmissionState>,
    published_readiness: AtomicU8,
    drain: CancellationToken,
    cancel: CancellationToken,
    changed: Notify,
}

struct AdmissionState {
    readiness: Readiness,
    supervised: bool,
    driver_started: bool,
    ready_requested: bool,
    pending_startups: usize,
    finite_active: usize,
    forced: bool,
    failed: bool,
    stop_started: Option<Instant>,
}

impl AdmissionState {
    fn publish_ready(&mut self) {
        if self.readiness == Readiness::Starting
            && self.ready_requested
            && (!self.supervised || self.driver_started)
            && self.pending_startups == 0
        {
            self.readiness = Readiness::Ready;
        }
    }

    fn request_drain(&mut self, started: Instant) {
        let started = started.min(Instant::now());
        self.stop_started = Some(self.stop_started.map_or(started, |old| old.min(started)));
        if self.readiness != Readiness::Stopped {
            self.readiness = Readiness::Draining;
        }
    }
}

impl Shared {
    pub(super) fn new(supervised: bool) -> Self {
        Self {
            admission: Mutex::new(AdmissionState {
                readiness: Readiness::Starting,
                supervised,
                driver_started: false,
                ready_requested: false,
                pending_startups: 0,
                finite_active: 0,
                forced: false,
                failed: false,
                stop_started: None,
            }),
            published_readiness: AtomicU8::new(Readiness::Starting as u8),
            drain: CancellationToken::new(),
            cancel: CancellationToken::new(),
            changed: Notify::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, AdmissionState> {
        self.admission
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(super) fn admission(&self) -> Admission<'_> {
        Admission(self.lock())
    }

    pub(super) fn readiness(&self) -> Readiness {
        const STARTING: u8 = Readiness::Starting as u8;
        const READY: u8 = Readiness::Ready as u8;
        const DRAINING: u8 = Readiness::Draining as u8;
        match self.published_readiness.load(Ordering::Acquire) {
            STARTING => Readiness::Starting,
            READY => Readiness::Ready,
            DRAINING => Readiness::Draining,
            _ => Readiness::Stopped,
        }
    }

    // The only snapshot writer. Its argument retains the transition lock.
    fn publish_readiness(&self, state: &MutexGuard<'_, AdmissionState>) {
        self.published_readiness
            .store(state.readiness as u8, Ordering::Release);
    }

    pub(super) fn mark_ready(&self) -> bool {
        {
            let mut state = self.lock();
            if state.ready_requested
                || matches!(state.readiness, Readiness::Draining | Readiness::Stopped)
            {
                return false;
            }
            state.ready_requested = true;
            state.publish_ready();
            self.publish_readiness(&state);
        }
        self.changed.notify_waiters();
        true
    }

    pub(super) fn register_component(&self) {
        self.lock().pending_startups += 1;
    }

    pub(super) fn mark_started(&self, startup: &AtomicBool) -> bool {
        {
            let mut state = self.lock();
            if startup.swap(true, Ordering::AcqRel) {
                return false;
            }
            state.pending_startups -= 1;
            state.publish_ready();
            self.publish_readiness(&state);
        }
        self.changed.notify_waiters();
        true
    }

    pub(super) fn start_driver(&self) {
        {
            let mut state = self.lock();
            state.driver_started = true;
            state.publish_ready();
            self.publish_readiness(&state);
        }
        self.changed.notify_waiters();
    }

    pub(super) fn stop_driver(&self) {
        {
            let mut state = self.lock();
            state.readiness = Readiness::Stopped;
            self.publish_readiness(&state);
        }
        self.changed.notify_waiters();
    }

    pub(super) fn request(&self) {
        self.request_since(Instant::now());
    }

    pub(super) fn request_since(&self, started: Instant) {
        {
            let mut state = self.lock();
            state.request_drain(started);
            self.publish_readiness(&state);
        }
        // Waking arbitrary application futures must happen outside our mutex.
        self.drain.cancel();
        self.changed.notify_waiters();
    }

    pub(super) fn stop_started(&self) -> Option<Instant> {
        self.lock().stop_started
    }

    pub(super) async fn stop_before(&self, known: Instant) -> Instant {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(started) = self.stop_started().filter(|started| *started < known) {
                return started;
            }
            changed.await;
        }
    }

    pub(super) async fn phase_elapsed(&self, allowance: std::time::Duration) {
        loop {
            let started = self.stop_started().expect("phase wait follows drain");
            let deadline = started.checked_add(allowance).unwrap_or_else(Instant::now);
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => return,
                _ = self.stop_before(started) => {},
            }
        }
    }

    pub(super) fn force_cancel(&self) {
        {
            let mut state = self.lock();
            state.request_drain(Instant::now());
            state.forced = true;
            self.publish_readiness(&state);
        }
        self.drain.cancel();
        self.cancel.cancel();
        self.changed.notify_waiters();
    }

    pub(super) fn fail_task(&self) {
        self.lock().failed = true;
    }

    pub(super) fn has_finite_tasks(&self) -> bool {
        self.lock().finite_active != 0
    }

    pub(super) fn finish_finite(&self, active: &AtomicBool) {
        {
            let mut state = self.lock();
            active.store(false, Ordering::Release);
            state.finite_active -= 1;
        }
        self.changed.notify_waiters();
    }

    pub(super) async fn wait_ready(&self) -> Result<(), Readiness> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.readiness() {
                Readiness::Ready => return Ok(()),
                Readiness::Starting => notified.await,
                state => return Err(state),
            }
        }
    }

    pub(super) async fn draining(&self) {
        self.drain.cancelled().await;
    }

    pub(super) async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub(super) fn operation_token(&self) -> CancellationToken {
        self.cancel.child_token()
    }
}

/// Retains the transition lock through the actual bounded enqueue. There is no
/// mutable access to state outside this module. Native enqueue may wake its
/// receiver under this guard; readiness snapshots therefore remain lock-free.
pub(super) struct Admission<'a>(MutexGuard<'a, AdmissionState>);

impl Admission<'_> {
    pub(super) fn check(
        &self,
        ancestor: Option<&AtomicBool>,
        queue_closed: bool,
    ) -> Result<(), ProcessAdmissionError> {
        let state = &self.0;
        if state.forced
            || state.failed
            || queue_closed
            || ancestor.is_some_and(|active| !active.load(Ordering::Acquire))
            || (ancestor.is_none()
                && matches!(state.readiness, Readiness::Draining | Readiness::Stopped))
        {
            return Err(ProcessAdmissionError::Closed);
        }
        if !state.driver_started {
            return Err(ProcessAdmissionError::NotRunning);
        }
        if ancestor.is_none() && state.readiness != Readiness::Ready {
            return Err(ProcessAdmissionError::NotReady);
        }
        Ok(())
    }

    pub(super) fn admit_finite(&mut self) {
        self.0.finite_active += 1;
    }
}
