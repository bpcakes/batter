use super::ScopedOutput;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Publication shares protection without claiming the underlying output.
/// Only a completed observation transfers destruction to caller ownership.
pub(crate) struct SharedOutput<R>(Arc<State<R>>);

struct State<R> {
    output: ScopedOutput<R>,
    claimed: AtomicBool,
}

impl<R> SharedOutput<R> {
    /// Require an already guarded output across the producer/publication handoff.
    pub(crate) fn new(output: ScopedOutput<R>) -> Self {
        Self(Arc::new(State {
            output,
            claimed: AtomicBool::new(false),
        }))
    }

    /// Synchronous claim: there is no cancellation point before returning R.
    /// Clone failure leaves protection intact. Phase completions clone only
    /// their internal Arcs, never application values or errors.
    pub(crate) fn claim(&self) -> R
    where
        R: Clone,
    {
        let value = self
            .0
            .output
            .value
            .as_ref()
            .expect("shared output remains present until destruction")
            .clone();
        self.0.claimed.store(true, Ordering::Release);
        value
    }
}

impl<R> Clone for SharedOutput<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<R> Drop for State<R> {
    fn drop(&mut self) {
        if self.claimed.load(Ordering::Acquire) {
            // Actual observation makes all shared result copies caller-owned.
            // ScopedOutput still releases the producer span under its dispatch.
            drop(self.output.value.take());
        }
        // Otherwise ScopedOutput destroys the unread result under its producer,
        // even when the last receiver outlives the publisher task.
    }
}
