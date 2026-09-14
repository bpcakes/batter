use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Notify;

#[derive(Default)]
struct VerificationLedgerPauseState {
    reached: Notify,
    release: Notify,
}

type PauseSlot = OnceLock<Mutex<Option<Arc<VerificationLedgerPauseState>>>>;

static VERIFICATION_LEDGER_LOCK_PAUSE: PauseSlot = OnceLock::new();
static VERIFICATION_LEDGER_HISTORY_PAUSE: PauseSlot = OnceLock::new();

/// A one-shot test barrier after a protected-ledger lock attempt and before the
/// verifier proves the named relation is the relation it locked.
///
/// This narrow seam exists only with the `test-support` feature. Install at
/// most one barrier at a time and always release it; dropping the handle also
/// releases or removes its pending barrier. It does not alter production builds.
///
/// ```no_run
/// # async fn example() {
/// let pause = batter_sqlx::test_support::pause_next_verification_ledger_lock();
/// // Start a protected verification for a SQLx ledger here.
/// pause.wait_until_reached().await;
/// // Commit the concurrent fixture change, then let verification continue.
/// pause.release();
/// # }
/// ```
pub struct VerificationLedgerPause {
    state: Arc<VerificationLedgerPauseState>,
    slot: &'static PauseSlot,
}

/// Install the one-shot barrier returned as [`VerificationLedgerPause`].
///
/// # Panics
/// Panics when another unused barrier is already installed. Live tests using
/// this process-global seam must run serially.
pub fn pause_next_verification_ledger_lock() -> VerificationLedgerPause {
    install(&VERIFICATION_LEDGER_LOCK_PAUSE)
}

/// Install a one-shot barrier immediately before the protected ledger history
/// cursor is prepared and opened.
///
/// This proves that opening the history cursor remains coupled to the relation
/// identity retained by the earlier lock even after the snapshot exists.
///
/// # Panics
/// Panics when another unused history barrier is already installed. Live tests
/// using this process-global seam must run serially.
pub fn pause_next_verification_ledger_history() -> VerificationLedgerPause {
    install(&VERIFICATION_LEDGER_HISTORY_PAUSE)
}

fn install(slot: &'static PauseSlot) -> VerificationLedgerPause {
    let state = Arc::new(VerificationLedgerPauseState::default());
    let slot_state = slot.get_or_init(|| Mutex::new(None));
    let mut slot_state = slot_state.lock().unwrap_or_else(|error| error.into_inner());
    if slot_state.is_some() {
        drop(slot_state);
        panic!("verification-ledger pause already installed");
    }
    *slot_state = Some(Arc::clone(&state));
    VerificationLedgerPause { state, slot }
}

impl VerificationLedgerPause {
    /// Wait until the verifier reaches the configured protected-ledger barrier.
    pub async fn wait_until_reached(&self) {
        self.state.reached.notified().await;
    }

    /// Allow the paused verifier to continue.
    pub fn release(&self) {
        self.state.release.notify_one();
    }
}

impl Drop for VerificationLedgerPause {
    fn drop(&mut self) {
        self.state.release.notify_one();
        let slot = self.slot.get_or_init(|| Mutex::new(None));
        let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
        if slot
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(pending, &self.state))
        {
            slot.take();
        }
    }
}

pub(crate) async fn pause_after_verification_ledger_lock() {
    pause(&VERIFICATION_LEDGER_LOCK_PAUSE).await;
}

pub(crate) async fn pause_before_verification_ledger_history() {
    pause(&VERIFICATION_LEDGER_HISTORY_PAUSE).await;
}

async fn pause(slot: &'static PauseSlot) {
    let state = slot
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(state) = state {
        state.reached.notify_one();
        state.release.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_installation_does_not_poison_the_pause_slot() {
        static SLOT: PauseSlot = OnceLock::new();
        let first = install(&SLOT);
        let duplicate = std::panic::catch_unwind(|| install(&SLOT));
        assert!(duplicate.is_err());
        drop(first);
        drop(install(&SLOT));
    }
}
